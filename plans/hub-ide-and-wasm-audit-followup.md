# Audit: hub-ide-and-wasm fix-up (commits `b15111c` through `6ce4d0b`)

Re-review of the eight fix-up commits against `plans/hub-ide-and-wasm-audit.md`.

## Acceptance commands

- `cargo check --workspace`: PASS.
- `cargo check --target wasm32-unknown-unknown -p forage-http --no-default-features`: PASS.
- `cargo check --target wasm32-unknown-unknown -p forage-wasm`: PASS.
- `cargo check --target wasm32-unknown-unknown -p forage-lsp --no-default-features`: PASS.
- `cargo test --workspace`: PASS.
- `npm test` in `packages/studio-ui/`: PASS (13/13).
- `npm test` in `hub-site/ide/`: PASS (11/11) — `pretest` hook regenerates `pkg/` first.
- Cold-state `rm -rf crates/forage-wasm/pkg && npm run hub-site:build`: PASS only with the pre-existing `FORAGE_HUB_PERMISSIVE_BUILD=1` (vitepress static-route fetch fails the same way the original audit flagged). The S1 part — the `pkg/` regeneration step — fires correctly via `hub-site/ide/package.json:7,9,11`'s `predev`/`prebuild`/`pretest` hooks. Confirmed by deleting `pkg/` and watching the build chain regenerate it.
- `rg 'invoke\(' packages/studio-ui/src`: 0 hits. The 42 in `TauriStudioService.ts` are `invoke<Type>(args)` form, which the literal regex doesn't match (same as the original audit).

## Per-finding confirmation

**S1 (build chain).** `package.json:12` adds `forage-wasm:build` (`wasm-pack build --target web --release crates/forage-wasm`); `hub-site/ide/package.json:7,9,11` chain it as `predev`/`prebuild`/`pretest`. `.gitignore:16` adds `crates/forage-wasm/pkg/`. Cold-state regeneration verified.

**S2 (`authWhoami`).** `hub-site/ide/src/HubStudioService.ts:360-374` fetches `${this.hubUrl}/v1/oauth/whoami` with `credentials: "include"`, parses the typed envelope, returns the login or null. Three new vitest cases at `HubStudioService.test.ts:165-190` assert URL shape + credentials, signed-in branch, signed-out branch, and 500-response branch.

**S3 (dead `EventBus`).** `rg 'EventBus' hub-site/ide/src` returns zero. `onRunEvent` (`HubStudioService.ts:468-474`) returns the no-op unsubscribe like its siblings.

**S4 / S5 / S6 (stubbed methods, `as unknown as` casts).** `rg 'as unknown as' hub-site/ide/src/HubStudioService.ts` returns zero (one hit remains in `HubStudioService.test.ts:29` for the jsdom `fetch` global, which is the standard test-only pattern). Each stubbed method now delegates to a real wasm export:

- `validateRecipe` -> `parse_and_validate` (`HubStudioService.ts:183`)
- `recipeOutline` -> `recipe_outline` (`:223`)
- `recipeHover` -> `recipe_hover` (`:226`)
- `recipeProgressUnit` -> `recipe_progress_unit` (`:235`)
- `languageDictionary` -> `language_dictionary` (`:239`)
- `runRecipe(replay)` -> `run_replay` (`:271`)

Object literals match the ts-rs bindings exactly: `runRecipe` (`:282-298`) returns `{ ok, error, snapshot, daemon_warning }` matching `RunOutcome`'s four-field shape (`bindings/RunOutcome.ts:3-10`); `daemonStatus` (`:313-318`) returns `{ running, version, started_at, active_count }` matching `DaemonStatus` (`bindings/DaemonStatus.ts:8-28`). The remaining single `as Type` casts (e.g., `as RecipeOutline` at `:223`) are coercions from `JsValue`/`any` to the bound shape — necessary because `wasm-bindgen` exports return `any`. The shapes are produced by the Rust side directly (`crates/forage-wasm/src/lib.rs:208-274`), so the cast doesn't paper over drift.

**M1 (malformed 409 envelopes).** `hub-site/ide/src/HubStudioService.ts:532-543` and `packages/studio-ui/src/lib/services/TauriStudioService.ts:439-456` both gate `StaleBaseError` construction on `typeof err.message === "string"`; the malformed-body branch falls through to the generic `${status} ${statusText}: ${JSON.stringify(body)}` envelope error. (Spec mentioned `crates/forage-hub/src/client.rs` for M1 — that crate already had the proper `ServerMalformed { detail }` handling at `client.rs:158-238` with three unit tests covering malformed envelopes; pre-existing and untouched by this fix-up. The actual M1 finding in the original audit was the two TS sites, which are now correct.)

**M4 (version URL encoding).** `HubStudioService.ts:419` now wraps `String(version)` in `encodeURIComponent`. Callers pass `number | "latest"` so no chars need escaping today, but the contract is documented.

**M5 (shared jsdom test stubs).** `packages/studio-ui/src/test-setup-jsdom.ts` is new (38 lines). Both `packages/studio-ui/src/test-setup.ts:11` and `hub-site/ide/src/test-setup.ts:6` import it. The Tauri-internals stub stays Studio-only.

**Q2 / M4 (`confirm` labels).** `HubStudioService.ts:497-512` accepts `options` (no longer `_options`) and emits `console.warn` when `title`/`okLabel`/`cancelLabel` are passed but `window.confirm` will drop them. Comment at the skip point explains why.

## Cross-cutting checks

**Method classification.** 19 `Promise.reject(new NotSupportedByService(...))` sites (verified by grep), matching the PE's claim. Methods classified as read-only with a natural empty value (`currentWorkspace`, `listRecentWorkspaces`, `listRuns`, `getRun`, `daemonStatus`, `loadRunRecords`, `listScheduledRuns`) return null / empty array / fully-populated "not running" `DaemonStatus`, which is wire-legitimate since the `StudioService` interface admits those as valid responses. Action methods (`openWorkspace`, `createRecipe`, `pickDirectory`, `configureRun`, `publishRecipe`, etc.) throw `NotSupportedByService`. The split is consistent.

Real-backend spot-checks:
- `validateRecipe` -> `parse_and_validate` wasm export ✓
- `recipeOutline` -> `recipe_outline` wasm export ✓
- `authWhoami` -> `GET /v1/oauth/whoami` ✓
- `listPackages` -> `GET /v1/packages?...` ✓
- `starPackage` -> `POST /v1/packages/.../stars` with `credentials: "include"` ✓

**forage-lsp WASM gating.** `crates/forage-lsp/Cargo.toml:12` `default = ["native"]`; `:16` gates `tokio`/`tower-lsp`/`walkdir`/`thiserror`/`tracing`/`serde_json` under the feature. `src/lib.rs:15` keeps `intel` unconditional. `apps/studio/src-tauri/Cargo.toml` and `apps/cli/Cargo.toml` opt back into `features = ["native"]`. `cargo check --target wasm32-unknown-unknown -p forage-lsp --no-default-features` PASSES.

**forage-wasm outline test.** `crates/forage-wasm/tests/outline.rs:1-54` exists with three cases: top-level steps in source order, descent into for-loop bodies, empty result on parse failure. All three pass under the workspace test run.

**Pre-existing clippy warnings (out of scope per spec).** `forage-core` lib: 9 warnings (doc list indentation, redundant guard, `Iterator::last` on `DoubleEndedIterator`). `forage-studio` lib: 3 warnings (deref which would be done by auto-deref). All pre-existing — not touched by this fix-up. Candidates for a follow-up cleanup pass, not gating.

**M2 (`hash route` in live source).** `rg 'hash route' .` excluding `.git/` returns zero hits. The phrase only survives in `git log` (commit `950bab4`'s body).

**M3 (`plans/` self-contradiction).** 6 audit files are tracked in `plans/` (force-added past the gitignore). None contradict the current state.

## Findings

### Critical

None.

### Significant

None.

### Minor

**N1. Cold-state `npm run hub-site:build` still needs `FORAGE_HUB_PERMISSIVE_BUILD=1`.** Pre-existing and explicitly noted in the original audit ("the `FORAGE_HUB_PERMISSIVE_BUILD` need is pre-existing; the wasm-pack need is new in this PR"). The S1 fix correctly addresses the wasm-pack piece. The vitepress static-route fetch behavior is out of scope for this PR.

**N2. `forage_version` wasm export is unused** (`crates/forage-wasm/src/lib.rs:19-21`). Not flagged in the previous audit, not introduced by this fix-up. Either wire it into the hub IDE's footer or drop the export per YAGNI; out of scope.

**N3. Pre-existing clippy warnings** in `forage-core` (9) and `forage-studio` (3). Out of scope per spec; flagged for a possible follow-up.

### Questions

None outstanding.

## Verdict

**Approve — audit closed.** All eight original findings (S1, S2, S3, S4, S5, S6, M1, M4, M5) are confirmed addressed in the fix-up commits. Two questions (Q1 inputs/secrets, Q2 version encoding) have explanatory comments at the relevant sites. The acceptance commands pass; the cold-state build chain regenerates `forage-wasm/pkg/` correctly; the new wasm exports (`recipe_outline`, `recipe_hover`, `recipe_progress_unit`, `language_dictionary`) match their ts-rs bindings exactly. `forage-lsp` feature-gating cleanly separates the native LSP surface from the wasm-callable `intel` module, with a passing wasm build and a new native-target outline test.
