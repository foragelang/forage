//! Engine progress events.
//!
//! The engine emits a stream of `RunEvent`s through a `ProgressSink` so
//! Studio (and the CLI, eventually) can show what the engine is doing
//! in real time instead of presenting a long silent stall. A run that
//! drives ten paginated requests and emits a few thousand records is
//! the common case for live mode — without progress, the UI has no
//! way to distinguish "working" from "hung."

use serde::Serialize;
use std::sync::Arc;
use ts_rs::TS;

use crate::debug::StepResponse;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum RunEvent {
    /// Run kicked off. Sent once at the start.
    RunStarted { recipe: String, replay: bool },
    /// Session auth login flow finished (or was not needed).
    Auth { flavor: String, status: String },
    /// A step request is about to go out.
    RequestSent {
        step: String,
        method: String,
        url: String,
        page: u32,
    },
    /// Response received. `status` is the HTTP code, `duration_ms` is the
    /// round-trip time including throttling.
    ResponseReceived {
        step: String,
        status: u16,
        #[ts(type = "number")]
        duration_ms: u64,
        bytes: usize,
    },
    /// A record was emitted into the snapshot. `total` is the running count
    /// for this type so far.
    Emitted { type_name: String, total: usize },
    /// Run completed successfully. `records` is the total record count;
    /// `duration_ms` is the wall-clock duration of the run.
    RunSucceeded {
        records: usize,
        #[ts(type = "number")]
        duration_ms: u64,
    },
    /// Run failed. The error message is the same one returned by `run()`.
    RunFailed {
        error: String,
        #[ts(type = "number")]
        duration_ms: u64,
    },
}

/// Anything the engine can hand events to. Studio wraps a Tauri Channel;
/// the CLI can wrap a logger; tests can use `NoopSink` or capture into a Vec.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: RunEvent);

    /// Fires once per executed step, right after the engine captures
    /// the response onto `DebugScope::step_responses`. Lets hosts
    /// stream the response shape to the UI independent of any pause
    /// — the studio surfaces this on a persistent "Responses" pane so
    /// the user can inspect a failed step even when the run aborts on
    /// 4xx/5xx before any pause site fires.
    ///
    /// Default no-op so CLI / daemon sinks compile unchanged.
    fn step_response_captured(&self, _step: &str, _response: &StepResponse) {}

    /// Hand the sink the **uncapped** response body for a step the
    /// engine just executed. Separate from `RunEvent::ResponseReceived`
    /// and `step_response_captured` because most sinks (CLI, daemon,
    /// tests) don't care about the bytes — only Studio's pop-out /
    /// "load full" code path needs them, and we don't want to ship
    /// multi-MB payloads through the regular event channel. The
    /// buffer is borrowed from the engine for the call's lifetime —
    /// hosts that need to keep the bytes around must copy. Default
    /// no-op so existing sinks compile unchanged.
    fn step_response_full_body(&self, _step: &str, _body: &[u8]) {}
}

/// Discard all events. Used by default when the caller doesn't care.
pub struct NoopSink;

impl ProgressSink for NoopSink {
    fn emit(&self, _: RunEvent) {}
}

/// Forwards every event to a closure. Convenient for tests and ad-hoc wiring.
pub struct FnSink<F>(pub F);

impl<F> ProgressSink for FnSink<F>
where
    F: Fn(RunEvent) + Send + Sync,
{
    fn emit(&self, event: RunEvent) {
        (self.0)(event);
    }
}

/// Captures every event into a shared Vec. Useful for assertions in tests.
#[derive(Default)]
pub struct CaptureSink(pub std::sync::Mutex<Vec<RunEvent>>);

impl CaptureSink {
    pub fn snapshot(&self) -> Vec<RunEvent> {
        self.0.lock().expect("captured events").clone()
    }
}

impl ProgressSink for CaptureSink {
    fn emit(&self, event: RunEvent) {
        self.0.lock().expect("captured events").push(event);
    }
}

/// Returned by `Engine::new`; the engine clones this on every emit.
pub type ProgressHandle = Arc<dyn ProgressSink>;
