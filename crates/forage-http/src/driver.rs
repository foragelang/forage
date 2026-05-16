//! `RecipeDriver` adapter for the HTTP engine.
//!
//! Wraps an [`Engine`] behind the engine-agnostic [`RecipeDriver`]
//! trait that `forage_core::run_recipe` dispatches through. The
//! adapter doesn't add behavior — it just bridges the trait's
//! object-safe shape onto the engine's concrete `run_with_prior` call
//! so the runtime can hand a [`LinkedRecipe`]-scoped recipe off
//! without depending on this crate.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;

use forage_core::ast::ForageFile;
use forage_core::{
    EvalValue, PriorRecords, RecipeDriver, RunError, RunOptions, Snapshot, TypeCatalog,
};

use crate::debug::Debugger;
use crate::engine::Engine;
use crate::progress::ProgressSink;
use crate::transport::Transport;

/// HTTP-engine driver. Constructed per run; the driver carries the
/// transport (live or replay), the progress sink, and an optional
/// debugger. Each `run_scraping` invocation builds an [`Engine`] over
/// these references and dispatches the recipe through the engine's
/// `run_with_prior` path.
pub struct HttpDriver<'t> {
    transport: &'t dyn Transport,
    progress: Arc<dyn ProgressSink>,
    debugger: Option<Arc<dyn Debugger>>,
}

impl<'t> HttpDriver<'t> {
    pub fn new(transport: &'t dyn Transport) -> Self {
        Self {
            transport,
            progress: Arc::new(crate::progress::NoopSink),
            debugger: None,
        }
    }

    pub fn with_progress(mut self, progress: Arc<dyn ProgressSink>) -> Self {
        self.progress = progress;
        self
    }

    pub fn with_debugger(mut self, debugger: Arc<dyn Debugger>) -> Self {
        self.debugger = Some(debugger);
        self
    }
}

#[async_trait]
impl RecipeDriver for HttpDriver<'_> {
    async fn run_scraping(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
        prior: PriorRecords,
    ) -> Result<Snapshot, RunError> {
        let mut engine = Engine::new(self.transport).with_progress(self.progress.clone());
        if let Some(dbg) = &self.debugger {
            engine = engine.with_debugger(dbg.clone());
        }
        engine
            .run_with_prior(recipe, catalog, inputs, secrets, options, prior)
            .await
            .map_err(|e| RunError::Driver(format!("http engine: {e}")))
    }
}

/// `RecipeDriver` that always errors. Useful for callers (e.g. the
/// CLI today) that don't ship a browser implementation but still need
/// to satisfy the `Drivers` shape — a recipe whose engine is
/// `browser` and that reaches the runtime is surfaced as a clean
/// error rather than panicking on an unwrap somewhere.
pub struct UnsupportedDriver {
    label: String,
}

impl UnsupportedDriver {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

#[async_trait]
impl RecipeDriver for UnsupportedDriver {
    async fn run_scraping(
        &self,
        _recipe: &ForageFile,
        _catalog: &TypeCatalog,
        _inputs: IndexMap<String, EvalValue>,
        _secrets: IndexMap<String, String>,
        _options: &RunOptions,
        _prior: PriorRecords,
    ) -> Result<Snapshot, RunError> {
        Err(RunError::Driver(self.label.clone()))
    }
}
