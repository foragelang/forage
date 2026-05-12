//! Shared capture/fixture types + replayers.
//!
//! `forage-http` and `forage-browser` both produce captures during live runs
//! and consume them during replay. This crate owns the serialized format
//! (`captures.jsonl`) and the replayer implementations.
//!
//! Filled in during R2 (HTTPReplayer) + R4 (BrowserReplayer).
