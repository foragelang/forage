//! Browser-engine recipe config.

use serde::{Deserialize, Serialize};

use crate::ast::expr::{ExtractionExpr, Template};
use crate::ast::recipe::Statement;

/// Top-level browser-engine config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserConfig {
    pub initial_url: Template,
    #[serde(default)]
    pub age_gate: Option<AgeGateConfig>,
    #[serde(default)]
    pub dismissals: Option<DismissalConfig>,
    #[serde(default)]
    pub warmup_clicks: Vec<String>,
    pub observe: String,
    pub pagination: BrowserPaginationConfig,
    #[serde(default)]
    pub captures: Vec<CaptureRule>,
    #[serde(default)]
    pub document_capture: Option<DocumentCaptureRule>,
    #[serde(default)]
    pub interactive: Option<InteractiveConfig>,
}

/// M10 interactive session bootstrap config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractiveConfig {
    /// URL to load for bootstrap. Defaults to `BrowserConfig::initial_url`.
    #[serde(default)]
    pub bootstrap_url: Option<Template>,
    /// Domain substrings whose cookies should be persisted.
    #[serde(default)]
    pub cookie_domains: Vec<String>,
    /// Substring on the rendered HTML signaling **the stored session has
    /// expired** and the human needs to re-handshake. Triggers re-prompt;
    /// not a bypass mechanism — Forage doesn't try to defeat the challenge.
    #[serde(default)]
    pub session_expired_pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeGateConfig {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    /// Force a reload after submitting so the SPA boots fresh post-gate.
    #[serde(default = "default_true")]
    pub reload_after: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DismissalConfig {
    pub max_attempts: u32,
    /// Additional labels recognized beyond the runtime defaults.
    #[serde(default)]
    pub extra_labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserPaginationConfig {
    pub mode: BrowserPaginationMode,
    pub until: BrowserPaginateUntil,
    pub max_iterations: u32,
    /// Seconds between iterations.
    pub iteration_delay_secs: f64,
    /// Optional substring filter on captured request bodies (replay-mode seed picking).
    #[serde(default)]
    pub seed_filter: Option<String>,
    /// Replay-mode override: dotted-path → value (with `$i` template substitution).
    #[serde(default)]
    pub replay_override: Vec<(String, ExtractionExpr)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserPaginationMode {
    Scroll,
    Replay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserPaginateUntil {
    NoProgressFor(u32),
    CaptureCount { matching: String, at_least: u32 },
}

/// `captures.match { urlPattern: "...", for $x in $.body | ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRule {
    pub url_pattern: String,
    /// Iterate this expression within the matched response.
    pub iter_path: ExtractionExpr,
    pub body: Vec<Statement>,
}

/// `captures.document { for $x in $ | select(...) }` — fires once after settle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentCaptureRule {
    pub iter_path: ExtractionExpr,
    pub body: Vec<Statement>,
}
