//! Per-run on-disk artifacts the debugger writes alongside a recipe
//! run.
//!
//! The studio mints a fresh ULID at every `run_recipe` invocation and
//! uses it as a directory prefix under `<workspace>/.forage/runs/`. The
//! only artifact today is the uncapped step response body each step
//! captures — the wire-side `StepResponse.body_raw` caps at 1 MiB to
//! keep IPC payloads bounded; this directory holds the full bytes so
//! the "load full" UI affordance can read them back on demand.
//!
//! Both path components (run_id, step name) are upstream-controlled:
//! the run_id rides through the Tauri event back to JS, and the step
//! name comes from a user-authored recipe. `full_body_path` validates
//! each so a malformed value can't escape the run directory via path
//! traversal.

use std::path::{Path, PathBuf};

/// Length of a Crockford base32 ULID. The studio mints these via
/// `ulid::Ulid::new().to_string()`; any value the frontend ships back
/// that doesn't match this length or the alphabet is rejected before
/// it can shape a path.
const ULID_LEN: usize = 26;

/// Crockford base32 alphabet (with `I`, `L`, `O`, `U` excluded — the
/// alphabet ULIDs use). Stricter than the broader RFC 4648 alphabet
/// on purpose: we only ever generate ULIDs through `ulid::Ulid`, so
/// validation can reject anything outside its alphabet.
fn is_crockford_base32(c: char) -> bool {
    matches!(
        c,
        '0'..='9' | 'A'..='H' | 'J' | 'K' | 'M' | 'N' | 'P'..='T' | 'V'..='Z'
    )
}

/// Identifier grammar from the parser's `expect_ident` shape: ASCII
/// letter/digit/underscore, must not start with a digit. The recipe
/// parser already enforces this on every step name; we re-check here
/// because the value rides through JS and could be tampered with on
/// the way back to a `load_full_step_body` call.
fn is_valid_step_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Path under the workspace where the full body for `(run_id, step)`
/// lives. Errors out when either component fails its grammar — both
/// are upstream-controlled, so a hard reject is the right answer.
/// Callers that hit the error path log + drop the artifact rather
/// than try to silently coerce a safe path; the debugger's "load
/// full" surface fails closed in that case.
pub fn full_body_path(
    workspace_root: &Path,
    run_id: &str,
    step_name: &str,
) -> Result<PathBuf, String> {
    if run_id.len() != ULID_LEN || !run_id.chars().all(is_crockford_base32) {
        return Err(format!("malformed run id: {run_id:?}"));
    }
    if !is_valid_step_name(step_name) {
        return Err(format!("malformed step name: {step_name:?}"));
    }
    let mut p = workspace_root.to_path_buf();
    p.push(".forage");
    p.push("runs");
    p.push(run_id);
    p.push("responses");
    p.push(format!("{step_name}.raw"));
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> PathBuf {
        PathBuf::from("/tmp/forage-test-ws")
    }

    #[test]
    fn well_formed_path_resolves() {
        let p = full_body_path(&ws(), "01HZ8TJC5V8R8K9PMXJ4NCQ4QT", "fetch_list").unwrap();
        assert!(p.ends_with(".forage/runs/01HZ8TJC5V8R8K9PMXJ4NCQ4QT/responses/fetch_list.raw"));
    }

    #[test]
    fn rejects_short_run_id() {
        assert!(full_body_path(&ws(), "abc", "step").is_err());
    }

    #[test]
    fn rejects_traversal_in_run_id() {
        assert!(full_body_path(&ws(), "../../etc/passwd-padded-here-26", "step").is_err());
    }

    #[test]
    fn rejects_traversal_in_step_name() {
        let valid_run = "01HZ8TJC5V8R8K9PMXJ4NCQ4QT";
        assert!(full_body_path(&ws(), valid_run, "../etc/passwd").is_err());
        assert!(full_body_path(&ws(), valid_run, "step.with.dots").is_err());
        assert!(full_body_path(&ws(), valid_run, "").is_err());
    }

    #[test]
    fn rejects_leading_digit_step() {
        let valid_run = "01HZ8TJC5V8R8K9PMXJ4NCQ4QT";
        assert!(full_body_path(&ws(), valid_run, "1step").is_err());
    }
}
