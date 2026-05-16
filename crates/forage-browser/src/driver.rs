//! `RecipeDriver` adapters for the browser engine's replay path.
//!
//! Replay-mode browser runs are pure data — no webview, no event loop
//! — so the adapter is a thin wrapper around [`run_browser_replay`].
//! Live-mode browser runs need a host-supplied driver (Studio provides
//! one via `LiveBrowserDriver`); the corresponding `RecipeDriver` impl
//! lives in the host crate so it can reach the webview's
//! `AppHandle`.

use async_trait::async_trait;
use indexmap::IndexMap;

use forage_core::ast::ForageFile;
use forage_core::{
    EvalValue, PriorRecords, RecipeDriver, RunError, RunOptions, Snapshot, TypeCatalog,
};
use forage_replay::Capture;

use crate::replay::run_browser_replay;

/// Browser-engine driver backed by pre-recorded captures. The
/// captures are shared across every browser-engine stage in a
/// composition chain — same shape the existing daemon path used.
///
/// Live-mode browser-engine runs go through a separate driver (Studio
/// implements `RecipeDriver` directly against its webview).
pub struct BrowserReplayDriver<'c> {
    captures: &'c [Capture],
}

impl<'c> BrowserReplayDriver<'c> {
    pub fn new(captures: &'c [Capture]) -> Self {
        Self { captures }
    }
}

#[async_trait]
impl RecipeDriver for BrowserReplayDriver<'_> {
    async fn run_scraping(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
        _prior: PriorRecords,
    ) -> Result<Snapshot, RunError> {
        // Browser-engine downstream stages are rejected upstream by
        // the runtime — by the time we reach here, prior records are
        // empty by construction.
        run_browser_replay(recipe, catalog, self.captures, inputs, secrets, options)
            .map_err(|e| RunError::Driver(format!("browser engine: {e}")))
    }
}
