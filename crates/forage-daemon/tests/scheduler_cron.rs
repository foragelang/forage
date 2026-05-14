//! Scheduler cron-cadence tests.
//!
//! Cron expressions parse via `cron::Schedule`. The daemon exposes
//! `validate_cron` for configure-time rejection and `next_fire_for`
//! for the schedule computation; both are tested here.

use std::time::Duration;

use chrono::{Datelike, TimeZone, Timelike, Utc};
use forage_daemon::{Cadence, Daemon, Health, Outcome, RunConfig, next_fire_for, validate_cron};

mod common;
use common::{StubClock, init_workspace};

#[test]
fn cron_top_of_each_hour_parses_and_computes_next_fire() {
    // `sec min hour day-of-month month day-of-week` — every hour at xx:00:00.
    let expr = "0 0 * * * *";
    validate_cron(expr).expect("valid cron");

    let run = forage_daemon::Run {
        id: "1".into(),
        recipe_slug: "x".into(),
        workspace_root: std::path::PathBuf::from("/tmp"),
        enabled: true,
        cadence: Cadence::Cron {
            expr: expr.to_string(),
        },
        output: std::path::PathBuf::from("/tmp/out.sqlite"),
        health: Health::Unknown,
        next_run: None,
    };

    // Anchor `now` deterministically at 2030-01-01 03:17:23 UTC; the
    // next top-of-hour fire is 04:00:00.
    let now = Utc
        .with_ymd_and_hms(2030, 1, 1, 3, 17, 23)
        .single()
        .unwrap();
    let next = next_fire_for(&run, now.timestamp_millis()).expect("cron returns next");
    let dt = chrono::DateTime::<Utc>::from_timestamp_millis(next).unwrap();
    assert_eq!(dt.year(), 2030);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 1);
    assert_eq!(dt.hour(), 4);
    assert_eq!(dt.minute(), 0);
    assert_eq!(dt.second(), 0);
}

#[test]
fn bad_cron_expression_rejected() {
    assert!(validate_cron("not-a-cron").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configure_run_rejects_bad_cron() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let slug = "hourly";
    init_workspace(&ws_root, slug, RECIPE_STATIC);

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    let cfg = RunConfig {
        cadence: Cadence::Cron {
            expr: "totally invalid".into(),
        },
        output: ws_root.join(".forage").join("data").join(format!("{slug}.sqlite")),
        enabled: true,
    };
    let err = daemon.configure_run(slug, cfg).expect_err("expected bad-cron");
    assert!(matches!(
        err,
        forage_daemon::DaemonError::BadCron { .. }
    ));
}

/// A stored cron expression that was valid at configure-time can
/// become unparseable later (DB corruption, downgrade, manual edit).
/// The scheduler must surface that as a recorded failed `ScheduledRun`
/// + `Health::Fail` rather than silently skipping the run forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupted_stored_cron_records_synthetic_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let slug = "hourly";
    init_workspace(&ws_root, slug, RECIPE_STATIC);

    let clock = StubClock::new(0);
    let daemon = Daemon::open_with_clock(ws_root.clone(), clock.clone()).expect("open daemon");

    let cfg = RunConfig {
        cadence: Cadence::Cron {
            // Valid at configure-time; we'll tamper with the row below.
            expr: "0 0 * * * *".into(),
        },
        output: ws_root
            .join(".forage")
            .join("data")
            .join(format!("{slug}.sqlite")),
        enabled: true,
    };
    let run = daemon.configure_run(slug, cfg).expect("configure_run");

    // Tamper: replace the stored cron expression with garbage,
    // mimicking a corrupt DB. The daemon owns the connection, so we
    // open a sibling sqlite handle on the same file.
    let db_path = ws_root.join(".forage").join("daemon.sqlite");
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    raw.execute(
        "UPDATE runs SET cadence_json = ?1 WHERE id = ?2",
        rusqlite::params![
            serde_json::json!({"kind": "cron", "expr": "definitely-not-valid"}).to_string(),
            run.id,
        ],
    )
    .unwrap();
    drop(raw);

    daemon.start_scheduler();

    // The scheduler tick re-reads the corrupt row and persists a
    // synthetic failure. Bound by 2s wall-clock so a hang surfaces as
    // a timeout instead of a stalled test.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let synthetic = loop {
        let rows = daemon
            .list_scheduled_runs(&run.id, 5, None)
            .expect("list scheduled_runs");
        if let Some(row) = rows.into_iter().next() {
            break row;
        }
        if std::time::Instant::now() > deadline {
            panic!("synthetic failure was not recorded within 2s");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(synthetic.outcome, Outcome::Fail);
    let stall = synthetic.stall.as_deref().expect("synthetic failure carries stall message");
    assert!(
        stall.contains("invalid cron expression"),
        "stall message must explain the failure, got: {stall:?}",
    );
    assert!(
        stall.contains("definitely-not-valid"),
        "stall message must include the offending expression, got: {stall:?}",
    );

    let refreshed = daemon.get_run(&run.id).expect("get_run").expect("run row");
    assert_eq!(refreshed.health, Health::Fail);
}

const RECIPE_STATIC: &str = r#"recipe "hourly"
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
