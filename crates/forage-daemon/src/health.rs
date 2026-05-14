//! Drift detection. Given the latest `ScheduledRun` plus up to seven
//! prior ok runs, decide whether this Run's health is `Ok`, `Drift`,
//! or `Fail`.
//!
//! Rule (decided in `plans/forage-studio-redesign.md` §Decisions):
//!
//! - latest outcome failed → `Fail`
//! - latest outcome ok, but fewer than 3 prior ok runs → `Ok` (we
//!   don't have a meaningful baseline yet)
//! - otherwise: for each record type in the latest run, compute the
//!   median emit count across the prior ok runs. If any type's
//!   latest count is `<= 0.7 * median` → `Drift`.
//! - otherwise → `Ok`.
//!
//! The median is computed over the prior runs' counts for *that
//! specific type*. Missing-from-prior-but-present-in-latest doesn't
//! flag drift (it's the inverse direction); missing-from-latest does
//! — a record type that used to emit and now emits zero is a drift
//! signal even if "zero" doesn't appear in `counts` explicitly.

use crate::model::{Health, Outcome, ScheduledRun};

/// Drift threshold: a record type that falls to ≤70% of its prior
/// median is flagged. 0.7 is the locked-in value from the design plan.
const DRIFT_THRESHOLD: f64 = 0.7;

/// Minimum number of prior ok runs required to derive `Drift`. Below
/// this we always report `Ok` (or `Fail` if the latest failed).
const MIN_HISTORY: usize = 3;

/// Up to N most recent prior runs feed the median.
pub const PRIOR_WINDOW: usize = 7;

pub fn derive_health(latest: &ScheduledRun, prior_ok: &[ScheduledRun]) -> Health {
    if latest.outcome == Outcome::Fail {
        return Health::Fail;
    }
    if prior_ok.len() < MIN_HISTORY {
        return Health::Ok;
    }
    // Collect every type seen across the prior window AND the latest
    // run — types that disappeared entirely are a stronger drift signal
    // than ones that merely shrank, so we count them.
    let mut all_types: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for sr in prior_ok {
        for k in sr.counts.keys() {
            all_types.insert(k.as_str());
        }
    }
    for k in latest.counts.keys() {
        all_types.insert(k.as_str());
    }

    for ty in all_types {
        let prior_counts: Vec<u32> = prior_ok
            .iter()
            .map(|sr| sr.counts.get(ty).copied().unwrap_or(0))
            .collect();
        let median = median_u32(&prior_counts);
        if median == 0.0 {
            // No prior signal for this type — can't drift from zero.
            continue;
        }
        let latest_count = latest.counts.get(ty).copied().unwrap_or(0) as f64;
        if latest_count <= DRIFT_THRESHOLD * median {
            return Health::Drift;
        }
    }
    Health::Ok
}

fn median_u32(xs: &[u32]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<u32> = xs.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid] as f64
    } else {
        (sorted[mid - 1] as f64 + sorted[mid] as f64) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Trigger;
    use std::collections::BTreeMap;

    fn sr_ok(counts: &[(&str, u32)]) -> ScheduledRun {
        let mut map = BTreeMap::new();
        for (k, v) in counts {
            map.insert((*k).into(), *v);
        }
        ScheduledRun {
            id: ulid::Ulid::new().to_string(),
            run_id: "r".into(),
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
    fn fail_short_circuits() {
        let mut latest = sr_ok(&[("Product", 100)]);
        latest.outcome = Outcome::Fail;
        let prior = vec![sr_ok(&[("Product", 100)]); 7];
        assert_eq!(derive_health(&latest, &prior), Health::Fail);
    }

    #[test]
    fn insufficient_history_is_ok() {
        let latest = sr_ok(&[("Product", 1)]);
        let prior = vec![sr_ok(&[("Product", 100)]); 2];
        assert_eq!(derive_health(&latest, &prior), Health::Ok);
    }
}
