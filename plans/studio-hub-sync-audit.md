# studio-hub-sync audit

Reviewed five commits merged directly to `main`: `a4fc4df`, `1a1d274`,
`f4ff377`, `9d36493`, `c9bc5d6`. `cargo check --workspace` is clean;
`cargo test --workspace` reports 313 passed (PE's claim holds). The
Studio Tauri tree (`apps/studio/src-tauri`) compiles with the deep-link
plugin wired through `tauri-plugin-deep-link = "2.4"` (latest 2.4.9 on
crates.io — fine).

The shape is mostly in line with the plan: shared operations in
`crates/forage-hub/src/operations.rs` consumed by Studio + CLI, atomic
`PublishRequest` matching `hub-api/src/types.ts`, typed
`PublishError::StaleBase` discriminant, `forage://` deeplink via the
plugin's `desktop.schemes`, `.forage-meta.json` sidecar threaded
through sync→publish, and the manifest enriched with the metadata the
wire requires. Three defects below break the actual roundtrip; one
violates an explicit greenfield rule the plan called out.

## 🔴 Critical

### C1. `forked_from` smuggled into v2+ publish payloads — hub will 400

`crates/forage-hub/src/operations.rs:107-120` + `:223-238` + `:261`.
`sync_from_hub` reads the upstream package metadata and writes the
sidecar with `forked_from` populated. `assemble_publish_request` then
pulls that field back out of the sidecar and stuffs it into
`PublishRequest.forked_from`. `publish_from_workspace` rewrites the
sidecar *with the same `forked_from`*, so every subsequent publish on
that fork carries it too.

`hub-api/src/routes/packages.ts:286-294` rejects any v2+ publish that
includes a non-null `forked_from` with 400 `forked_from_on_existing`.
The hub-social-api audit's C2 was the explicit reason that check
exists; the spec for this audit calls it out:

> "The CLI's `forage publish` and the Tauri `publish_recipe` MUST NOT
>  include `forked_from` in their PublishRequest payloads."

Effect: every `forage publish` and Studio Publish on a forked recipe
fails with 400 (after sync wrote the sidecar). The integration tests
do not exercise this path — `fork_then_sync_round_trip` stops after
the sidecar is populated; the only publish coverage uses sidecars
where `forked_from = None`.

Fix: drop `forked_from` from `PublishRequest` entirely (hub stamps it
on v1 from the fork endpoint, never on the publish payload). The
sidecar can still track lineage for display; just don't echo it onto
the wire.

### C2. `fetch_to_cache` writes decls outside the dep-cache scan root

`crates/forage-hub/src/operations.rs:147-161`. `fetch_to_cache` calls
`materialize_version(&dir, ...)` with `dir =
<cache>/<author>/<slug>/<version>/`. `materialize_version`
(`:312-355`) places decls at `recipe_dir.parent()` — i.e.
`<cache>/<author>/<slug>/shared.forage`, *one level above* the
version directory.

`crates/forage-core/src/workspace/mod.rs:396-401` walks the dep cache
via `resolve_dep(slug, version)` which returns
`<cache>/<author>/<slug>/<version>/`, then
`scan_package_declarations` (`:450-471`) recurses *inside* that
directory only. The decls written by `fetch_to_cache` are outside
the walked subtree, so `forage update` populates the cache in a shape
the workspace loader cannot see.

The test at `crates/forage-hub/tests/hub_sync.rs:481-503` asserts the
decls land at `<cache>/<author>/<slug>/shared.forage` and comments
"`scan_package_declarations` walks" that layout — but
`scan_package_declarations` only walks the version-pinned subtree.
The assertion confirms the wrong layout.

Fix: write decls under the version directory
(`<cache>/<author>/<slug>/<version>/shared.forage`) so the existing
dep loader picks them up. `sync_from_hub` should keep writing them at
the workspace root because that's where the workspace loader
discovers shared declarations during recipe-time catalog build.
Two different roots for two different consumers; one
`materialize_version` shouldn't try to serve both.

### C3. `#[serde(default)]` on the new manifest fields — explicit plan violation

`crates/forage-core/src/workspace/manifest.rs:32-40`. `description`,
`category`, `tags` all carry `#[serde(default)]`. The plan
(`plans/studio-hub-sync.md:201-204`) and the hub roadmap
(`plans/hub-roadmap.md:144-145`) both call out exactly this:

> "Greenfield: no `#[serde(default)]` on the new fields."
> "Greenfield: no `#[serde(default)]` to soften shape changes; every
>  caller updates in one PR."

CLAUDE.md greenfield section reinforces. The CLI publish path
compensates with explicit emptiness checks
(`apps/cli/src/main.rs:618-622`), but the manifest parser still
silently accepts a `forage.toml` missing all three fields and yields
empty strings — which is exactly the drift the rule exists to
prevent.

Fix: drop the `#[serde(default)]` attributes. Stale `forage.toml`
files in the repo (there are none — see S2 below) fail to parse
loudly. New manifests must include the fields.

## 🟡 Significant

### S1. Server-issued 401 maps to `PublishError::Other`, not `not_signed_in`

`apps/studio/src-tauri/src/hub_sync.rs:43-60`. `from_hub_error` only
maps `HubError::StaleBase` to a typed variant; an `HubError::Api {
status: 401, ... }` from the server (expired/revoked token) falls
through to `PublishError::Other`. The audit spec asks for:

> "401 unauthenticated surfaces typed error (not_signed_in)."

Today `NotSignedIn` only fires when the local token is missing
(`hub_sync.rs:155-159`, `:175-180`). A user with a stale token gets
a generic "Other" toast and no rebanner-to-sign-in affordance.

Fix: extend `from_hub_error` to map `HubError::Api { status: 401, ..
}` to `PublishError::NotSignedIn`.

### S2. Error-envelope masking with magic defaults

`crates/forage-hub/src/client.rs:171,176,183`. Three
`unwrap_or(...)` masks on the HTTP error decode path:

- `.unwrap_or("ERROR")` — when the envelope omits `error.code`, the
  client invents `"ERROR"` and the caller can't tell the server is
  returning malformed errors.
- `.unwrap_or(text.as_str())` — same, but for the message.
- `.unwrap_or(0)` — `latest_version` on a `stale_base` response
  defaults to 0. The Studio toast would read "hub is at v0" — useless
  and misleading.

The audit spec calls this out: "No `unwrap_or_else(|| default)`
masking on the auth or HTTP layers." CLAUDE.md's "Never mask errors
with defaults" section applies.

Fix: when the envelope is malformed, return
`HubError::Transport(format!("malformed error envelope: {text}"))`
instead of guessing. For `stale_base` specifically, a missing
`latest_version` means the server is broken — surface that, don't
report version 0.

### S3. CLI sync `bare-slug` (no `@`) parity untested

`apps/cli/src/main.rs:744-752` accepts `alice/zen-leaf` and
`@alice/zen-leaf`. The audit spec asks for both shapes to work and
to be tested. `apps/cli/tests/hub_sync.rs` covers only `@alice/...`
positive paths and `bare-slug-no-author` negative — no positive test
that `alice/zen-leaf` (without the `@`) round-trips end-to-end. The
parser handles it, but a regression would be silent.

Fix: add one assertion in either the CLI subprocess test or a unit
test on `parse_spec` covering both shapes.

### S4. Snapshot serialization masks with `Value::Null`

`crates/forage-hub/src/operations.rs:491`. `core_snapshot_to_wire`
serializes each record with `serde_json::to_value(r).unwrap_or(Value::Null)`.
If serialization fails (it shouldn't for a `Record` of strings +
`IndexMap`, but the failure is still latent), the record silently
becomes `null` in the snapshot. The hub would then store an
artifact with `[null]` in the record arrays, and replay against the
snapshot later would silently lose the record.

Fix: propagate the serialization error. `core_snapshot_to_wire`
should return `Result<PackageSnapshot, _>` (or the call site should
fail loudly).

### S5. `let _ = write!(...)` on hex encode

`crates/forage-hub/src/operations.rs:179`. Writing to `String`
through `fmt::Write` is infallible; `let _ =` swallows a Result that
genuinely cannot fail. CLAUDE.md flags `let _ = ...` as masking a
signal even when the underlying call is infallible. Use `.unwrap()`
(or, better, `write!` directly without the `let`).

### S6. Existing canonical recipes lack manifests; publish-from-recipe-library doesn't work

`recipes/leafbridge`, `recipes/jane`, `recipes/sweed` (and every
other entry under `recipes/`) ship without a `forage.toml`. The
plan called this out as one of two acceptable outcomes — either
populate the manifests or surface a clear "missing X" error. The
CLI does the latter cleanly (`apps/cli/src/main.rs:606-622`), so
this is acceptable but worth flagging: the recipe library is not
currently publishable as-is. Studio publish is also not testable
end-to-end without populating one of these.

## 🔵 Minor

### M1. `sync_from_hub` bumps the upstream download counter even when called via `fork_from_hub`

`crates/forage-hub/src/operations.rs:107-133` always calls
`record_download`. `fork_from_hub` (`:188-198`) calls `sync_from_hub`
on the *new fork*, so the download bump lands on the new fork — but
the hub's fork endpoint already bumps the upstream's downloads
(`hub-api/src/routes/forks.ts:148`). Result: a fresh fork gets one
download counted against itself for the very first sync. Probably
intentional ("the user 'downloaded' their own fork") but worth
naming explicitly in the operations docstring.

### M2. `record_download` failure logged at WARN, but the operation comment says "informational"

`crates/forage-hub/src/operations.rs:124-133` logs at `warn!` for a
counter bump failure. The comment says "informational; if it fails
we still consider the sync successful." A real-world counter outage
would spam warnings on every sync — `tracing::debug!` is the right
level for a best-effort counter, with `warn!` reserved for things
the user should actually act on.

### M3. `validate_segments` builds the regex on every call

`apps/studio/src-tauri/src/hub_sync.rs:234-246`. Compiles
`SEGMENT_PATTERN` fresh each call. `once_cell::sync::Lazy` or
`std::sync::LazyLock` would compile once. Tiny but the deeplink
handler can fire repeatedly.

### M4. PE-flagged divergences are sound

- **Shared operations live in `forage-hub`, not Studio**: correct.
  `crates/forage-hub` is already shared by Studio + CLI + daemon;
  the plan's tentative "or `crates/forage-daemon`" alternative was
  worse because the daemon isn't in the CLI's dep tree.
- **Manifest got `description`/`category`/`tags`**: correct location
  (`crates/forage-core/src/workspace/manifest.rs`). Bindings
  regenerated cleanly to `apps/studio/ui/src/bindings/Manifest.ts`.

## 💭 Questions

### Q1. Is the dep-cache decl layout intended to differ from the workspace decl layout?

Connected to C2. `sync_from_hub` places decls at the workspace root
so the workspace loader's catalog scan picks them up. `fetch_to_cache`
inherits the same `materialize_version` and lands them outside the
version-pinned cache subtree. If the two consumers genuinely want
different layouts, `materialize_version` should take an enum (or two
call sites should diverge); using the same helper and hoping
`recipe_dir.parent()` lands in the right place for both is the
defect.

### Q2. Should `record_download` run on a re-sync that returns no new content?

`sync_into_same_workspace_at_higher_version_succeeds`
(`tests/hub_sync.rs:422-478`) confirms a re-sync overwrites at a
higher version and bumps the counter. The hub's `downloads` counter
on the package gets bumped for every re-sync from the same user
which inflates the count. Worth deciding whether "download" means
"first sync per user" or "every sync call."

---

## Verdict

**Request fix-up.**

C1 breaks the publish-after-fork roundtrip end-to-end; C2 breaks
`forage update`'s dep cache; C3 violates an explicit greenfield rule
both this plan and the roadmap called out. S1 + S2 are real
correctness gaps on the error-handling surface that the audit spec
called out specifically. The wire shape, deep-link plumbing, sidecar
contract, and shared operations location are all correct — fix the
five issues above and the branch is ready.
