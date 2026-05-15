//! Forage HTTP engine.
//!
//! Drives HTTP-engine recipes against the network (live mode) or against
//! captured fixtures (replay mode). Handles auth flavors (staticHeader,
//! htmlPrime, session.*), pagination strategies (pageWithTotal, untilEmpty,
//! cursor), retry, rate limiting, cookie threading, and the session cache.
//!
//! The `native` feature (on by default) compiles in `LiveTransport`, the
//! `reqwest`-backed live transport. Disabling it leaves the engine,
//! request/response types, and `ReplayTransport` intact — what the hub
//! IDE consumes when compiled to wasm32.

pub mod auth;
pub mod body;
#[cfg(feature = "native")]
pub mod client;
pub mod debug;
pub mod engine;
pub mod error;
pub mod paginate;
pub mod progress;
pub mod transport;

#[cfg(feature = "native")]
pub use client::{LiveTransport, LiveTransportConfig};
pub use debug::{
    DebugFrame, DebugScope, Debugger, IterationPause, NoopDebugger, ResumeAction, StepPause,
};
pub use engine::{Engine, EngineConfig, PriorRecords};
pub use error::{HttpError, HttpResult};
pub use progress::{CaptureSink, FnSink, NoopSink, ProgressHandle, ProgressSink, RunEvent};
pub use transport::{HttpRequest, HttpResponse, ReplayTransport, Transport};
