# forage

Rust rewrite of the Forage scraping DSL + runtime. Workspace at the
repo root; crates under `crates/`, apps (CLI + Studio) under `apps/`.

## Every bug fix lands with a regression test

When you fix a bug, add a test that reproduces the original failure
*before* you fix it, then confirms it passes after. No exceptions —
not even for "obvious" fixes. The test is the receipt that you
understood the failure, not just patched a symptom.

The test goes next to the code being fixed, in the existing `#[cfg(test)]
mod tests { ... }` block where applicable.

### Especially for `crates/forage-http/src/engine.rs`

The engine is the load-bearing center of the runtime — every recipe
flows through it. Bugs here surface as cryptic eval errors at runtime,
days after the change. A regression test isolates the failure mode and
catches it the next time someone touches scope binding, pagination,
auth threading, or response parsing.

Pattern to follow (see `paginated_step_binds_accumulated_items`):

1. Write a minimal recipe in a string literal that reproduces the bug.
2. Build a `ReplayTransport` with the exact response shape that
   triggered the failure (multi-page if pagination is involved).
3. Run the engine, assert on `snap.records` — record count, type,
   field values, and order.

If the bug only reproduces against a real server, capture the
exchange into a `Capture::Http { ... }` fixture and replay it.

## Greenfield — no migrations, no compat shims

This is a pre-1.0 rewrite. Schema changes, AST changes, IR changes,
binary format changes: edit the canonical definition and update every
caller in one PR. Don't write `migrateV1ToV2`, don't keep both shapes
around behind a flag, don't add `#[serde(default)]` to silently absorb
old field names. If a fixture is stale, regenerate it.

The same applies to capture/replay fixtures, hub-api KV entries, and
LSP wire messages — break them and move on.
