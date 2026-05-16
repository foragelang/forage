//! Forage browser engine.
//!
//! Two execution modes share most of the code:
//! - **Replay** (this commit): walks a recipe's `browser` config against
//!   pre-recorded captures (HttpExchange + DocumentCapture variants).
//!   `captures.match` rules filter captures by URL pattern; the matched
//!   body becomes the iteration current value. `captures.document` runs
//!   against the recorded document HTML. Pure data — no webview needed.
//! - **Live**: drives a `wry`-backed `WebView`, injects a JS shim that
//!   intercepts fetch/XHR, captures responses as they fire, runs the
//!   pagination strategy (scroll until settle), then re-routes the
//!   captures through the same body execution as replay. Lands with
//!   Studio (R9) — the live driver needs a tao event loop, which the
//!   Tauri shell provides natively.

pub mod driver;
pub mod error;
pub mod replay;
pub mod shim;

pub use driver::BrowserReplayDriver;
pub use error::{BrowserError, BrowserResult};
pub use replay::{ReplayEngine, run_browser_replay};
pub use shim::{DUMP_DOCUMENT_JS, FETCH_INTERCEPT_JS, INTERACTIVE_OVERLAY_JS, SCROLL_TO_BOTTOM_JS};
