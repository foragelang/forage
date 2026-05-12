//! Forage HTTP engine.
//!
//! Drives HTTP-engine recipes against the network (live mode) or against
//! captured fixtures (replay mode). Handles auth flavors (staticHeader,
//! htmlPrime, session.*), pagination strategies (pageWithTotal, untilEmpty,
//! cursor), retry, rate limiting, cookie threading, and the session cache.

pub mod auth;
pub mod body;
pub mod client;
pub mod debug;
pub mod engine;
pub mod error;
pub mod paginate;
pub mod progress;
pub mod transport;

pub use client::{LiveTransport, LiveTransportConfig};
pub use debug::{DebugFrame, DebugScope, Debugger, NoopDebugger, ResumeAction, StepPause};
pub use engine::{Engine, EngineConfig};
pub use error::{HttpError, HttpResult};
pub use progress::{CaptureSink, FnSink, NoopSink, ProgressHandle, ProgressSink, RunEvent};
pub use transport::{HttpRequest, HttpResponse, ReplayTransport, Transport};
