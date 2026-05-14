//! Drift-detection rule:
//!
//! - 7 prior ok runs at count=100 for Product, then a new ok run at
//!   count=60 → health = Drift (60 ≤ 0.7 * 100).
//! - Same baseline, new ok run at count=80 → health = Ok (80 > 70).
//! - Any latest run with `Outcome::Fail` → health = Fail regardless of
//!   prior history.

use std::collections::BTreeMap;

use forage_daemon::{Health, Outcome, ScheduledRun, Trigger, derive_health};

fn ok_run(counts: &[(&str, u32)]) -> ScheduledRun {
    let mut map = BTreeMap::new();
    for (k, v) in counts {
        map.insert((*k).into(), *v);
    }
    ScheduledRun {
        id: ulid::Ulid::new().to_string(),
        run_id: "run".into(),
        at: 0,
        trigger: Trigger::Schedule,
        outcome: Outcome::Ok,
        duration_s: 0.0,
        counts: map,
        diagnostics: 0,
        stall: None,
    }
}

#[test]
fn drift_flagged_when_latest_falls_below_seventy_percent_of_median() {
    let prior: Vec<ScheduledRun> = (0..7).map(|_| ok_run(&[("Product", 100)])).collect();
    let latest = ok_run(&[("Product", 60)]); // 60 / 100 = 0.6, ≤ 0.7
    assert_eq!(derive_health(&latest, &prior), Health::Drift);
}

#[test]
fn no_drift_when_latest_stays_above_threshold() {
    let prior: Vec<ScheduledRun> = (0..7).map(|_| ok_run(&[("Product", 100)])).collect();
    let latest = ok_run(&[("Product", 80)]); // 80 / 100 = 0.8, > 0.7
    assert_eq!(derive_health(&latest, &prior), Health::Ok);
}

#[test]
fn fail_outcome_short_circuits_to_fail_health() {
    let prior: Vec<ScheduledRun> = (0..7).map(|_| ok_run(&[("Product", 100)])).collect();
    let mut latest = ok_run(&[("Product", 100)]);
    latest.outcome = Outcome::Fail;
    assert_eq!(derive_health(&latest, &prior), Health::Fail);
}

#[test]
fn drift_per_type_in_a_multi_type_run() {
    // Two record types in the recipe. Product holds steady, Variant
    // collapses → drift flagged from the Variant axis even though
    // Product is fine.
    let mut prior_counts = Vec::new();
    for _ in 0..7 {
        prior_counts.push(ok_run(&[("Product", 100), ("Variant", 200)]));
    }
    let latest = ok_run(&[("Product", 100), ("Variant", 100)]);
    assert_eq!(derive_health(&latest, &prior_counts), Health::Drift);
}

#[test]
fn missing_from_latest_counts_as_zero() {
    // A type that used to emit 100 records per run and now emits zero
    // (absent from the counts map) is a stronger drift signal than a
    // partial drop.
    let prior: Vec<ScheduledRun> = (0..5).map(|_| ok_run(&[("Product", 100)])).collect();
    let latest = ok_run(&[]); // 0 / 100 = 0 → drift
    assert_eq!(derive_health(&latest, &prior), Health::Drift);
}

#[test]
fn under_three_prior_runs_reports_ok_regardless_of_count() {
    let prior: Vec<ScheduledRun> = (0..2).map(|_| ok_run(&[("Product", 100)])).collect();
    let latest = ok_run(&[("Product", 1)]); // would be drift with 3+
    assert_eq!(derive_health(&latest, &prior), Health::Ok);
}

#[test]
fn drift_threshold_boundary_locks_seventy_percent() {
    // Threshold is `latest <= 0.7 * median`. With median=100, latest=70
    // sits exactly on the boundary and flags drift; latest=71 is the
    // first value that clears it.
    let prior: Vec<ScheduledRun> = (0..7).map(|_| ok_run(&[("Product", 100)])).collect();

    let exactly_at_boundary = ok_run(&[("Product", 70)]);
    assert_eq!(
        derive_health(&exactly_at_boundary, &prior),
        Health::Drift,
        "70 / 100 == 0.7 satisfies `<= 0.7`, so drift fires"
    );

    let just_above_boundary = ok_run(&[("Product", 71)]);
    assert_eq!(
        derive_health(&just_above_boundary, &prior),
        Health::Ok,
        "71 / 100 == 0.71 clears the 0.7 threshold"
    );
}
