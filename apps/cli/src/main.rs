//! `forage` — the command-line tool.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use indexmap::IndexMap;
use owo_colors::OwoColorize;

use forage_browser::run_browser_replay;
use forage_core::ast::{EngineKind, JSONValue};
use forage_core::workspace::fixtures_path;
use forage_core::{EvalValue, Snapshot, parse, validate};
use forage_http::{Engine, LiveTransport, ReplayTransport};
use forage_hub::{
    AuthStore, AuthTokens, HubClient, HubError, device::run_device_flow, fetch_to_cache,
    fork_from_hub, hub_cache_root, publish_from_workspace, sync_from_hub,
};
use forage_replay::Capture;

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
    /// Parse and execute a .forage recipe; print the snapshot.
    ///
    /// Discovers the surrounding workspace (ancestor `forage.toml`) and
    /// validates against the merged catalog. Without a workspace, runs
    /// in lonely-recipe mode.
    Run {
        /// Recipe path. Accepts either a recipe directory
        /// (`<slug>/recipe.forage` is appended) or the recipe file itself.
        recipe_path: PathBuf,
        /// Replay against the workspace's `_fixtures/<recipe>.jsonl`
        /// instead of hitting the network.
        #[arg(long)]
        replay: bool,
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        output: OutputFormat,
    },
    /// Run a recipe against fixtures and diff against an expected snapshot.
    Test {
        recipe_dir: PathBuf,
        /// Write the produced snapshot to expected.snapshot.json.
        #[arg(long)]
        update: bool,
    },
    /// Launch a webview and record fetch/XHR exchanges to JSONL. Ships
    /// with Forage Studio (R9) — needs a tao event loop to host wry.
    Capture,
    /// Build a starter .forage recipe from a captures JSONL file.
    Scaffold {
        /// Path to a captures.jsonl file.
        captures: PathBuf,
        /// Recipe name (defaults to the parent directory name).
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
    /// Push the workspace's recipe to the Forage hub. Requires
    /// `name = "<author>/<slug>"` in `forage.toml`. Sends the atomic
    /// per-version artifact (recipe + workspace decls + fixtures +
    /// snapshot + base_version).
    Publish {
        /// Workspace root (defaults to cwd).
        #[arg(default_value = ".")]
        dir: PathBuf,
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
            recipe_path,
            replay,
            output,
        } => rt.block_on(run(&recipe_path, replay, output)),
        Command::Test { recipe_dir, update } => rt.block_on(test(&recipe_dir, update)),
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
            dir,
            hub,
            publish,
            token,
        } => rt.block_on(do_publish(&dir, hub, publish, token)),
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

async fn run(recipe_path: &Path, replay: bool, output: OutputFormat) -> Result<()> {
    let (recipe_dir, recipe_file) = resolve_recipe_dir(recipe_path)?;
    let recipe = load_recipe_at(&recipe_dir, &recipe_file)?;
    let inputs = load_inputs(&recipe_dir)?;
    let secrets = load_secrets_from_env(&recipe);

    let engine_kind = recipe
        .engine_kind()
        .ok_or_else(|| anyhow::anyhow!("recipe file has no `recipe \"<name>\" engine <kind>` header"))?;
    let snapshot = match (engine_kind, replay) {
        (EngineKind::Http, true) => {
            let captures = load_captures_for(&recipe_file, &recipe)?;
            let transport = ReplayTransport::new(captures);
            let engine = Engine::new(&transport);
            engine
                .run(&recipe, inputs, secrets)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
        (EngineKind::Http, false) => {
            let transport = LiveTransport::new().map_err(|e| anyhow::anyhow!("{e}"))?;
            let engine = Engine::new(&transport);
            engine
                .run(&recipe, inputs, secrets)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
        (EngineKind::Browser, true) => {
            let captures = load_captures_for(&recipe_file, &recipe)?;
            run_browser_replay(&recipe, &captures, inputs, secrets)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
        (EngineKind::Browser, false) => {
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

async fn test(recipe_dir: &Path, update: bool) -> Result<()> {
    let recipe_file = recipe_dir.join("recipe.forage");
    let recipe = load_recipe(recipe_dir)?;
    if recipe.engine_kind().is_none() {
        bail!(
            "`forage test` requires a file with a `recipe \"<name>\" engine <kind>` header; \
             a header-less declarations file can't be run"
        );
    }
    let inputs = load_inputs(recipe_dir)?;
    let secrets = load_secrets_from_env(&recipe);
    let captures = load_captures_for(&recipe_file, &recipe)?;
    let transport = ReplayTransport::new(captures);
    let engine = Engine::new(&transport);
    let produced = engine
        .run(&recipe, inputs, secrets)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let expected_path = recipe_dir.join("expected.snapshot.json");
    if update || !expected_path.exists() {
        let j = serde_json::to_string_pretty(&produced)?;
        std::fs::write(&expected_path, &j)?;
        println!("{} {}", "wrote".green(), expected_path.display());
        return Ok(());
    }
    let raw = std::fs::read_to_string(&expected_path)?;
    let expected: Snapshot = serde_json::from_str(&raw)?;
    if expected == produced {
        println!("{} matches expected snapshot", "ok:".green());
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

fn load_recipe(dir: &Path) -> Result<forage_core::ForageFile> {
    load_recipe_at(dir, &dir.join("recipe.forage"))
}

fn load_recipe_at(_dir: &Path, path: &Path) -> Result<forage_core::ForageFile> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let recipe = parse(&source).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    let catalog = build_catalog_for(path, &recipe)?;
    let report = validate(&recipe, &catalog);
    if report.has_errors() {
        for e in report.errors() {
            eprintln!("{} {}", "validate:".red(), e.message);
        }
        bail!("recipe failed validation");
    }
    Ok(recipe)
}

/// Build the type catalog for a recipe at `recipe_path`. If the recipe
/// sits inside a workspace (ancestor `forage.toml`), the catalog folds
/// in workspace declarations files plus cached hub-dep declarations.
/// Otherwise lonely-recipe mode — recipe-local types only.
fn build_catalog_for(
    recipe_path: &Path,
    recipe: &forage_core::ForageFile,
) -> Result<forage_core::TypeCatalog> {
    if let Some(ws) = forage_core::workspace::discover(recipe_path) {
        return ws
            .catalog(recipe, |p| std::fs::read_to_string(p))
            .map_err(|e| anyhow::anyhow!("workspace catalog: {e}"));
    }
    Ok(forage_core::TypeCatalog::from_file(recipe))
}

fn load_inputs(dir: &Path) -> Result<IndexMap<String, EvalValue>> {
    let path = dir.join("fixtures").join("inputs.json");
    if !path.exists() {
        return Ok(IndexMap::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let mut out = IndexMap::new();
    if let serde_json::Value::Object(o) = value {
        for (k, v) in o {
            out.insert(k, EvalValue::from(&v));
        }
    }
    Ok(out)
}

fn load_secrets_from_env(recipe: &forage_core::ForageFile) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}

/// Resolve the workspace root and recipe name for `recipe_file`, then
/// read `<root>/_fixtures/<recipe>.jsonl`. Falls back to the recipe
/// file's parent directory as the root when no `forage.toml` is found
/// up the tree (lonely-recipe mode), so a single-file workspace still
/// resolves captures from a sibling `_fixtures/` directory.
fn load_captures_for(
    recipe_file: &Path,
    recipe: &forage_core::ForageFile,
) -> Result<Vec<Capture>> {
    let recipe_name = recipe
        .recipe_name()
        .ok_or_else(|| anyhow::anyhow!("recipe file has no `recipe \"<name>\"` header"))?;
    let root = match forage_core::workspace::discover(recipe_file) {
        Some(ws) => ws.root,
        None => recipe_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let path = fixtures_path(&root, recipe_name);
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

/// Resolve `forage run <path>` to a (recipe-dir, recipe-file) pair.
///
/// - A directory: append `recipe.forage` to it.
/// - A file named `recipe.forage`: use it directly; the recipe dir is
///   its parent.
/// - Any other file: surface a clear error. Silently rewriting
///   `/path/to/foo.forage` into `/path/to/recipe.forage` hides whatever
///   the user actually meant.
fn resolve_recipe_dir(path: &Path) -> Result<(PathBuf, PathBuf)> {
    if path.is_dir() {
        return Ok((path.to_path_buf(), path.join("recipe.forage")));
    }
    if path.is_file() {
        let leaf = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if leaf == "recipe.forage" {
            let dir = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            return Ok((dir, path.to_path_buf()));
        }
        bail!(
            "forage run requires either a recipe.forage path or a directory containing one; got {}",
            path.display()
        );
    }
    bail!("recipe path not found: {}", path.display())
}

fn do_init(dir: &Path) -> Result<()> {
    let path = dir.join(forage_core::workspace::MANIFEST_NAME);
    if path.exists() {
        println!(
            "{} {} already exists; leaving untouched",
            "note:".yellow(),
            path.display()
        );
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let manifest = forage_core::workspace::Manifest::default();
    let body = forage_core::workspace::serialize_manifest(&manifest)
        .map_err(|e| anyhow::anyhow!("serialize manifest: {e}"))?;
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    println!("{} {}", "wrote".green(), path.display());
    Ok(())
}

async fn do_update(dir: &Path, hub_override: Option<String>) -> Result<()> {
    let ws = forage_core::workspace::load(dir).map_err(|e| anyhow::anyhow!("workspace: {e}"))?;
    let hub = resolve_hub(hub_override);
    if ws.manifest.deps.is_empty() {
        println!("{} [deps] is empty; nothing to do", "note:".dimmed());
        return Ok(());
    }
    let client = hub_client(&hub, None);
    let cache_root = hub_cache_root();

    let mut lock = forage_core::workspace::Lockfile::default();
    for (slug, &version) in &ws.manifest.deps {
        let (author, slug_only) = split_dep_slug(slug)?;
        let fetched = fetch_to_cache(&client, &cache_root, author, slug_only, version)
            .await
            .with_context(|| format!("fetching {slug}@{version}"))?;
        println!(
            "{} {slug}@{version} → {}",
            "fetched".green(),
            fetched.dir.display()
        );
        lock.deps.insert(
            slug.clone(),
            forage_core::workspace::LockedDep {
                version,
                hash: fetched.sha256,
            },
        );
    }

    let lock_body = forage_core::workspace::serialize_lockfile(&lock)
        .map_err(|e| anyhow::anyhow!("serialize lockfile: {e}"))?;
    let lock_path = ws.root.join(forage_core::workspace::LOCKFILE_NAME);
    std::fs::write(&lock_path, lock_body)?;
    println!("{} {}", "wrote".green(), lock_path.display());
    Ok(())
}

async fn do_publish(
    dir: &Path,
    hub_override: Option<String>,
    really_publish: bool,
    token_override: Option<String>,
) -> Result<()> {
    let ws = forage_core::workspace::load(dir).map_err(|e| anyhow::anyhow!("workspace: {e}"))?;
    let Some(name) = ws.manifest.name.clone() else {
        bail!(
            "{} requires `name = \"<author>/<slug>\"` in forage.toml",
            ws.root
                .join(forage_core::workspace::MANIFEST_NAME)
                .display()
        );
    };
    let (author, slug) = split_dep_slug(&name)?;
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
        let preview =
            forage_hub::assemble_publish_request(&ws.root, slug, description, category, tags)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        let bytes = preview.recipe.len()
            + preview.decls.iter().map(|d| d.source.len()).sum::<usize>()
            + preview.fixtures.iter().map(|f| f.content.len()).sum::<usize>();
        println!(
            "{} would POST atomic artifact ({bytes} bytes total) to {hub}/v1/packages/{author}/{slug}/versions",
            "dry-run:".yellow(),
        );
        println!(
            "    · recipe.forage ({} bytes)",
            preview.recipe.len()
        );
        for f in &preview.decls {
            println!("    · {} ({} bytes)", f.name, f.source.len());
        }
        for f in &preview.fixtures {
            println!("    · {} ({} bytes)", f.name, f.content.len());
        }
        println!(
            "    · base_version: {}",
            preview
                .base_version
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| "(first publish)".into())
        );
        println!("Re-run with --publish to actually POST.");
        return Ok(());
    }

    let client = hub_client(&hub, token_override);
    match publish_from_workspace(&client, &ws.root, author, slug, description, category, tags).await
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
        outcome.recipe_dir.display()
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
        outcome.recipe_dir.display()
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
