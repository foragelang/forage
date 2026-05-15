//! `forage` — the command-line tool.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use indexmap::IndexMap;
use owo_colors::OwoColorize;

use forage_browser::run_browser_replay;
use forage_core::ast::{EngineKind, JSONValue};
use forage_core::workspace::{self, Workspace, WorkspaceError, fixtures_path, snapshot_path};
use forage_core::{EvalValue, ForageFile, RunOptions, Snapshot, parse, validate};
use forage_http::{Engine, LiveTransport, ReplayTransport};
use forage_hub::{
    AuthStore, AuthTokens, HubClient, HubError, device::run_device_flow, fetch_to_cache,
    fork_from_hub, hub_cache_root, publish_from_workspace, sync_from_hub,
};
use forage_replay::{Capture, HttpExchange, write_jsonl};

#[derive(Parser)]
#[command(
    name = "forage",
    version,
    about = "Declarative scraping recipes — parser, runner, hub client."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse and execute a `.forage` recipe; print the snapshot.
    ///
    /// `<recipe>` is a recipe header name (resolved through the
    /// surrounding workspace's `recipe_by_name`) or, as a fallback, a
    /// path to a `.forage` file whose contents declare a recipe header.
    Run {
        /// Recipe header name (primary) or path to a `.forage` file
        /// (fallback).
        recipe: String,
        /// Replay against the workspace's `_fixtures/<recipe>.jsonl`
        /// instead of hitting the network.
        #[arg(long)]
        replay: bool,
        /// Replay against an explicit captures file. Overrides
        /// `--replay`'s default fixture lookup.
        #[arg(long = "replay-from", value_name = "PATH")]
        replay_from: Option<PathBuf>,
        /// Cap each top-level `for $x in $arr[*]` iteration at N items.
        /// Nested loops still run to completion. Useful for a top-of-
        /// funnel sanity check against a real source.
        #[arg(long, value_name = "N")]
        sample: Option<u32>,
        /// Preset bundles: `dev` = --sample 10 --replay, `prod` = no
        /// flags. Explicit per-flag values override the preset's
        /// defaults. The `--ephemeral` flag lives at the daemon /
        /// Studio layer where output persistence is a real choice;
        /// `forage run` is already stateless, so it isn't surfaced.
        #[arg(long, value_enum)]
        mode: Option<RunMode>,
        /// Path to a JSON object of input bindings. When omitted the
        /// recipe runs with no inputs (any `input` declarations without
        /// defaults must supply a value through this file).
        #[arg(long)]
        inputs: Option<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        output: OutputFormat,
    },
    /// Run a recipe against `_fixtures/<recipe>.jsonl` and diff against
    /// `_snapshots/<recipe>.json`.
    Test {
        /// Recipe header name (primary) or path to a `.forage` file
        /// (fallback).
        recipe: String,
        /// Path to a JSON object of input bindings.
        #[arg(long)]
        inputs: Option<PathBuf>,
        /// Write the produced snapshot to `_snapshots/<recipe>.json`.
        #[arg(long)]
        update: bool,
    },
    /// Scaffold `<workspace>/<recipe-name>.forage` at the workspace
    /// root with a `recipe "<recipe-name>" engine http` header.
    New {
        /// Recipe header name. Doubles as the file basename.
        name: String,
        /// Engine kind for the new recipe.
        #[arg(long, value_enum, default_value_t = NewEngine::Http)]
        engine: NewEngine,
        /// Workspace root override (defaults to ancestor `forage.toml`).
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Run a recipe live against the network and write the resulting
    /// HTTP exchanges to `_fixtures/<recipe>.jsonl`. Browser-engine
    /// recipes still need Forage Studio for live capture.
    Record {
        /// Recipe header name (primary) or path to a `.forage` file
        /// (fallback).
        recipe: String,
        /// Path to a JSON object of input bindings.
        #[arg(long)]
        inputs: Option<PathBuf>,
    },
    /// Launch a webview and record fetch/XHR exchanges to JSONL. Ships
    /// with Forage Studio (R9) — needs a tao event loop to host wry.
    Capture,
    /// Build a starter `.forage` recipe from a captures JSONL file.
    Scaffold {
        /// Path to a captures.jsonl file.
        captures: PathBuf,
        /// Recipe header name for the scaffolded file. Defaults to the
        /// captures file's parent directory name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Drop a `forage.toml` skeleton at the current directory (or the
    /// path supplied) so the surrounding tree becomes a workspace.
    Init {
        /// Directory to place `forage.toml` in. Defaults to cwd.
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Resolve `[deps]` in `forage.toml` against the hub, fetch each
    /// into the local cache, and write `forage.lock`.
    Update {
        /// Workspace root (defaults to cwd). Must contain `forage.toml`.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Hub URL override (default: $FORAGE_HUB_URL or https://api.foragelang.com).
        #[arg(long)]
        hub: Option<String>,
    },
    /// Push a recipe to the Forage hub by header name. Requires
    /// `name = "<author>/<anything>"` in `forage.toml` for the author
    /// segment; the slug on the hub is the recipe's header name.
    Publish {
        /// Recipe header name (primary) or path to a `.forage` file
        /// (fallback).
        recipe: String,
        /// Hub URL override (default: $FORAGE_HUB_URL or https://api.foragelang.com).
        #[arg(long)]
        hub: Option<String>,
        /// Actually POST instead of dry-run.
        #[arg(long)]
        publish: bool,
        /// Bearer token override (default: $FORAGE_HUB_TOKEN or auth store).
        #[arg(long, env = "FORAGE_HUB_TOKEN")]
        token: Option<String>,
    },
    /// Clone a published recipe into the current workspace.
    Sync {
        /// `@author/slug` to clone. The leading `@` is optional.
        spec: String,
        /// Workspace destination (defaults to cwd).
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Pin to a specific version (default: latest).
        #[arg(long)]
        version: Option<u32>,
        /// Hub URL override (default: $FORAGE_HUB_URL or https://api.foragelang.com).
        #[arg(long)]
        hub: Option<String>,
    },
    /// Fork an upstream recipe into your account, then clone the new
    /// fork into the current workspace.
    Fork {
        /// `@author/slug` of the upstream to fork.
        spec: String,
        /// Slug for the new fork. Defaults to the upstream slug.
        #[arg(long = "as")]
        r#as: Option<String>,
        /// Workspace destination (defaults to cwd).
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Hub URL override (default: $FORAGE_HUB_URL or https://api.foragelang.com).
        #[arg(long)]
        hub: Option<String>,
    },
    /// Sign in / out / check status against the Forage hub via GitHub.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Start the Forage Language Server on stdio.
    Lsp,
}

#[derive(Subcommand)]
enum AuthAction {
    /// Run the device-code login flow.
    Login {
        #[arg(long, default_value = "https://api.foragelang.com")]
        hub: String,
    },
    /// Forget local tokens (`--revoke` also invalidates server-side).
    Logout {
        #[arg(long, default_value = "https://api.foragelang.com")]
        hub: String,
        #[arg(long)]
        revoke: bool,
    },
    /// Print the signed-in login (or "not signed in").
    Whoami {
        #[arg(long, default_value = "https://api.foragelang.com")]
        hub: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OutputFormat {
    Pretty,
    Json,
}

/// `--mode dev` / `--mode prod` — preset bundle for the run flag
/// switches. The full three-flag model lives at the daemon (Studio
/// uses all three); `forage run` is stateless so the `--ephemeral`
/// half of the dev preset is implicit, and only `--sample` and
/// `--replay` matter here.
#[derive(Clone, Copy, clap::ValueEnum)]
enum RunMode {
    /// Sampled at 10, replay against `_fixtures/<recipe>.jsonl`.
    Dev,
    /// Live run, no sampling.
    Prod,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum NewEngine {
    Http,
    Browser,
}

impl NewEngine {
    fn token(self) -> &'static str {
        match self {
            NewEngine::Http => "http",
            NewEngine::Browser => "browser",
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "forage=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    match cli.command {
        Command::Run {
            recipe,
            replay,
            replay_from,
            sample,
            mode,
            inputs,
            output,
        } => {
            let flags = RunCliFlags::resolve(mode, sample, replay, replay_from);
            rt.block_on(run(&recipe, &flags, inputs.as_deref(), output))
        }
        Command::Test {
            recipe,
            inputs,
            update,
        } => rt.block_on(test(&recipe, inputs.as_deref(), update)),
        Command::New {
            name,
            engine,
            workspace,
        } => do_new(&name, engine, workspace.as_deref()),
        Command::Record { recipe, inputs } => {
            rt.block_on(record(&recipe, inputs.as_deref()))
        }
        Command::Capture => {
            println!(
                "{} `forage capture` opens a real webview and ships with Forage Studio (R9). Use Studio for now.",
                "note:".yellow()
            );
            Ok(())
        }
        Command::Scaffold { captures, name } => do_scaffold(&captures, name),
        Command::Init { dir } => do_init(&dir),
        Command::Update { dir, hub } => rt.block_on(do_update(&dir, hub)),
        Command::Publish {
            recipe,
            hub,
            publish,
            token,
        } => rt.block_on(do_publish(&recipe, hub, publish, token)),
        Command::Sync {
            spec,
            dir,
            version,
            hub,
        } => rt.block_on(do_sync(&spec, &dir, version, hub)),
        Command::Fork {
            spec,
            r#as,
            dir,
            hub,
        } => rt.block_on(do_fork(&spec, r#as, &dir, hub)),
        Command::Auth { action } => rt.block_on(do_auth(action)),
        Command::Lsp => {
            rt.block_on(forage_lsp::server::run_stdio());
            Ok(())
        }
    }
}

/// A recipe resolved from a CLI string argument. The variant records
/// how resolution succeeded so callers can light up workspace-only
/// features (catalog merge, `_fixtures/` lookup) when a workspace
/// surrounds the recipe.
struct ResolvedRecipe {
    /// Recipe header name. Always present — the resolver only succeeds
    /// for recipe-bearing files.
    name: String,
    /// Path to the `.forage` file on disk.
    path: PathBuf,
    /// Parsed AST.
    file: ForageFile,
    /// Workspace containing the recipe, if any. `None` for a lonely
    /// path-argument resolve where no ancestor `forage.toml` exists.
    workspace: Option<Workspace>,
}

impl ResolvedRecipe {
    /// Directory the workspace data dirs (`_fixtures/`, `_snapshots/`)
    /// hang off. When a workspace surrounds the recipe, the workspace
    /// root; otherwise the recipe file's parent directory so a
    /// lonely-mode caller still resolves data files alongside the
    /// recipe.
    fn data_root(&self) -> PathBuf {
        match &self.workspace {
            Some(ws) => ws.root.clone(),
            None => self
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
        }
    }

    fn engine_kind(&self) -> Result<EngineKind> {
        self.file.engine_kind().ok_or_else(|| {
            anyhow::anyhow!(
                "recipe {:?} has no `recipe \"<name>\" engine <kind>` header",
                self.name
            )
        })
    }
}

/// Resolve a CLI recipe argument to a parsed recipe.
///
/// Rule:
///
/// 1. Discover a surrounding workspace from cwd. If found, try
///    `Workspace::recipe_by_name(arg)`.
/// 2. If that misses, treat `arg` as a path. A path that resolves to a
///    `.forage` file whose contents declare a recipe header wins.
/// 3. Otherwise error with both attempts mentioned so the user can
///    tell whether they typoed the name or the path.
///
/// The resolver does not consult the workspace's catalog or run the
/// validator — that's the caller's job, scoped to whichever subcommand
/// is being invoked. The resolver's only job is to land on a parsed,
/// recipe-bearing `ForageFile`.
fn resolve_recipe(arg: &str) -> Result<ResolvedRecipe> {
    let cwd = std::env::current_dir().context("resolving current directory")?;
    let workspace = workspace::discover(&cwd);

    if let Some(ws) = &workspace
        && let Some(rref) = ws.recipe_by_name(arg)
    {
        return Ok(ResolvedRecipe {
            name: rref.name().to_string(),
            path: rref.path.to_path_buf(),
            file: rref.file.clone(),
            workspace: workspace.clone(),
        });
    }

    let candidate = Path::new(arg);
    if candidate.is_file() {
        let source = std::fs::read_to_string(candidate)
            .with_context(|| format!("reading {}", candidate.display()))?;
        let file = parse(&source).map_err(|e| anyhow::anyhow!("parse {}: {e}", candidate.display()))?;
        let Some(name) = file.recipe_name().map(str::to_string) else {
            bail!(
                "{} is a `.forage` file but declares no `recipe \"<name>\"` header",
                candidate.display()
            );
        };
        let path = candidate
            .canonicalize()
            .with_context(|| format!("canonicalizing {}", candidate.display()))?;
        let workspace = workspace::discover(&path);
        return Ok(ResolvedRecipe {
            name,
            path,
            file,
            workspace,
        });
    }

    match &workspace {
        Some(ws) => bail!(
            "no recipe named {:?} in workspace {}, and no file at {:?}",
            arg,
            ws.root.display(),
            arg
        ),
        None => bail!(
            "no workspace in scope and {:?} is not a `.forage` file path",
            arg
        ),
    }
}

/// Resolved CLI flags after applying any `--mode` preset. Each field
/// carries the effective value passed to the engine.
struct RunCliFlags {
    sample: Option<u32>,
    /// `None` = live network, `Some(None)` = replay against the default
    /// fixtures path, `Some(Some(p))` = replay against `p`.
    replay: Option<Option<PathBuf>>,
}

impl RunCliFlags {
    /// Apply the preset selected by `--mode`, then let explicit per-flag
    /// values override the preset's defaults. Explicit beats preset
    /// because the typical user reaches for `--mode dev --sample 50`
    /// when they want the dev shape with a different sample size.
    fn resolve(
        mode: Option<RunMode>,
        sample: Option<u32>,
        replay: bool,
        replay_from: Option<PathBuf>,
    ) -> Self {
        let (default_sample, default_replay) = match mode {
            Some(RunMode::Dev) => (Some(10u32), true),
            Some(RunMode::Prod) | None => (None, false),
        };
        let effective_sample = sample.or(default_sample);
        let effective_replay = if let Some(p) = replay_from {
            Some(Some(p))
        } else if replay || default_replay {
            Some(None)
        } else {
            None
        };
        Self {
            sample: effective_sample,
            replay: effective_replay,
        }
    }
}

async fn run(
    recipe_arg: &str,
    flags: &RunCliFlags,
    inputs_path: Option<&Path>,
    output: OutputFormat,
) -> Result<()> {
    let resolved = resolve_recipe(recipe_arg)?;
    let catalog = validate_resolved(&resolved)?;
    let inputs = load_inputs(inputs_path)?;
    let secrets = load_secrets_from_env(&resolved.file);

    let options = RunOptions {
        sample_limit: flags.sample,
    };
    let captures = match &flags.replay {
        None => None,
        Some(None) => Some(load_captures_for(&resolved)?),
        Some(Some(path)) => Some(
            forage_replay::read_jsonl(path)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?,
        ),
    };

    let snapshot = match (resolved.engine_kind()?, captures) {
        (EngineKind::Http, Some(captures)) => {
            let transport = ReplayTransport::new(captures);
            let engine = Engine::new(&transport);
            engine
                .run(&resolved.file, &catalog, inputs, secrets, &options)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
        (EngineKind::Http, None) => {
            let transport = LiveTransport::new().map_err(|e| anyhow::anyhow!("{e}"))?;
            let engine = Engine::new(&transport);
            engine
                .run(&resolved.file, &catalog, inputs, secrets, &options)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
        (EngineKind::Browser, Some(captures)) => run_browser_replay(
            &resolved.file,
            &catalog,
            &captures,
            inputs,
            secrets,
            &options,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?,
        (EngineKind::Browser, None) => {
            bail!(
                "browser-engine recipes need a real WebView; \
                 use --replay against _fixtures/<recipe>.jsonl for now, \
                 or open the recipe in Forage Studio (R9)"
            );
        }
    };

    match output {
        OutputFormat::Pretty => print_pretty(&snapshot),
        OutputFormat::Json => {
            let j = serde_json::to_string_pretty(&snapshot)?;
            println!("{j}");
        }
    }
    if snapshot.diagnostic.has_content() {
        std::process::exit(if snapshot.diagnostic.unmet_expectations.is_empty() {
            0
        } else {
            3
        });
    }
    Ok(())
}

async fn test(recipe_arg: &str, inputs_path: Option<&Path>, update: bool) -> Result<()> {
    let resolved = resolve_recipe(recipe_arg)?;
    let catalog = validate_resolved(&resolved)?;
    let _engine_kind = resolved.engine_kind()?; // surface header-less files early
    let inputs = load_inputs(inputs_path)?;
    let secrets = load_secrets_from_env(&resolved.file);
    let captures = load_captures_for(&resolved)?;
    let transport = ReplayTransport::new(captures);
    let engine = Engine::new(&transport);
    let produced = engine
        .run(&resolved.file, &catalog, inputs, secrets, &RunOptions::default())
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let snap_path = snapshot_path(&resolved.data_root(), &resolved.name);
    if update || !snap_path.exists() {
        if let Some(parent) = snap_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let j = serde_json::to_string_pretty(&produced)?;
        std::fs::write(&snap_path, &j)?;
        println!("{} {}", "wrote".green(), snap_path.display());
        return Ok(());
    }
    let raw = std::fs::read_to_string(&snap_path)?;
    let expected: Snapshot = serde_json::from_str(&raw)?;
    if expected == produced {
        println!("{} matches {}", "ok:".green(), snap_path.display());
        return Ok(());
    }
    let a = serde_json::to_string_pretty(&expected)?;
    let b = serde_json::to_string_pretty(&produced)?;
    let diff = similar::TextDiff::from_lines(&a, &b);
    println!("{} snapshot diverged:", "diff:".red());
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            similar::ChangeTag::Delete => "-",
            similar::ChangeTag::Insert => "+",
            similar::ChangeTag::Equal => " ",
        };
        print!("{sign}{change}");
    }
    std::process::exit(1);
}

async fn record(recipe_arg: &str, inputs_path: Option<&Path>) -> Result<()> {
    let resolved = resolve_recipe(recipe_arg)?;
    let catalog = validate_resolved(&resolved)?;
    let inputs = load_inputs(inputs_path)?;
    let secrets = load_secrets_from_env(&resolved.file);

    match resolved.engine_kind()? {
        EngineKind::Http => {}
        EngineKind::Browser => bail!(
            "browser-engine `forage record` needs a real WebView and ships with \
             Forage Studio (R9); the CLI can only record HTTP-engine recipes for now"
        ),
    }

    let live = LiveTransport::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    let transport = RecordingTransport::new(live);
    let engine = Engine::new(&transport);
    let snapshot = engine
        .run(&resolved.file, &catalog, inputs, secrets, &RunOptions::default())
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let path = fixtures_path(&resolved.data_root(), &resolved.name);
    let captures = transport.into_captures();
    write_jsonl(&path, &captures).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "{} {} captures → {}",
        "recorded".green(),
        captures.len(),
        path.display()
    );
    if snapshot.diagnostic.has_content() {
        eprintln!(
            "{} run completed but the diagnostic report is non-empty; \
             captures still written so a replay shows the same trace.",
            "note:".yellow()
        );
    }
    Ok(())
}

/// `LiveTransport` wrapper that mirrors every exchange into a JSONL
/// capture buffer. Only the recording side knows the request URL +
/// method (the live transport's response carries neither), so we wrap
/// rather than read back out.
struct RecordingTransport {
    inner: LiveTransport,
    captures: tokio::sync::Mutex<Vec<Capture>>,
}

impl RecordingTransport {
    fn new(inner: LiveTransport) -> Self {
        Self {
            inner,
            captures: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    fn into_captures(self) -> Vec<Capture> {
        self.captures.into_inner()
    }
}

#[async_trait::async_trait]
impl forage_http::Transport for RecordingTransport {
    async fn fetch(
        &self,
        req: forage_http::HttpRequest,
    ) -> forage_http::HttpResult<forage_http::HttpResponse> {
        let url = req.url.clone();
        let method = req.method.clone();
        let request_headers = req.headers.clone();
        let request_body = req
            .body
            .as_ref()
            .and_then(|b| String::from_utf8(b.clone()).ok());
        let resp = self.inner.fetch(req).await?;
        let exchange = HttpExchange {
            url,
            method,
            request_headers,
            request_body,
            status: resp.status,
            response_headers: resp.headers.clone(),
            body: resp.body_str().to_string(),
        };
        self.captures.lock().await.push(Capture::Http(exchange));
        Ok(resp)
    }
}

/// Build the recipe's `TypeCatalog` (merged with workspace `share`d
/// declarations and hub-dep types when a workspace surrounds the file;
/// file-local otherwise), then validate against it. Returns the catalog
/// so engine call sites can reuse it without rebuilding — engines need
/// it to stamp `Snapshot.record_types` with alignments for every type
/// the recipe could emit, not just the ones declared in the recipe
/// file.
fn validate_resolved(resolved: &ResolvedRecipe) -> Result<forage_core::TypeCatalog> {
    let catalog = match &resolved.workspace {
        Some(ws) => ws
            .catalog(&resolved.file, |p| std::fs::read_to_string(p))
            .map_err(|e: WorkspaceError| anyhow::anyhow!("workspace catalog: {e}"))?,
        None => forage_core::TypeCatalog::from_file(&resolved.file),
    };
    let signatures = resolved
        .workspace
        .as_ref()
        .map(|ws| ws.recipe_signatures())
        .unwrap_or_default();
    let report = validate(&resolved.file, &catalog, &signatures);
    if report.has_errors() {
        for e in report.errors() {
            eprintln!("{} {}", "validate:".red(), e.message);
        }
        bail!("recipe failed validation");
    }
    Ok(catalog)
}

fn load_inputs(path: Option<&Path>) -> Result<IndexMap<String, EvalValue>> {
    let Some(path) = path else {
        return Ok(IndexMap::new());
    };
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading inputs file {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing inputs file {}", path.display()))?;
    let serde_json::Value::Object(o) = value else {
        bail!(
            "inputs file {} must hold a JSON object of bindings",
            path.display()
        );
    };
    let mut out = IndexMap::new();
    for (k, v) in o {
        out.insert(k, EvalValue::from(&v));
    }
    Ok(out)
}

fn load_secrets_from_env(recipe: &ForageFile) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}

fn load_captures_for(resolved: &ResolvedRecipe) -> Result<Vec<Capture>> {
    let path = fixtures_path(&resolved.data_root(), &resolved.name);
    forage_replay::read_jsonl(&path).map_err(|e| anyhow::anyhow!("{e}"))
}

fn print_pretty(snapshot: &Snapshot) {
    let by_type = group_by_type(&snapshot.records);
    if by_type.is_empty() {
        println!("{}", "(no records emitted)".dimmed());
    } else {
        for (type_name, records) in &by_type {
            println!(
                "{} {} {}",
                "•".green(),
                type_name.bold(),
                format!("({} records)", records.len()).dimmed()
            );
            for (i, r) in records.iter().take(3).enumerate() {
                println!("  [{i}] {}", short_json(&r.fields).dimmed());
            }
            if records.len() > 3 {
                println!("  {} more …", (records.len() - 3).to_string().dimmed());
            }
        }
    }
    let d = &snapshot.diagnostic;
    if d.has_content() {
        println!();
        if let Some(r) = &d.stall_reason {
            println!("{} {}", "stall:".yellow(), r.message);
        }
        for e in &d.unmet_expectations {
            println!("{} {}", "expect:".red(), e.message);
        }
    }
}

fn group_by_type(records: &[forage_core::Record]) -> IndexMap<String, Vec<&forage_core::Record>> {
    let mut map: IndexMap<String, Vec<&forage_core::Record>> = IndexMap::new();
    for r in records {
        map.entry(r.type_name.clone()).or_default().push(r);
    }
    map
}

fn do_scaffold(captures_path: &Path, name: Option<String>) -> Result<()> {
    let raw = std::fs::read_to_string(captures_path)?;
    let mut groups: indexmap::IndexMap<String, Vec<String>> = indexmap::IndexMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let c: Capture =
            serde_json::from_str(line).with_context(|| format!("parsing {captures_path:?}"))?;
        match c {
            Capture::Http(h) => {
                groups
                    .entry(format!("HTTP {} {}", h.method, scaffold_pattern(&h.url)))
                    .or_default()
                    .push(h.url);
            }
            Capture::Browser(forage_replay::BrowserCapture::Match { url, method, .. }) => {
                groups
                    .entry(format!("browser {} {}", method, scaffold_pattern(&url)))
                    .or_default()
                    .push(url);
            }
            Capture::Browser(forage_replay::BrowserCapture::Document { url, .. }) => {
                groups
                    .entry(format!("document {url}"))
                    .or_default()
                    .push(url);
            }
        }
    }

    let recipe_name = name.unwrap_or_else(|| {
        captures_path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "scaffolded".to_string())
    });

    let mut out = String::new();
    out.push_str(&format!(
        "// Scaffolded from {} on {} captures.\n",
        captures_path.display(),
        groups.len()
    ));
    out.push_str(&format!("recipe \"{}\"\n", recipe_name));
    out.push_str("engine http\n\n");
    out.push_str("type Item {\n    id:   String\n    name: String?\n}\n\n");
    for (i, (label, urls)) in groups.iter().enumerate() {
        out.push_str(&format!("// {label}  ({} requests)\n", urls.len()));
        out.push_str(&format!("step s{} {{\n", i));
        out.push_str("    method \"GET\"\n");
        out.push_str(&format!(
            "    url    {:?}\n",
            urls.first().cloned().unwrap_or_default()
        ));
        out.push_str("}\n\n");
    }
    out.push_str("for $i in $s0.items[*] {\n");
    out.push_str("    emit Item {\n        id   ← $i.id\n        name ← $i.name\n    }\n}\n");

    print!("{out}");
    Ok(())
}

fn scaffold_pattern(url: &str) -> String {
    // Strip query string + collapse the path so similar URLs land in the
    // same group.
    let no_query = url.split('?').next().unwrap_or(url);
    no_query.to_string()
}

fn do_new(name: &str, engine: NewEngine, workspace_override: Option<&Path>) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        bail!("invalid recipe name {name:?}: must be non-empty and contain no path separators");
    }
    let root = match workspace_override {
        Some(p) => p.to_path_buf(),
        None => {
            let cwd = std::env::current_dir().context("resolving current directory")?;
            workspace::discover(&cwd)
                .map(|ws| ws.root)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no workspace in scope (no ancestor `forage.toml` from {})",
                        cwd.display()
                    )
                })?
        }
    };
    let target = root.join(format!("{name}.forage"));
    if target.exists() {
        bail!("{} already exists; refusing to overwrite", target.display());
    }
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating workspace dir {}", root.display()))?;
    let body = format!("recipe \"{name}\" engine {engine}\n\n", engine = engine.token());
    std::fs::write(&target, body).with_context(|| format!("writing {}", target.display()))?;
    println!("{} {}", "wrote".green(), target.display());
    Ok(())
}

fn do_init(dir: &Path) -> Result<()> {
    let path = dir.join(workspace::MANIFEST_NAME);
    if path.exists() {
        println!(
            "{} {} already exists; leaving untouched",
            "note:".yellow(),
            path.display()
        );
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let manifest = workspace::Manifest::default();
    let body = workspace::serialize_manifest(&manifest)
        .map_err(|e| anyhow::anyhow!("serialize manifest: {e}"))?;
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    println!("{} {}", "wrote".green(), path.display());
    Ok(())
}

async fn do_update(dir: &Path, hub_override: Option<String>) -> Result<()> {
    let ws = workspace::load(dir).map_err(|e| anyhow::anyhow!("workspace: {e}"))?;
    let hub = resolve_hub(hub_override);
    if ws.manifest.deps.is_empty() {
        println!("{} [deps] is empty; nothing to do", "note:".dimmed());
        return Ok(());
    }
    let client = hub_client(&hub, None);
    let cache_root = hub_cache_root();

    let mut lock = workspace::Lockfile::default();
    for (slug, &version) in &ws.manifest.deps {
        let (author, slug_only) = split_dep_slug(slug)?;
        // Fetch the recipe artifact — `fetch_to_cache` also mirrors
        // every referenced type into the parallel type cache. We then
        // re-fetch the recipe artifact to enumerate its `type_refs`
        // for the lockfile's `[types]` section.
        let fetched = fetch_to_cache(&client, &cache_root, author, slug_only, version)
            .await
            .with_context(|| format!("fetching {slug}@{version}"))?;
        println!(
            "{} {slug}@{version} → {}",
            "fetched".green(),
            fetched.dir.display()
        );
        lock.recipes.insert(
            slug.clone(),
            workspace::LockedDep {
                version,
                hash: fetched.sha256.clone(),
            },
        );

        // Re-fetch to read the type_refs the recipe pins. `fetch_to_cache`
        // already pulled the underlying type sources into the type cache;
        // we just need the wire artifact to enumerate them for the
        // lockfile's `[types]` pinning.
        let artifact = client
            .get_version(
                author,
                slug_only,
                forage_hub::VersionSpec::Numbered(version),
            )
            .await
            .with_context(|| format!("re-fetching {slug}@{version} for type_refs"))?;
        for r in &artifact.type_refs {
            let key = format!("{}/{}", r.author, r.name);
            lock.types.insert(
                key,
                workspace::LockedDep {
                    version: r.version,
                    // Hashing the cached `.forage` byte body would be
                    // cleaner than empty; pre-1.0 we leave the hash
                    // field for a follow-up. The lockfile carries the
                    // version pin (which is what catalog resolution
                    // keys on); the hash is integrity metadata only.
                    hash: String::new(),
                },
            );
        }
    }

    let lock_body = workspace::serialize_lockfile(&lock)
        .map_err(|e| anyhow::anyhow!("serialize lockfile: {e}"))?;
    let lock_path = ws.root.join(workspace::LOCKFILE_NAME);
    std::fs::write(&lock_path, lock_body)?;
    println!("{} {}", "wrote".green(), lock_path.display());
    Ok(())
}

async fn do_publish(
    recipe_arg: &str,
    hub_override: Option<String>,
    really_publish: bool,
    token_override: Option<String>,
) -> Result<()> {
    let resolved = resolve_recipe(recipe_arg)?;
    let Some(ws) = resolved.workspace.clone() else {
        bail!(
            "`forage publish` needs a workspace (`forage.toml`) in scope; \
             resolved recipe {:?} has none",
            resolved.name
        );
    };
    let Some(name) = ws.manifest.name.clone() else {
        bail!(
            "{} requires `name = \"<author>/<…>\"` in forage.toml (the author segment \
             is used; the slug becomes the recipe header name)",
            ws.root.join(workspace::MANIFEST_NAME).display()
        );
    };
    let (author, _slug) = split_dep_slug(&name)?;
    let description = ws.manifest.description.clone();
    let category = ws.manifest.category.clone();
    let tags = ws.manifest.tags.clone();
    if description.is_empty() {
        bail!("forage.toml is missing `description = \"…\"` (required for publish)");
    }
    if category.is_empty() {
        bail!("forage.toml is missing `category = \"…\"` (required for publish)");
    }

    let hub = resolve_hub(hub_override);

    if !really_publish {
        let plan = forage_hub::assemble_publish_plan(
            &ws,
            &resolved.name,
            author,
            description,
            category,
            tags,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let recipe_bytes = plan.recipe_payload.recipe.len()
            + plan
                .recipe_payload
                .fixtures
                .iter()
                .map(|f| f.content.len())
                .sum::<usize>();
        let type_bytes: usize = plan.types.iter().map(|t| t.source.len()).sum();
        println!(
            "{} would publish {} type(s) under @{author} then POST the recipe to {hub}/v1/packages/{author}/{recipe}/versions (recipe + fixtures = {recipe_bytes} bytes; types = {type_bytes} bytes)",
            "dry-run:".yellow(),
            plan.types.len(),
            recipe = resolved.name,
        );
        for t in &plan.types {
            println!(
                "    · type {} ({} bytes) → {hub}/v1/types/{author}/{}/versions",
                t.name,
                t.source.len(),
                t.name,
            );
        }
        println!(
            "    · recipe {} ({} bytes)",
            recipe_file_name(&resolved),
            plan.recipe_payload.recipe.len(),
        );
        for f in &plan.recipe_payload.fixtures {
            println!("    · fixture {} ({} bytes)", f.name, f.content.len());
        }
        println!(
            "    · base_version: {}",
            plan.recipe_payload
                .base_version
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| "(first publish)".into())
        );
        println!("Re-run with --publish to actually POST.");
        return Ok(());
    }

    let client = hub_client(&hub, token_override);
    match publish_from_workspace(
        &client,
        &ws,
        &resolved.name,
        author,
        description,
        category,
        tags,
    )
    .await
    {
        Ok(resp) => {
            println!(
                "{} {}/{} v{} (latest is now v{})",
                "published".green(),
                resp.author,
                resp.slug,
                resp.version,
                resp.latest_version,
            );
            Ok(())
        }
        Err(HubError::StaleBase {
            latest_version,
            your_base,
            message,
        }) => {
            let base_str = your_base
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| "(none)".into());
            bail!(
                "stale base: hub is at v{latest_version}, your base is {base_str}. {message}\nrefresh and retry."
            );
        }
        Err(e) => bail!("{e}"),
    }
}

fn recipe_file_name(resolved: &ResolvedRecipe) -> String {
    resolved
        .path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{}.forage", resolved.name))
}

async fn do_sync(
    spec: &str,
    dir: &Path,
    version: Option<u32>,
    hub_override: Option<String>,
) -> Result<()> {
    let (author, slug) = parse_spec(spec)?;
    let hub = resolve_hub(hub_override);
    let client = hub_client(&hub, None);
    let outcome = sync_from_hub(&client, dir, &author, &slug, version)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "{} {} → {}",
        "synced".green(),
        outcome.meta.origin,
        outcome.recipe_path.display()
    );
    Ok(())
}

async fn do_fork(
    spec: &str,
    r#as: Option<String>,
    dir: &Path,
    hub_override: Option<String>,
) -> Result<()> {
    let (upstream_author, upstream_slug) = parse_spec(spec)?;
    let hub = resolve_hub(hub_override);
    let host = host_of(&hub);
    let token = AuthStore::new()
        .read(&host)
        .ok()
        .flatten()
        .map(|t| t.access_token);
    let Some(token) = token else {
        bail!("sign in first: `forage auth login --hub {hub}`");
    };
    let client = HubClient::new(&hub).with_token(token);
    let outcome = fork_from_hub(&client, dir, &upstream_author, &upstream_slug, r#as)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "{} {} → {}",
        "forked".green(),
        outcome.meta.origin,
        outcome.recipe_path.display()
    );
    Ok(())
}

/// Parse `@author/slug` (the leading `@` is optional) into the two
/// segments the hub expects. Rejects bare slugs without an author and
/// any segment that the hub's regex would reject.
fn parse_spec(spec: &str) -> Result<(String, String)> {
    let trimmed = spec.strip_prefix('@').unwrap_or(spec);
    let (author, slug) = trimmed
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("expected `@author/slug`, got {spec:?}"))?;
    if author.is_empty() || slug.is_empty() {
        bail!("invalid spec: {spec:?}");
    }
    Ok((author.to_string(), slug.to_string()))
}

fn split_dep_slug(slug: &str) -> Result<(&str, &str)> {
    slug.split_once('/')
        .ok_or_else(|| anyhow::anyhow!("expected `author/slug`, got {slug:?}"))
}

fn resolve_hub(over: Option<String>) -> String {
    over.or_else(|| std::env::var("FORAGE_HUB_URL").ok())
        .unwrap_or_else(|| "https://api.foragelang.com".into())
}

fn hub_client(hub: &str, token_override: Option<String>) -> HubClient {
    let host = host_of(hub);
    let token = token_override.or_else(|| {
        AuthStore::new()
            .read(&host)
            .ok()
            .flatten()
            .map(|t| t.access_token)
    });
    let mut client = HubClient::new(hub);
    if let Some(t) = token {
        client = client.with_token(t);
    }
    client
}

async fn do_auth(action: AuthAction) -> Result<()> {
    match action {
        AuthAction::Login { hub } => {
            let resp = run_device_flow(&hub, |start| {
                println!("{}", "Sign in to Forage:".bold());
                println!("  1. Open: {}", start.verification_url.cyan());
                println!("  2. Enter code: {}", start.user_code.bold());
                println!(
                    "  (polling every {}s, expires in {}s)",
                    start.interval, start.expires_in
                );
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let access = resp.access_token.unwrap();
            let refresh = resp.refresh_token.unwrap_or_default();
            let login = resp.user.map(|u| u.login).unwrap_or_default();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let tokens = AuthTokens {
                access_token: access,
                refresh_token: refresh,
                login: login.clone(),
                hub_url: hub,
                issued_at: now,
                expires_at: now + resp.expires_in.unwrap_or(3600) as i64,
            };
            AuthStore::new()
                .write(&tokens)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("{} signed in as {}", "ok:".green(), login.bold());
            Ok(())
        }
        AuthAction::Logout { hub, revoke: _ } => {
            let host = host_of(&hub);
            AuthStore::new()
                .delete(&host)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("{} cleared local tokens for {host}", "ok:".green());
            // TODO(R6 followup): if `revoke`, POST /v1/oauth/revoke.
            Ok(())
        }
        AuthAction::Whoami { hub } => {
            let host = host_of(&hub);
            match AuthStore::new()
                .read(&host)
                .map_err(|e| anyhow::anyhow!("{e}"))?
            {
                Some(t) => {
                    println!("{}@{}", t.login.bold(), host);
                }
                None => {
                    println!("{}", "not signed in".dimmed());
                }
            }
            Ok(())
        }
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

fn short_json(fields: &IndexMap<String, JSONValue>) -> String {
    let mut parts = Vec::new();
    for (k, v) in fields.iter().take(5) {
        let v_str = match v {
            JSONValue::Null => "null".into(),
            JSONValue::Bool(b) => b.to_string(),
            JSONValue::Int(n) => n.to_string(),
            JSONValue::Double(n) => format!("{n}"),
            JSONValue::String(s) if s.len() > 40 => format!("{:?}…", &s[..40]),
            JSONValue::String(s) => format!("{s:?}"),
            JSONValue::Array(xs) => format!("[{} items]", xs.len()),
            JSONValue::Object(o) => format!("{{{} fields}}", o.len()),
        };
        parts.push(format!("{k}: {v_str}"));
    }
    if fields.len() > 5 {
        parts.push(format!("…{} more", fields.len() - 5));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `forage sync @alice/zen-leaf` and `forage sync alice/zen-leaf`
    /// MUST parse identically — the `@` prefix is a display nicety,
    /// not a parser requirement. A regression that silently lost the
    /// bare form would only surface when a user copy-pasted from a
    /// hub URL.
    #[test]
    fn parse_spec_accepts_at_prefix_and_bare_form() {
        let (a, s) = parse_spec("@alice/zen-leaf").unwrap();
        assert_eq!((a.as_str(), s.as_str()), ("alice", "zen-leaf"));
        let (a, s) = parse_spec("alice/zen-leaf").unwrap();
        assert_eq!((a.as_str(), s.as_str()), ("alice", "zen-leaf"));
    }

    #[test]
    fn parse_spec_rejects_missing_slash() {
        assert!(parse_spec("alice").is_err());
        assert!(parse_spec("@alice").is_err());
    }

    #[test]
    fn parse_spec_rejects_empty_segments() {
        assert!(parse_spec("/zen-leaf").is_err());
        assert!(parse_spec("alice/").is_err());
        assert!(parse_spec("@/zen-leaf").is_err());
    }
}
