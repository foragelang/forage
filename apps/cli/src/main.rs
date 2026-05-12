//! `forage` — the command-line tool.
//!
//! Subcommand surface is filled in across R3 (run/test), R5 (capture/scaffold),
//! R6 (publish/auth), R7 (lsp).

use clap::{Parser, Subcommand};

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
    Run,
    /// Run a recipe against fixtures and diff against an expected snapshot.
    Test,
    /// Launch a webview and record fetch/XHR exchanges to JSONL.
    Capture,
    /// Build a starter .forage recipe from a captures JSONL file.
    Scaffold,
    /// Push a recipe to the Forage hub.
    Publish,
    /// Sign in to the Forage hub via GitHub.
    Auth,
    /// Start the Forage Language Server (stdio or WebSocket).
    Lsp,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "forage=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Run => println!("`forage run` lands in R3."),
        Command::Test => println!("`forage test` lands in R3."),
        Command::Capture => println!("`forage capture` lands in R5."),
        Command::Scaffold => println!("`forage scaffold` lands in R5."),
        Command::Publish => println!("`forage publish` lands in R6."),
        Command::Auth => println!("`forage auth` lands in R6."),
        Command::Lsp => println!("`forage lsp` lands in R7."),
    }
    Ok(())
}
