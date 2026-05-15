//! Bridge from `forage-daemon`'s engine-agnostic `LiveBrowserDriver`
//! trait to Studio's existing `browser_driver::run_live` helper.
//!
//! The daemon doesn't ship a `wry` driver — it can't, since the host
//! owns the Tauri `AppHandle` and the webview lifecycle. Studio plugs
//! this adapter into the daemon at boot via `Daemon::set_browser_driver`,
//! and from then on the scheduler's `run_once` can drive `engine
//! browser` recipes the same way it drives HTTP recipes.
//!
//! The adapter is `Send + Sync + 'static` because the daemon stores it
//! as `Arc<dyn LiveBrowserDriver>`. The only field is the cloned
//! `AppHandle`, which is `Send + Sync` by Tauri's contract.

use std::sync::Arc;

use async_trait::async_trait;
use forage_core::{EvalValue, ForageFile, RunOptions, Snapshot, TypeCatalog};
use forage_daemon::{BrowserDriverError, LiveBrowserDriver};
use forage_http::ProgressSink;
use indexmap::IndexMap;
use tauri::AppHandle;

use crate::browser_driver::{LiveRunOptions, run_live};

/// Adapter wired into the daemon at app init. Forwards browser-engine
/// runs to Studio's existing live driver. The `ProgressSink` passed by
/// the daemon is currently ignored: Studio's live driver doesn't yet
/// emit per-step events for browser runs, so threading a sink through
/// would be a no-op. When the live driver gains that capability the
/// sink plugs in here without changing the trait surface.
pub struct StudioLiveBrowserDriver {
    app: AppHandle,
}

impl StudioLiveBrowserDriver {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl LiveBrowserDriver for StudioLiveBrowserDriver {
    async fn run_live(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        _progress: Arc<dyn ProgressSink>,
        options: &RunOptions,
    ) -> Result<Snapshot, BrowserDriverError> {
        // `run_live` returns `Result<Snapshot, String>`. The daemon's
        // `BrowserDriverError` is a boxed `Error + Send + Sync`, so
        // promote the string error through `From<String>` for
        // `Box<dyn Error + Send + Sync>` — the standard erasure path.
        run_live(
            &self.app,
            recipe,
            catalog,
            inputs,
            secrets,
            LiveRunOptions::default(),
            options,
        )
        .await
        .map_err(Box::<dyn std::error::Error + Send + Sync>::from)
    }
}
