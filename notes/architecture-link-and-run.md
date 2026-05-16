# Link, run, deploy

## Why three layers

A `.forage` file goes through three transformations between source on
disk and records in an output store:

1. **Parse** (`forage_core::parse`) — source text → `ForageFile` AST.
2. **Link** (`forage_core::link`) — `ForageFile` + workspace →
   `LinkedModule`. This is the boundary that resolves composition
   stage references and produces the closure the runtime can execute
   without any further name lookup.
3. **Run** (`forage_core::run_recipe`) — `LinkedModule` + drivers →
   `Snapshot`. The runtime walks the module's body, dispatching each
   scraping stage through an engine driver and recursing into linked
   stages for composition bodies.

Each layer is owned by a different consumer:

| Layer            | Crate                | Consumers                              |
| ---------------- | -------------------- | -------------------------------------- |
| Parse            | `forage-core`        | every other layer + LSP / hub publish  |
| Validate-as-step | `forage-core`        | LSP, Studio's per-keystroke diagnostic |
| Link             | `forage-core`        | CLI, Studio, daemon                    |
| Run              | `forage-core`        | CLI, Studio, daemon                    |
| Drivers          | `forage-http`        | runtime per scraping HTTP stage        |
|                  | `forage-browser`     | runtime per scraping browser stage     |
|                  | Studio host          | runtime per live-browser stage         |

`Validate-as-step` is the single-recipe `forage_core::validate` entry
point — the per-file checker that Link calls internally on the root
and on every linked stage. The LSP and Studio's live-diagnostic
surface call it directly for snappy in-editor feedback; the runtime
path always reaches it through `link`.

## LinkedModule

`crates/forage-core/src/linked.rs`:

```rust
pub struct LinkedModule {
    pub root: LinkedRecipe,
    pub stages: BTreeMap<String, LinkedRecipe>,
    pub catalog: SerializableCatalog,
}

pub struct LinkedRecipe {
    pub file: ForageFile,
    pub emit_types: BTreeSet<String>,
}
```

`root` is the recipe the user invoked. `stages` is every recipe
transitively reachable through composition (keyed by header name).
`catalog` is the unified type catalog visible to every node — shared
types from the workspace plus every reachable file's local types,
deduplicated.

The module is the deployment unit. The daemon serializes a
`LinkedModule` to disk per version
(`<deployments>/<recipe>/v<n>/module.json`) and reads it back at run
time. No re-resolution per stage, no chase through `current_deployed`
— the closure is the contract.

## Linker

`crates/forage-core/src/link.rs`:

```rust
pub fn link(workspace: &Workspace, recipe_name: &str) -> Result<LinkOutcome, LinkError>;
pub fn link_standalone(file: ForageFile) -> LinkOutcome;

pub struct LinkOutcome {
    pub module: Option<LinkedModule>,
    pub report: ValidationReport,
}
```

`link` is a superset of `validate`:

- Validates the root recipe.
- For composition bodies, walks `compose A | B | C`, resolves each
  stage name through `Workspace::recipe_by_name`, and recursively
  links each reachable stage.
- Detects cycles via `check_compose_cycle`; unknown stages, hub-dep
  stages, and multi-type stages all surface the same diagnostics as
  the per-recipe validator.

`link_standalone` is the workspaceless mode — a parsed file with no
surrounding `forage.toml`. Composition stages can't resolve, so any
`compose A | B` from a standalone file surfaces as
`UnknownComposeStage`.

## Runtime

`crates/forage-core/src/runtime.rs`:

```rust
pub async fn run_recipe(
    module: &LinkedModule,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    options: &RunOptions,
    drivers: &Drivers<'_>,
) -> Result<Snapshot, RunError>;
```

The runtime dispatches per body kind:

- **Scraping / empty body.** Calls `drivers.<engine>.run_scraping(...)`
  through the [`RecipeDriver`] trait. The driver wraps the
  per-engine transport (HTTP live / HTTP replay / browser replay /
  browser live) and applies any per-driver state (progress sink,
  debugger).
- **Composition body.** Iterates `module.stages` in chain order,
  feeding stage N's emitted records to stage N+1 via [`PriorRecords`].
  Looks each stage up by name in `module.stage(...)` — no workspace
  consultation, no parse, no validate.

The runtime owns no I/O. Drivers do.

## Drivers

`forage-core` defines `RecipeDriver`; concrete impls live in:

- `forage-http::HttpDriver` — wraps an `Engine` over a chosen
  `Transport` (`LiveTransport` or `ReplayTransport`). Carries a
  progress sink + optional debugger.
- `forage-browser::BrowserReplayDriver` — replay against pre-recorded
  captures. No webview.
- Studio's `apps/studio/src-tauri/src/browser_driver.rs` — live
  webview driver. Studio implements `RecipeDriver` (via the daemon's
  `LiveBrowserDriver` bridge today) so the runtime dispatches into
  the Tauri-managed webview.
- `forage-http::UnsupportedDriver` — every error, used by callers
  that don't ship a browser implementation. The CLI uses this for
  the browser slot.

## Daemon deployment

`crates/forage-daemon/src/lib.rs`:

```rust
impl Daemon {
    pub fn deploy(&self, name: &str, module: LinkedModule) -> Result<DeployedVersion, DeployError>;
    pub fn load_deployed(&self, name: &str, version: u32) -> Result<DeployedRecord, DaemonError>;
}
```

`deploy` writes the whole `LinkedModule` to
`<deployments>/<name>/v<n>/module.json`. `load_deployed` reads it back
verbatim. Run-time execution goes through `forage_core::run_recipe`
against the loaded module — same code path as the CLI's `forage run`.

The daemon does not run a validator at deploy time. The linker has
already validated every node in the closure; the daemon's only
deploy-time invariant is that the module's root recipe name matches
the name argument.

## CLI

`apps/cli/src/main.rs`:

`run`, `test`, and `record` all go through `link_resolved` →
`run_recipe`. Composition recipes work for the first time from
`forage run` because the runtime knows how to walk a composition
body. The CLI ships HTTP (live + replay) and browser-replay drivers;
browser-live runs are Studio-only.

## Studio

`apps/studio/src-tauri/src/commands.rs`:

`deploy_recipe` links through the workspace and hands the module to
the daemon. `run_recipe` (the Tauri command) links through the
workspace, mints HTTP + browser drivers (including the live browser
adapter), and dispatches via `forage_core::run_recipe`. `persist_snapshot`
reads the linked module to derive the output schema.

## Notebook

`Daemon::run_composition(name, stages, inputs, flags)` is the
ephemeral composition surface — the user picks deployed stages from a
list, the daemon synthesizes a `LinkedModule` by loading each stage's
deployed closure, and dispatches through the same `run_recipe` path.
The notebook is the one place that *does* resolve "latest deployed"
per stage at run time, because that's its design: a dynamic chain
over current deployments, not a frozen closure.
