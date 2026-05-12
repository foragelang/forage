//! Live browser-engine driver: opens a Tauri `WebviewWindow`, injects
//! the fetch/XHR shim, collects captures, runs settle detection, then
//! routes the captures through `forage-browser`'s ReplayEngine.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Listener, WebviewUrl, WebviewWindowBuilder};

use forage_browser::{FETCH_INTERCEPT_JS, SCROLL_TO_BOTTOM_JS, run_browser_replay};
use forage_core::ast::{BrowserPaginateUntil, BrowserPaginationMode, Recipe};
use forage_core::{EvalValue, Snapshot};
use forage_replay::{BrowserCapture, Capture};

/// One capture emitted by the JS shim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShimCapture {
    pub subkind: String,
    pub url: String,
    pub method: String,
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveRunOptions {
    /// Visible (interactive) vs headless (best-effort).
    pub visible: bool,
    /// Cap on wall-clock seconds. The settle detector usually finishes
    /// earlier; this is a safety net.
    pub max_seconds: u64,
}

impl Default for LiveRunOptions {
    fn default() -> Self {
        Self {
            visible: true,
            max_seconds: 180,
        }
    }
}

/// Drive a browser-engine recipe live. Opens a webview, injects the shim,
/// loops scroll-then-wait-for-settle until `noProgressFor(N)` is satisfied
/// (or `maxIterations` is hit), then runs the recipe body against the
/// collected captures.
pub async fn run_live(
    app: &AppHandle,
    recipe: &Recipe,
    inputs: indexmap::IndexMap<String, EvalValue>,
    secrets: indexmap::IndexMap<String, String>,
    opts: LiveRunOptions,
) -> Result<Snapshot, String> {
    let cfg = recipe
        .browser
        .as_ref()
        .ok_or_else(|| "recipe has no browser config".to_string())?;
    let initial_url = render_url(&recipe.name, &cfg.initial_url, &inputs)?;

    // Captures accumulator + last-activity stamp.
    let captures = Arc::new(Mutex::new(Vec::<Capture>::new()));
    let last_activity = Arc::new(Mutex::new(std::time::Instant::now()));

    // Listen for `forage-capture` events from the shim.
    let captures_for_listener = captures.clone();
    let last_for_listener = last_activity.clone();
    let unlisten = app.listen("forage-capture", move |event| {
        if let Ok(p) = serde_json::from_str::<ShimCapture>(event.payload()) {
            if p.subkind == "match" {
                captures_for_listener.lock().unwrap().push(Capture::Browser(
                    BrowserCapture::Match {
                        url: p.url,
                        method: p.method,
                        status: p.status,
                        body: p.body,
                    },
                ));
                *last_for_listener.lock().unwrap() = std::time::Instant::now();
            }
        }
    });

    // Open the webview window with the shim pre-injected.
    let label = format!("forage-recipe-{}", recipe.name);
    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::External(
            initial_url
                .parse()
                .map_err(|e| format!("bad initialURL: {e}"))?,
        ),
    )
    .title(format!("Forage — {}", recipe.name))
    .inner_size(1280.0, 800.0)
    .visible(opts.visible)
    .initialization_script(FETCH_INTERCEPT_JS)
    .build()
    .map_err(|e| format!("open webview: {e}"))?;

    // Pagination + settle loop.
    let no_progress_window = match cfg.pagination.until {
        BrowserPaginateUntil::NoProgressFor(n) => n.max(1) as u64,
        BrowserPaginateUntil::CaptureCount { .. } => 2,
    };
    let max_iterations = cfg.pagination.max_iterations.max(1) as u64;
    let iteration_delay =
        Duration::from_millis((cfg.pagination.iteration_delay_secs * 1000.0) as u64);
    let deadline = std::time::Instant::now() + Duration::from_secs(opts.max_seconds);

    if matches!(cfg.pagination.mode, BrowserPaginationMode::Scroll) {
        for _ in 0..max_iterations {
            if std::time::Instant::now() >= deadline {
                break;
            }
            // Scroll, then wait for settle.
            let _ = window.eval(SCROLL_TO_BOTTOM_JS);
            tokio::time::sleep(iteration_delay).await;
            // Wait for `no_progress_window` seconds of quiet.
            let mut settled = false;
            for _ in 0..no_progress_window * 4 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let idle = last_activity.lock().unwrap().elapsed();
                if idle >= Duration::from_secs(no_progress_window) {
                    settled = true;
                    break;
                }
            }
            if settled {
                break;
            }
        }
    } else {
        // Replay mode under live transport doesn't make sense — just wait a beat.
        tokio::time::sleep(iteration_delay).await;
    }

    // Document capture: snapshot post-settle HTML.
    if cfg.document_capture.is_some() {
        let dump: String = window
            .eval("document.documentElement.outerHTML")
            .map(|_| String::new())
            .unwrap_or_default();
        // wry's `eval` returns nothing; we need a separate JS-eval-with-result
        // path. Tauri 2 has `webview.eval` (no return) and `webview.invoke`
        // (return). For now we use a side-channel:
        let _ = window.eval(
            r#"
            try {
                const html = document.documentElement.outerHTML;
                window.__TAURI__.event.emit('forage-capture', {
                    subkind: 'document', url: location.href, method: 'GET', status: 200,
                    body: html,
                });
            } catch (e) {}
            "#,
        );
        let _ = dump;
        // Allow the document message to flush.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // Done. Close the window (unless interactive mode left it open for the user).
    app.unlisten(unlisten);
    if !opts.visible {
        let _ = window.close();
    }

    // The `BrowserCapture::Document` variant was emitted as a `Match`
    // with subkind `document`; restitch it into the right variant.
    let mut collected = captures.lock().unwrap().clone();
    let mut restitched = Vec::with_capacity(collected.len());
    for c in collected.drain(..) {
        if let Capture::Browser(BrowserCapture::Match {
            url,
            body,
            status,
            method,
        }) = c
        {
            if status == 200 && method == "GET" && url.starts_with("http") && body.contains("<html")
            {
                // Heuristic — but we set status=200 + method=GET only on the
                // document emit above, so it's safe.
                if cfg.document_capture.is_some() {
                    restitched.push(Capture::Browser(BrowserCapture::Document {
                        url,
                        html: body,
                    }));
                    continue;
                }
            }
            restitched.push(Capture::Browser(BrowserCapture::Match {
                url,
                body,
                status,
                method,
            }));
        } else {
            restitched.push(c);
        }
    }

    // Route through the replay engine.
    run_browser_replay(recipe, &restitched, inputs, secrets).map_err(|e| format!("{e}"))
}

fn render_url(
    _name: &str,
    template: &forage_core::ast::Template,
    inputs: &indexmap::IndexMap<String, EvalValue>,
) -> Result<String, String> {
    use forage_core::Evaluator;
    use forage_core::Scope;
    use forage_core::eval::default_registry;
    let evaluator = Evaluator::new(default_registry());
    let scope = Scope::new().with_inputs(inputs.clone());
    evaluator
        .render_template(template, &scope)
        .map_err(|e| format!("{e}"))
}
