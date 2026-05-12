//! Tauri commands exposed to the frontend.

use indexmap::IndexMap;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Notify;

use forage_browser::run_browser_replay;
use forage_core::ast::EngineKind;
use forage_core::{EvalValue, Snapshot, parse, validate};
use forage_http::{Engine, LiveTransport, ProgressSink, ReplayTransport, RunEvent};
use forage_hub::{AuthStore, AuthTokens, HubClient, RecipeMeta};

/// Tauri event name for streaming engine progress to the frontend.
pub const RUN_EVENT: &str = "forage:run-event";

use crate::browser_driver::{LiveRunOptions, run_live as run_browser_live};
use crate::library::{self, RecipeEntry};
use crate::state::StudioState;

#[derive(Serialize)]
pub struct ValidationOutcome {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub fn studio_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub fn list_recipes() -> Vec<RecipeEntry> {
    library::list_entries()
}

#[tauri::command]
pub fn load_recipe(slug: String) -> Result<String, String> {
    library::read_source(&slug).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_recipe(slug: String, source: String) -> Result<ValidationOutcome, String> {
    library::write_source(&slug, &source).map_err(|e| e.to_string())?;
    Ok(validate_source(&source))
}

#[tauri::command]
pub fn create_recipe() -> Result<String, String> {
    library::create_recipe(None).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_recipe(slug: String) -> Result<(), String> {
    library::delete_recipe(&slug).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct RunOutcome {
    pub ok: bool,
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
}

/// Bridges the engine's `ProgressSink` to a Tauri global event so the
/// frontend gets live updates. Standard `emit()` instead of a command-scoped
/// `Channel<T>` because the latter had observed delivery problems in 2.8
/// where events would not surface despite the run progressing; the global
/// event bus is well-trodden and survives the round-trip cleanly.
struct EmitterSink {
    app: AppHandle,
}

impl ProgressSink for EmitterSink {
    fn emit(&self, event: RunEvent) {
        // Emit failures are non-fatal (e.g. window closed mid-run): the run
        // continues even if the UI can't hear it anymore.
        let _ = self.app.emit(RUN_EVENT, &event);
    }
}

#[tauri::command]
pub async fn run_recipe(
    app: AppHandle,
    state: State<'_, crate::state::StudioState>,
    slug: String,
    replay: bool,
) -> Result<RunOutcome, String> {
    let source = library::read_source(&slug).map_err(|e| e.to_string())?;
    let recipe = match parse(&source) {
        Ok(r) => r,
        Err(e) => {
            return Ok(RunOutcome {
                ok: false,
                snapshot: None,
                error: Some(format!("{e}")),
            });
        }
    };
    let report = validate(&recipe);
    if report.has_errors() {
        let msgs: Vec<String> = report.errors().map(|e| e.message.clone()).collect();
        return Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(msgs.join("; ")),
        });
    }
    let raw_inputs = library::read_inputs(&slug);
    let mut inputs: IndexMap<String, EvalValue> = IndexMap::new();
    for (k, v) in raw_inputs {
        inputs.insert(k, EvalValue::from(&v));
    }
    let secrets = library::read_secrets_from_env(&recipe);
    let captures = if replay {
        library::read_captures(&slug)
    } else {
        Vec::new()
    };

    let sink: Arc<dyn ProgressSink> = Arc::new(EmitterSink { app: app.clone() });

    // Install a cancellation handle so `cancel_run` can interrupt this run.
    // Replaces any previous handle — Studio only runs one recipe at a time.
    let cancel = Arc::new(Notify::new());
    {
        let mut guard = state.run_cancel.lock().expect("run_cancel mutex");
        *guard = Some(cancel.clone());
    }

    let snapshot: Result<Snapshot, String> = match (recipe.engine_kind, replay) {
        (EngineKind::Http, true) => {
            let transport = ReplayTransport::new(captures);
            let engine = Engine::new(&transport).with_progress(sink);
            tokio::select! {
                biased;
                _ = cancel.notified() => Err("cancelled".into()),
                r = engine.run(&recipe, inputs, secrets) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Http, false) => {
            let transport = LiveTransport::new().map_err(|e| format!("{e}"))?;
            let engine = Engine::new(&transport).with_progress(sink);
            tokio::select! {
                biased;
                _ = cancel.notified() => Err("cancelled".into()),
                r = engine.run(&recipe, inputs, secrets) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Browser, true) => {
            run_browser_replay(&recipe, &captures, inputs, secrets).map_err(|e| format!("{e}"))
        }
        (EngineKind::Browser, false) => {
            // Open a Tauri WebviewWindow + inject the shim; collect
            // captures; route through the replay engine.
            run_browser_live(&app, &recipe, inputs, secrets, LiveRunOptions::default()).await
        }
    };

    // Clear the cancellation handle so a stale notify can't fire on the next run.
    {
        let mut guard = state.run_cancel.lock().expect("run_cancel mutex");
        *guard = None;
    }

    match snapshot {
        Ok(s) => Ok(RunOutcome {
            ok: true,
            snapshot: Some(s),
            error: None,
        }),
        Err(e) => Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(e),
        }),
    }
}

/// Cancel the currently-running recipe (if any). Idempotent — calling when
/// nothing is running is a no-op. Wakes the `tokio::select!` in `run_recipe`,
/// which drops the engine future and any in-flight reqwest call.
#[tauri::command]
pub fn cancel_run(state: State<'_, crate::state::StudioState>) -> Result<(), String> {
    let guard = state.run_cancel.lock().expect("run_cancel mutex");
    if let Some(n) = guard.as_ref() {
        n.notify_one();
    }
    Ok(())
}

#[tauri::command]
pub async fn publish_recipe(
    slug: String,
    hub_url: String,
    dry_run: bool,
) -> Result<RunOutcome, String> {
    let source = library::read_source(&slug).map_err(|e| e.to_string())?;
    let recipe = parse(&source).map_err(|e| format!("{e}"))?;
    if validate(&recipe).has_errors() {
        return Err("recipe failed validation".into());
    }
    let store = AuthStore::new();
    let host = host_of(&hub_url);
    let token = store.read(&host).ok().flatten().map(|t| t.access_token);

    if dry_run || token.is_none() {
        return Ok(RunOutcome {
            ok: true,
            snapshot: None,
            error: Some(format!(
                "would publish {} bytes to {hub_url}/v1/recipes/{}{}",
                source.len(),
                recipe.name,
                if token.is_none() {
                    " — not signed in"
                } else {
                    ""
                }
            )),
        });
    }
    let mut client = HubClient::new(&hub_url);
    if let Some(t) = token {
        client = client.with_token(t);
    }
    let meta = RecipeMeta {
        slug: recipe.name.clone(),
        version: 0,
        owner_login: None,
        display_name: Some(recipe.name.clone()),
        summary: None,
        tags: vec![],
        license: None,
        sha256: None,
        published_at: None,
    };
    match client.publish(&recipe.name, &source, &meta).await {
        Ok(r) => Ok(RunOutcome {
            ok: true,
            snapshot: None,
            error: Some(format!("published {} v{}", r.slug, r.version)),
        }),
        Err(e) => Err(format!("{e}")),
    }
}

#[tauri::command]
pub fn auth_whoami(hub_url: String) -> Result<Option<String>, String> {
    let host = host_of(&hub_url);
    let store = AuthStore::new();
    let tokens = store.read(&host).map_err(|e| e.to_string())?;
    Ok(tokens.map(|t| t.login))
}

#[derive(Serialize)]
pub struct DeviceStartOut {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[tauri::command]
pub async fn auth_start_device_flow(hub_url: String) -> Result<DeviceStartOut, String> {
    let s = forage_hub::device::start_device(&hub_url)
        .await
        .map_err(|e| e.to_string())?;
    Ok(DeviceStartOut {
        device_code: s.device_code,
        user_code: s.user_code,
        verification_url: s.verification_url,
        interval: s.interval,
        expires_in: s.expires_in,
    })
}

#[derive(Serialize)]
pub struct PollOutcome {
    pub status: String,
    pub login: Option<String>,
}

#[tauri::command]
pub async fn auth_poll_device(hub_url: String, device_code: String) -> Result<PollOutcome, String> {
    let r = forage_hub::device::poll_device(&hub_url, &device_code)
        .await
        .map_err(|e| e.to_string())?;
    if r.status == "ok" {
        if let (Some(access), Some(refresh), Some(user)) = (r.access_token, r.refresh_token, r.user)
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let tokens = AuthTokens {
                access_token: access,
                refresh_token: refresh,
                login: user.login.clone(),
                hub_url,
                issued_at: now,
                expires_at: now + r.expires_in.unwrap_or(3600) as i64,
            };
            AuthStore::new().write(&tokens).map_err(|e| e.to_string())?;
            return Ok(PollOutcome {
                status: "ok".into(),
                login: Some(user.login),
            });
        }
    }
    Ok(PollOutcome {
        status: r.status,
        login: None,
    })
}

#[tauri::command]
pub fn auth_logout(hub_url: String) -> Result<(), String> {
    let host = host_of(&hub_url);
    AuthStore::new().delete(&host).map_err(|e| e.to_string())
}

fn validate_source(source: &str) -> ValidationOutcome {
    match parse(source) {
        Ok(r) => {
            let report = validate(&r);
            let errors: Vec<String> = report
                .issues
                .iter()
                .filter(|i| matches!(i.severity, forage_core::Severity::Error))
                .map(|i| i.message.clone())
                .collect();
            let warnings: Vec<String> = report
                .issues
                .iter()
                .filter(|i| matches!(i.severity, forage_core::Severity::Warning))
                .map(|i| i.message.clone())
                .collect();
            ValidationOutcome {
                ok: errors.is_empty(),
                errors,
                warnings,
            }
        }
        Err(e) => ValidationOutcome {
            ok: false,
            errors: vec![format!("{e}")],
            warnings: vec![],
        },
    }
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split("//").nth(1).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .to_string()
}

// Quiet `dead_code` on the unused state for now — the dirty buffer cache
// is wired in when we add background autosave.
#[allow(dead_code)]
fn _state_typecheck(_s: State<'_, StudioState>) {}
