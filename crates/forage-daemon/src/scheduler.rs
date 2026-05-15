//! Single-tokio-task scheduler. Walks `Run.cadence` for every enabled
//! non-manual run, sleeps until the soonest next-fire, fires that run,
//! repeats. Configuration changes (`configure_run`, `remove_run`,
//! `ensure_run`) wake the loop via a `Notify` so a freshly-added run
//! starts ticking immediately.
//!
//! Cron expressions use the `cron` crate's `Schedule::from_str`. Bad
//! expressions surface at configure-time (`Daemon::configure_run`
//! validates) so the scheduler doesn't have to fail dynamically. If
//! the stored expression somehow corrupts (tampered DB, downgrade),
//! the next tick records a synthetic failed `ScheduledRun` and flips
//! the Run's health to `Fail` so the UI shows a stall reason instead
//! of a silently-not-firing schedule.
//!
//! Both "now" and the wait between fires flow through `Daemon::clock`.
//! Production uses `SystemClock` (wall-clock + tokio sleep); tests
//! inject a `StubClock` so `advance()` drives ticks without any
//! wall-clock waits.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use chrono::Utc;
use tokio::task::JoinHandle;

use crate::error::DaemonError;
use crate::model::{Cadence, Outcome, Run, RunFlags, TimeUnit};
use crate::{Daemon, ScheduledRun, Trigger};

/// Start the scheduler task. Hold the returned `JoinHandle` to wait on
/// the task; drop it to fire-and-forget. Calling `Daemon::close` aborts
/// the task by signalling the shutdown notify.
pub(crate) fn start(daemon: Arc<Daemon>) -> JoinHandle<()> {
    tokio::spawn(async move { run_loop(daemon).await })
}

async fn run_loop(daemon: Arc<Daemon>) {
    // Mark this task live; clear on exit so `Daemon::status().running`
    // tracks the real state even on panics (held across the loop).
    struct LiveGuard<'d>(&'d Daemon);
    impl Drop for LiveGuard<'_> {
        fn drop(&mut self) {
            self.0.scheduler_running.store(false, Ordering::SeqCst);
        }
    }
    let _live = LiveGuard(&daemon);
    daemon.scheduler_running.store(true, Ordering::SeqCst);
    // Safety-net poll horizon when no runs are scheduled. The loop
    // wakes on `schedule_changed` whenever the schedule actually
    // changes; this is the worst-case latency for corrupted state to
    // be re-evaluated.
    const IDLE_POLL_MS: i64 = 60_000;
    loop {
        // Compute the earliest fire across every enabled non-manual run.
        let now_ms = daemon.now_ms();
        let plan = match next_fire(&daemon, now_ms) {
            NextFire::Run { at, run_id } => Some((at, run_id)),
            NextFire::SyntheticFail {
                run_id,
                stall_message,
            } => {
                if let Err(e) = record_synthetic_failure(&daemon, &run_id, &stall_message).await {
                    tracing::error!(
                        run_id = %run_id,
                        error = %e,
                        "failed to record synthetic cron failure",
                    );
                }
                continue;
            }
            NextFire::None => None,
        };

        let deadline_ms = plan
            .as_ref()
            .map(|(at, _)| *at)
            .unwrap_or(now_ms + IDLE_POLL_MS);

        tokio::select! {
            biased;
            _ = daemon.shutdown.notified() => {
                tracing::debug!("scheduler shutdown signal received");
                return;
            }
            _ = daemon.schedule_changed.notified() => {
                // Re-enter the loop with the fresh state.
                continue;
            }
            _ = daemon.clock.sleep_until_ms(deadline_ms) => {
                if let Some((_, run_id)) = plan {
                    // Scheduled fires are production runs: live transport,
                    // full record set, persistent output. The dev preset
                    // is a Studio "Run" button concern — never the
                    // scheduler's. Hard-coding `RunFlags::prod()` keeps
                    // the cadence path honest about what it produces.
                    if let Err(e) = daemon
                        .run_once(&run_id, Trigger::Schedule, RunFlags::prod())
                        .await
                    {
                        tracing::warn!(
                            run_id = %run_id,
                            error = %e,
                            "scheduled run produced an error",
                        );
                        // run_once persists the scheduled-run row even on
                        // failure (the run's `health` ends up `Fail`); the
                        // only errors that bubble out are `RunError::Daemon`
                        // for genuinely-corrupt state.
                    }
                }
            }
        }
    }
}

/// Outcome of `next_fire`. The synthetic-failure case lets the scheduler
/// surface a corrupt stored cron expression to the UI as a recorded
/// failed `ScheduledRun` + `Health::Fail`, rather than silently skipping
/// the run forever.
enum NextFire {
    Run {
        at: i64,
        run_id: String,
    },
    SyntheticFail {
        run_id: String,
        stall_message: String,
    },
    None,
}

/// Persist a synthetic failed `ScheduledRun` for a Run whose cron
/// expression no longer parses, and flip the Run's health to `Fail`.
/// Mirrors the persist-under-transaction path in `run_once` so the
/// recorded row and the updated health move together.
async fn record_synthetic_failure(
    daemon: &Arc<Daemon>,
    run_id: &str,
    stall_message: &str,
) -> Result<(), DaemonError> {
    let at_ms = daemon.now_ms();
    let run = {
        let conn = daemon
            .connection
            .lock()
            .expect("daemon connection poisoned");
        crate::db::get_run_by_id(&conn, run_id)?
    };
    let Some(run) = run else {
        // Run vanished between scheduling and recording — nothing to
        // record. The loop will recompute next-fire.
        return Ok(());
    };
    let scheduled = ScheduledRun {
        id: ulid::Ulid::new().to_string(),
        run_id: run.id.clone(),
        at: at_ms,
        trigger: Trigger::Schedule,
        outcome: Outcome::Fail,
        duration_s: 0.0,
        counts: std::collections::BTreeMap::new(),
        diagnostics: 0,
        stall: Some(stall_message.to_string()),
        // Synthetic cron-fail fires before the engine resolves any
        // deployed version — the cron expression itself failed to
        // parse, so we never got far enough to pick a version.
        recipe_version: None,
    };
    {
        let mut conn = daemon
            .connection
            .lock()
            .expect("daemon connection poisoned");
        let tx = conn.transaction()?;
        crate::db::insert_scheduled_run(&tx, &scheduled)?;
        let updated = crate::model::Run {
            health: crate::model::Health::Fail,
            next_run: None,
            ..run
        };
        crate::db::update_run(&tx, &updated)?;
        tx.commit()?;
    }
    if let Some(cb) = daemon
        .run_completed_cb
        .lock()
        .expect("cb poisoned")
        .as_ref()
    {
        cb(&scheduled);
    }
    Ok(())
}

/// Earliest scheduled fire across all enabled non-manual runs, plus a
/// best-effort detection of stored-cron corruption. If the cheapest
/// run to fire is one with a now-unparseable expression, the loop
/// records a synthetic failure for it and skips re-firing.
fn next_fire(daemon: &Daemon, now_ms: i64) -> NextFire {
    let runs = match daemon.list_runs() {
        Ok(rs) => rs,
        Err(e) => {
            // A DB read failure here strands the scheduler. Log + back
            // off — the loop will poll again on the idle horizon (the
            // `None` plan path falls through to a long sleep with
            // notify-cancel available).
            tracing::error!(error = %e, "scheduler list_runs failed");
            return NextFire::None;
        }
    };
    let mut best: Option<(i64, String)> = None;
    for run in &runs {
        if !run.enabled {
            continue;
        }
        // Detect a corrupt stored cron up front. We can't surface this
        // as the loop's "next fire" timestamp because the run is
        // un-fire-able, so we record a synthetic failure right away
        // and the loop continues. Once recorded, the Run's health is
        // `Fail` — we skip it on subsequent ticks so we don't record
        // the same failure forever. A new `configure_run` validates
        // and resets health to `Unknown`, letting the run re-enter
        // the schedule.
        if let Cadence::Cron { expr } = &run.cadence {
            if let Err(parse_err) = cron::Schedule::from_str(expr) {
                if run.health == crate::model::Health::Fail {
                    continue;
                }
                return NextFire::SyntheticFail {
                    run_id: run.id.clone(),
                    stall_message: format!("invalid cron expression: {expr} — {parse_err}"),
                };
            }
        }
        let Some(at) = next_fire_for(run, now_ms) else {
            continue;
        };
        match &best {
            None => best = Some((at, run.id.clone())),
            Some((prev_at, _)) if at < *prev_at => best = Some((at, run.id.clone())),
            _ => {}
        }
    }
    match best {
        Some((at, run_id)) => NextFire::Run { at, run_id },
        None => NextFire::None,
    }
}

/// Compute the next ms-epoch fire timestamp for a single run, given a
/// "now" reference. Returns `None` for manual cadences or unparseable
/// cron expressions (we already validate at configure-time; this is
/// belt-and-suspenders).
pub fn next_fire_for(run: &Run, now_ms: i64) -> Option<i64> {
    match &run.cadence {
        Cadence::Manual => None,
        Cadence::Interval { every_n, unit } => {
            let step_ms = interval_ms(*every_n, *unit).max(1);
            // The stored `next_run` is the canonical next fire. If
            // it's still in the future, return it as-is; if it's
            // already past `now_ms`, the tick is overdue and the
            // scheduler fires it immediately (wait collapses to zero).
            //
            // Fresh runs without a stored `next_run` start one
            // interval-step from now.
            Some(run.next_run.unwrap_or(now_ms + step_ms))
        }
        Cadence::Cron { expr } => cron_next_fire(expr, now_ms),
    }
}

/// After a fire, compute the run's *next* `next_run`. Interval: the
/// first multiple of `step` strictly greater than `now_ms`. Cron:
/// `cron::Schedule.after(now)`. Manual: `None`.
pub fn advance_next_run(run: &Run, now_ms: i64) -> Option<i64> {
    match &run.cadence {
        Cadence::Manual => None,
        Cadence::Interval { every_n, unit } => {
            let step_ms = interval_ms(*every_n, *unit).max(1);
            // Walk forward from the prior next_run (the tick we just
            // fired) until we land strictly in the future. Honours the
            // "missed multiple ticks while offline" case — interval
            // runs catch up to *one* tick after the current time
            // rather than firing the entire backlog.
            let anchor = run.next_run.unwrap_or(now_ms);
            let mut next = anchor + step_ms;
            if next <= now_ms {
                let behind = now_ms - next;
                let skips = behind / step_ms + 1;
                next += skips * step_ms;
            }
            Some(next)
        }
        Cadence::Cron { expr } => cron_next_fire(expr, now_ms),
    }
}

/// Parse a cron expression and compute the next fire time relative to
/// `now_ms`. Returns `None` if the expression doesn't parse or
/// produces no future fire.
fn cron_next_fire(expr: &str, now_ms: i64) -> Option<i64> {
    // We validate cron expressions at configure-time, so a parse
    // failure here means a tampered-with row (or a config-time
    // validator gap). Log and treat the run as un-fire-able; the
    // daemon keeps making progress on other runs.
    let schedule = match cron::Schedule::from_str(expr) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(expr = expr, error = %e, "stored cron expression no longer parses");
            return None;
        }
    };
    let now = chrono::DateTime::<Utc>::from_timestamp_millis(now_ms)?;
    let next = schedule.after(&now).next()?;
    Some(next.timestamp_millis())
}

/// Validate a cron expression without computing a fire. Used by
/// `Daemon::configure_run` so a bad expression rejects the update
/// instead of corrupting the schedule.
pub fn validate_cron(expr: &str) -> Result<(), DaemonError> {
    cron::Schedule::from_str(expr).map_err(|e| DaemonError::BadCron {
        expr: expr.to_string(),
        detail: e.to_string(),
    })?;
    Ok(())
}

pub fn interval_ms(every_n: u32, unit: TimeUnit) -> i64 {
    let multiplier_secs: i64 = match unit {
        TimeUnit::M => 60,
        TimeUnit::H => 60 * 60,
        TimeUnit::D => 60 * 60 * 24,
    };
    (every_n as i64) * multiplier_secs * 1000
}
