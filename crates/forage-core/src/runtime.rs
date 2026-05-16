//! Driver-agnostic recipe runtime.
//!
//! [`run_recipe`] is the single execution entry point. It consumes a
//! [`LinkedModule`] (already resolved by the linker) and dispatches on
//! body kind:
//!
//! - **Scraping bodies** delegate to a [`RecipeDriver`] supplied by the
//!   caller. Drivers wrap the HTTP transport, the browser driver, or
//!   any replay-backed substitute; they own the per-engine knobs
//!   (progress sinks, transport state). Forage-core itself touches no
//!   I/O.
//! - **Composition bodies** walk `module.stages` (pre-resolved at link
//!   time), recursing into [`run_recipe_node`] for each stage. The
//!   prior stage's emitted records thread into the next stage's input
//!   slot via [`PriorRecords`].
//!
//! No name resolution happens here — the linked module is the source
//! of truth. No `daemon.current_deployed(...)` chase per stage; the
//! composition's behavior is frozen at link / deploy time.

use std::pin::Pin;

use async_trait::async_trait;
use indexmap::IndexMap;

use crate::ast::{EngineKind, ForageFile, RecipeBody};
use crate::eval::EvalValue;
use crate::eval::RunOptions;
use crate::linked::{LinkedModule, LinkedRecipe};
use crate::snapshot::{Record, Snapshot};
use crate::workspace::TypeCatalog;

/// Records to seed a downstream recipe with — the upstream stage's
/// emissions, threaded into the next stage's input slot at run
/// boundary. `PriorRecords::default()` is the standalone-recipe case
/// (no prior); composition stages 2+ pass the prior stage's `records`.
#[derive(Debug, Default, Clone)]
pub struct PriorRecords {
    pub records: Vec<Record>,
    /// The downstream recipe's output type that the prior records
    /// claim to be. The engine matches against the recipe's input
    /// declarations to find the slot to bind them to. Empty when no
    /// prior records flow.
    pub type_name: String,
}

/// One engine-side driver. Forage-core dispatches scraping bodies
/// through this trait; concrete implementations live in `forage-http`
/// (HTTP transport, replay) and `forage-browser` (browser replay, host-
/// supplied live driver in Studio).
///
/// The trait sits in forage-core so the runtime can hand off without
/// taking a direct dependency on either engine crate. Driver
/// implementations carry their own progress sink, transport state, and
/// replay captures — those are not the runtime's concern.
#[async_trait]
pub trait RecipeDriver: Send + Sync {
    async fn run_scraping(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
        prior: PriorRecords,
    ) -> Result<Snapshot, RunError>;
}

/// Bundle of drivers the runtime dispatches to per stage. One trait
/// object per engine kind; the runtime reads the stage's
/// `engine_kind()` and picks the matching driver. A caller that
/// doesn't expect to run browser-engine recipes can supply a
/// `RecipeDriver` that errors at the first call instead of plumbing a
/// real browser driver.
pub struct Drivers<'a> {
    pub http: &'a dyn RecipeDriver,
    pub browser: &'a dyn RecipeDriver,
}

/// Errors that can surface from a driver-dispatched run. Driver
/// implementations wrap engine-specific errors into the `Driver`
/// variant; the runtime adds shape-level diagnostics for things only
/// it can detect (e.g. browser-engine downstream of a non-empty prior,
/// which the browser engine doesn't yet support).
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("driver: {0}")]
    Driver(String),
    #[error(
        "compose stage '{stage}' is browser-engine; browser engines can't yet receive prior records"
    )]
    BrowserDownstream { stage: String },
    #[error("recipe has no header — runtime requires a recipe-bearing module")]
    HeaderlessRoot,
    #[error(
        "composition stage '{0}' is not in the linked module — the linker should have rejected the recipe"
    )]
    UnresolvedStage(String),
}

/// Execute a linked recipe end-to-end. The root's body kind decides
/// how the runtime dispatches:
///
/// - Scraping / empty body → one call into `drivers.<engine>` for the
///   root.
/// - Composition body → walks `module.stages` in chain order, feeding
///   each stage's snapshot into the next stage's `prior`.
///
/// `inputs` flows into the root only; downstream composition stages
/// consume the upstream record stream and ignore the outer `inputs`.
pub async fn run_recipe(
    module: &LinkedModule,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    options: &RunOptions,
    drivers: &Drivers<'_>,
) -> Result<Snapshot, RunError> {
    let catalog: TypeCatalog = module.catalog.clone().into();
    run_recipe_node(
        &module.root,
        module,
        &catalog,
        inputs,
        secrets,
        options,
        drivers,
        PriorRecords::default(),
    )
    .await
}

/// Recursive driver: each node consults its body kind and either
/// hands off to the engine driver (scraping/empty) or walks its own
/// composition chain.
///
/// The boxed future return is necessary for the recursive composition
/// path — `async fn` can't recurse without it.
#[allow(clippy::too_many_arguments)]
fn run_recipe_node<'a>(
    node: &'a LinkedRecipe,
    module: &'a LinkedModule,
    catalog: &'a TypeCatalog,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    options: &'a RunOptions,
    drivers: &'a Drivers<'a>,
    prior: PriorRecords,
) -> Pin<Box<dyn std::future::Future<Output = Result<Snapshot, RunError>> + Send + 'a>> {
    Box::pin(async move {
        match &node.file.body {
            RecipeBody::Composition(_) => {
                run_composition_node(node, module, catalog, inputs, secrets, options, drivers).await
            }
            RecipeBody::Scraping { .. } | RecipeBody::Empty => {
                run_scraping_node(node, catalog, inputs, secrets, options, drivers, prior).await
            }
        }
    })
}

async fn run_scraping_node(
    node: &LinkedRecipe,
    catalog: &TypeCatalog,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    options: &RunOptions,
    drivers: &Drivers<'_>,
    prior: PriorRecords,
) -> Result<Snapshot, RunError> {
    let engine_kind = node.file.engine_kind().ok_or(RunError::HeaderlessRoot)?;
    if engine_kind == EngineKind::Browser && !prior.records.is_empty() {
        let stage_name = node.file.recipe_name().unwrap_or("<unknown>").to_string();
        return Err(RunError::BrowserDownstream { stage: stage_name });
    }
    let driver = match engine_kind {
        EngineKind::Http => drivers.http,
        EngineKind::Browser => drivers.browser,
    };
    driver
        .run_scraping(&node.file, catalog, inputs, secrets, options, prior)
        .await
}

async fn run_composition_node(
    node: &LinkedRecipe,
    module: &LinkedModule,
    catalog: &TypeCatalog,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    options: &RunOptions,
    drivers: &Drivers<'_>,
) -> Result<Snapshot, RunError> {
    let comp = node
        .file
        .body
        .composition()
        .expect("run_composition_node called on a non-composition node");
    let mut prior = PriorRecords::default();
    let mut stage_inputs = inputs;
    let mut last_snapshot: Option<Snapshot> = None;
    for stage_ref in &comp.stages {
        // Hub-dep stages (`author.is_some()`) and unknown stages are
        // rejected at link time. By the time we reach a deployed
        // composition every stage resolves through `module.stage`.
        let stage = module
            .stage(&stage_ref.name)
            .ok_or_else(|| RunError::UnresolvedStage(stage_ref.name.clone()))?;
        // Pass the composition's secrets through to inner stages; the
        // composition file doesn't redeclare its own `secret` set, so
        // anything the outer caller supplied is what inner stages see
        // by name. The HTTP engine still surfaces missing-secret
        // errors per recipe.
        let inner_secrets = secrets.clone();
        let snapshot = run_recipe_node(
            stage,
            module,
            catalog,
            stage_inputs.clone(),
            inner_secrets,
            options,
            drivers,
            prior.clone(),
        )
        .await?;
        // Stage 2+: the recipe consumes the upstream stream and
        // ignores the composition's outer `inputs` (which are stage-1
        // only).
        stage_inputs.clear();
        prior = derive_prior(&snapshot);
        last_snapshot = Some(snapshot);
    }
    last_snapshot.ok_or_else(|| {
        RunError::Driver(format!(
            "composition '{}' has zero stages — validator should have rejected",
            node.file.recipe_name().unwrap_or("<unknown>"),
        ))
    })
}

/// Build the `PriorRecords` carrier for the next stage from this
/// stage's snapshot. When the upstream emitted multiple types, the
/// downstream stage's input-slot lookup picks the matching one — but
/// the per-stage validator already pinned each output to a single
/// type, so in practice this is a one-type stream.
fn derive_prior(snap: &Snapshot) -> PriorRecords {
    let mut type_name = String::new();
    let mut records: Vec<Record> = Vec::with_capacity(snap.records.len());
    for rec in &snap.records {
        if type_name.is_empty() {
            type_name = rec.type_name.clone();
        }
        records.push(rec.clone());
    }
    PriorRecords { records, type_name }
}

/// Resolve a recipe's "terminal" emit set: the types the user can
/// expect to find in the output store after a run. Used by the
/// daemon's `derive_schema` to build per-type tables before the run
/// fires.
///
/// - Scraping body → the recipe's declared `emits` clause (when
///   present) or the inferred body emits (otherwise).
/// - Composition body → walk the chain to the terminal stage and
///   recurse; if the chain's terminal is itself a composition,
///   recurse again.
///
/// Returns an empty set when the chain ends at an unresolved stage —
/// the linker's diagnostic catches that case at validate time, so
/// runtime callers reaching here on an empty set will fail their
/// own output-store schema check downstream.
pub fn resolve_terminal_emits(
    node: &LinkedRecipe,
    module: &LinkedModule,
) -> std::collections::BTreeSet<String> {
    if !node.emit_types.is_empty() {
        return node.emit_types.clone();
    }
    let Some(comp) = node.file.body.composition() else {
        return std::collections::BTreeSet::new();
    };
    let Some(terminal) = comp.stages.last() else {
        return std::collections::BTreeSet::new();
    };
    let Some(stage) = module.stage(&terminal.name) else {
        return std::collections::BTreeSet::new();
    };
    resolve_terminal_emits(stage, module)
}
