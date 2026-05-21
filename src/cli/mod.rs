//  Project:      git-scrub
//  File:         src/cli/mod.rs
//  Purpose:      clap-based CLI definition and top-level dispatch.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Command-line interface.
//!
//! The binary is intended to be invokable as `git-scrub <use-case>` (custom
//! git subcommand discovery, when on `PATH`) AND as `git scrub <use-case>`
//! (git's `git-<name>` convention).

pub mod ai;
pub mod clean;
pub mod curate;
pub mod spill;
pub mod supply;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Top-level CLI parser.
#[derive(Debug, Parser)]
#[command(
    name = "git-scrub",
    version,
    about = "Surgical removal of unwanted content from git history",
    long_about = "git-scrub removes AI tool residue (attribution trailers + artefact \
                  files) and accidental data spill from a repository's entire history. \
                  Default mode is dry-run. Pass `--execute` to actually rewrite history."
)]
pub struct Cli {
    /// Path to the repository working tree (defaults to current directory).
    #[arg(short = 'C', long, global = true)]
    pub repo: Option<PathBuf>,

    /// Reduce output (errors only).
    #[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Increase output (debug logging).
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Subcommand selecting the use case.
    #[command(subcommand)]
    pub command: Command,
}

/// Use-case subcommands. `ai` is opinionated/v1; `spill` is operator-supplied/v2.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// AI scrub: attribution trailers and artefact files.
    Ai(ai::AiArgs),
    /// Spill scrub: operator-supplied path / text / secret patterns.
    Spill(spill::SpillArgs),
    /// Supply chain scrub: strip known-bad lockfile entries from history.
    Supply(supply::SupplyArgs),
    /// Umbrella: single-pass composition of ai + spill-paths + supply.
    Clean(clean::CleanArgs),
}

/// Run the parsed CLI to completion.
///
/// Initialises logging, then dispatches the subcommand to its implementation
/// module. Returns a single `anyhow::Result` so the binary can render errors
/// uniformly.
pub fn run(cli: &Cli) -> anyhow::Result<()> {
    init_logging(cli.quiet, cli.verbose);
    let repo = cli.repo.as_deref();
    match &cli.command {
        Command::Ai(args) => ai::run(args, repo),
        Command::Spill(args) => spill::run(args, repo),
        Command::Supply(args) => supply::run(args, repo),
        Command::Clean(args) => clean::run(args, repo),
    }
}

fn init_logging(quiet: bool, verbose: bool) {
    use std::io::IsTerminal;

    use tracing_subscriber::{EnvFilter, fmt};

    let level = if quiet {
        "warn"
    } else if verbose {
        "debug"
    } else {
        "info"
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("git_scrub={level},info")));

    let is_terminal = std::io::stderr().is_terminal();
    let builder = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);

    if is_terminal {
        let _ = builder.with_ansi(true).try_init();
    } else {
        let _ = builder.with_ansi(false).json().try_init();
    }
}
