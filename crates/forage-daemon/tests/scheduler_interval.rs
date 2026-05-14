//! Scheduler interval-cadence tests.
//!
//! Two layers:
//! 1. `next_fire_for` is the pure computation. Given a stubbed clock,
//!    we verify the next-fire timestamp matches `last + every_n*unit`
//!    and that an interval whose anchor sits in the past advances to
//!    the next future tick.
//! 2. End-to-end: start the scheduler, advance the stub clock past
//!    the next-fire timestamp, ping `schedule_changed`, observe that
//!    the run fires (a scheduled_run row appears). This exercises
//!    the same code path production uses, just with a controllable
//!    clock so the test doesn't wait wall-time.

use std::time::Duration;

use forage_daemon::{
    Cadence, Daemon, RunConfig, TimeUnit, advance_next_run, interval_ms, next_fire_for,
};
mod common;
use common::{StubClock, deploy_disk_recipe, init_workspace};

#[test]
fn next_fire_returns_stored_next_run() {
    // The stored `next_run` is the canonical fire timestamp; the
    // scheduler honours it whether it's in the past or future.
    let run = build_run(
        "1m",
        Cadence::Interval {
            every_n: 1,
            unit: TimeUnit::M,
        },
        Some(60_000),
    );
    assert_eq!(next_fire_for(&run, 30_000), Some(60_000));
    // Already overdue — the scheduler's wait collapses to zero and
    // fires immediately.
    assert_eq!(next_fire_for(&run, 120_000), Some(60_000));
}

#[test]
fn next_fire_for_fresh_run_starts_one_step_ahead() {
    let run = build_run(
        "1m",
        Cadence::Interval {
            every_n: 1,
            unit: TimeUnit::M,
        },
        None,
    );
    assert_eq!(next_fire_for(&run, 0), Some(60_000));
}

#[test]
fn advance_next_run_skips_missed_ticks() {
    // The run fired at t=0 (next_run was 0); time has since advanced
    // to t=5 minutes. The next fire should be t=6m (one step past
    // current time), NOT t=1m (the very next regular step).
    let run = build_run(
        "every-2m",
        Cadence::Interval {
            every_n: 2,
            unit: TimeUnit::M,
        },
        Some(0),
    );
    let step = interval_ms(2, TimeUnit::M);
    let now = 5 * 60_000;
    let expected = (now / step + 1) * step;
    assert_eq!(advance_next_run(&run, now), Some(expected));
}

#[test]
fn next_fire_unit_h_and_d_match_seconds_math() {
    assert_eq!(interval_ms(1, TimeUnit::H), 60 * 60 * 1000);
    assert_eq!(interval_ms(1, TimeUnit::D), 24 * 60 * 60 * 1000);
}

/// End-to-end: a configured interval Run fires when the clock advances
/// past its next-fire timestamp.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interval_run_fires_when_clock_advances() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let slug = "tick";
    init_workspace(&ws_root, slug, RECIPE_STATIC);

    let mock = common::http_mock::server_returning_items(&[("a", 1.0)]).await;
    rewrite_url(
        &ws_root.join(slug).join("recipe.forage"),
        &mock.url("/items"),
    );

    let clock = StubClock::new(0);
    let daemon = Daemon::open_with_clock(ws_root.clone(), clock.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, slug);

    let output = ws_root.join(".forage").join("data").join("tick.sqlite");
    let cfg = RunConfig {
        cadence: Cadence::Interval {
            every_n: 1,
            unit: TimeUnit::M,
        },
        output,
        enabled: true,
    };
    let run = daemon.configure_run(slug, cfg).expect("configure_run");
    assert_eq!(run.next_run, Some(60_000));
    assert_eq!(run.deployed_version, Some(1));

    daemon.start_scheduler();
    // Advance the clock past the first tick. The stub clock's
    // sleep_until_ms wakes its waiters on each advance, so this is all
    // the scheduler needs to recompute the wait window.
    clock.set_now(70_000);

    // Poll until the row appears. Bounded by 2s wall-clock so a hang
    // surfaces as a timeout instead of a stalled test.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let rows = daemon
            .list_scheduled_runs(&run.id, 5, None)
            .expect("list scheduled_runs");
        if !rows.is_empty() {
            assert_eq!(rows[0].counts.get("Item").copied(), Some(1));
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("interval run did not fire within 2s");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn build_run(slug: &str, cadence: Cadence, next_run: Option<i64>) -> forage_daemon::Run {
    forage_daemon::Run {
        id: ulid::Ulid::new().to_string(),
        recipe_slug: slug.into(),
        workspace_root: std::path::PathBuf::from("/tmp"),
        enabled: true,
        cadence,
        output: std::path::PathBuf::from("/tmp/out.sqlite"),
        health: forage_daemon::Health::Unknown,
        next_run,
        deployed_version: None,
    }
}

fn rewrite_url(path: &std::path::Path, url: &str) {
    let src = std::fs::read_to_string(path).unwrap();
    std::fs::write(path, src.replace("https://example.test/items", url)).unwrap();
}

const RECIPE_STATIC: &str = r#"recipe "tick"
engine http
type Item { id: String }
step list {
    method "GET"
    url    "https://example.test/items"
}
for $i in $list.items[*] {
    emit Item { id ← $i.id }
}
"#;
