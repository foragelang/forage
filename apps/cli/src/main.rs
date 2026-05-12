//! `forage` — the command-line tool.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use indexmap::IndexMap;
use owo_colors::OwoColorize;

use forage_core::ast::JSONValue;
use forage_core::{EvalValue, Snapshot, parse, validate};
use forage_http::{Engine, LiveTransport, ReplayTransport};
use forage_hub::{AuthStore, AuthTokens, HubClient, RecipeMeta, device::run_device_flow};
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
    Run {
        recipe_dir: PathBuf,
        /// Replay against `fixtures/captures.jsonl` instead of hitting the network.
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
    /// Launch a webview and record fetch/XHR exchanges to JSONL. (R5.)
    Capture,
    /// Build a starter .forage recipe from a captures JSONL file. (R5.)
    Scaffold,
    /// Push a recipe to the Forage hub.
    Publish {
        recipe_dir: PathBuf,
        /// Hub URL (default: $FORAGE_HUB_URL or https://api.foragelang.com).
        #[arg(long)]
        hub: Option<String>,
        /// Actually POST instead of dry-run.
        #[arg(long)]
        publish: bool,
        /// Bearer token override (default: $FORAGE_HUB_TOKEN or auth store).
        #[arg(long, env = "FORAGE_HUB_TOKEN")]
        token: Option<String>,
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
            recipe_dir,
            replay,
            output,
        } => rt.block_on(run(&recipe_dir, replay, output)),
        Command::Test { recipe_dir, update } => rt.block_on(test(&recipe_dir, update)),
        Command::Capture => {
            println!("`forage capture` lands in R5.");
            Ok(())
        }
        Command::Scaffold => {
            println!("`forage scaffold` lands in R5.");
            Ok(())
        }
        Command::Publish {
            recipe_dir,
            hub,
            publish,
            token,
        } => rt.block_on(do_publish(&recipe_dir, hub, publish, token)),
        Command::Auth { action } => rt.block_on(do_auth(action)),
        Command::Lsp => {
            rt.block_on(forage_lsp::server::run_stdio());
            Ok(())
        }
    }
}

async fn run(recipe_dir: &Path, replay: bool, output: OutputFormat) -> Result<()> {
    let recipe = load_recipe(recipe_dir)?;
    let inputs = load_inputs(recipe_dir)?;
    let secrets = load_secrets_from_env(&recipe);

    let snapshot = if replay {
        let captures = load_captures(recipe_dir).context("loading fixtures/captures.jsonl")?;
        let transport = ReplayTransport::new(captures);
        let engine = Engine::new(&transport);
        engine
            .run(&recipe, inputs, secrets)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
    } else {
        let transport = LiveTransport::new().map_err(|e| anyhow::anyhow!("{e}"))?;
        let engine = Engine::new(&transport);
        engine
            .run(&recipe, inputs, secrets)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
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
    let recipe = load_recipe(recipe_dir)?;
    let inputs = load_inputs(recipe_dir)?;
    let secrets = load_secrets_from_env(&recipe);
    let captures = load_captures(recipe_dir)?;
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

fn load_recipe(dir: &Path) -> Result<forage_core::Recipe> {
    let path = dir.join("recipe.forage");
    let source =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let recipe = parse(&source).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    let report = validate(&recipe);
    if report.has_errors() {
        for e in report.errors() {
            eprintln!("{} {}", "validate:".red(), e.message);
        }
        bail!("recipe failed validation");
    }
    Ok(recipe)
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

fn load_secrets_from_env(recipe: &forage_core::Recipe) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}

fn load_captures(dir: &Path) -> Result<Vec<Capture>> {
    let path = dir.join("fixtures").join("captures.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let c: Capture =
            serde_json::from_str(line).with_context(|| format!("parsing capture: {line}"))?;
        out.push(c);
    }
    Ok(out)
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
            println!("{} {r}", "stall:".yellow());
        }
        for e in &d.unmet_expectations {
            println!("{} {e}", "expect:".red());
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

async fn do_publish(
    recipe_dir: &Path,
    hub_override: Option<String>,
    really_publish: bool,
    token_override: Option<String>,
) -> Result<()> {
    let recipe = load_recipe(recipe_dir)?;
    let hub = hub_override
        .or_else(|| std::env::var("FORAGE_HUB_URL").ok())
        .unwrap_or_else(|| "https://api.foragelang.com".into());

    // Bearer source: CLI arg > $FORAGE_HUB_TOKEN > auth store.
    let host = host_of(&hub);
    let token = token_override.or_else(|| {
        AuthStore::new()
            .read(&host)
            .ok()
            .flatten()
            .map(|t| t.access_token)
    });

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
    let source = std::fs::read_to_string(recipe_dir.join("recipe.forage"))?;

    if !really_publish {
        println!(
            "{} would POST {} bytes to {}/v1/recipes/{}",
            "dry-run:".yellow(),
            source.len(),
            hub,
            recipe.name
        );
        if let Some(t) = &token {
            println!("{} (token: {}…)", "auth:".dimmed(), &t[..t.len().min(8)]);
        } else {
            println!("{} no token; live publish would 401", "auth:".yellow());
        }
        println!("Re-run with --publish to actually POST.");
        return Ok(());
    }

    let mut client = HubClient::new(&hub);
    if let Some(t) = token {
        client = client.with_token(t);
    }
    let resp = client
        .publish(&recipe.name, &source, &meta)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "{} {} v{} (sha256 {})",
        "published".green(),
        resp.slug,
        resp.version,
        resp.sha256.as_deref().unwrap_or("?")
    );
    Ok(())
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
