//  Project:      git-scrub
//  File:         src/cli/spill.rs
//  Purpose:      `spill` use-case — operator-supplied path / text / secret patterns.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `spill` subcommand family — incident-response history scrubbing.
//!
//! Three sub-modes:
//!
//! - **`spill paths <pattern>...`** — operator-supplied glob patterns; drop
//!   matching file ops from history (same `FileMatcher` engine as `ai files`).
//!   **v1: fully wired.**
//! - **`spill text <file>`** — operator-supplied YAML of byte-level
//!   replacements; rewrite blob contents anywhere they match.
//!   **v1: scaffolded only (CLI accepts args; not part of v1 release surface).**
//! - **`spill secrets <file>`** — same engine as `text`, but the runbook
//!   gains a credential-rotation checklist because anything that leaked
//!   must be considered compromised.
//!   **v1: scaffolded only (CLI accepts args; not part of v1 release surface).**

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use tracing::{info, warn};

use crate::engine::{self, EngineStats, backup};
use crate::patterns::{
    BlobRewriter, FileMatcher, FileMatcherOptions, blob as blob_patterns, files as file_patterns,
};
use crate::{gh, plan, preflight, runbook, scan, verify};

/// Args for `spill <mode>`.
#[derive(Debug, Args)]
pub struct SpillArgs {
    /// What flavour of spill scrub to run.
    #[command(subcommand)]
    pub mode: SpillMode,
}

/// Modes for spill scrub.
#[derive(Debug, Subcommand)]
pub enum SpillMode {
    /// Drop file operations whose path matches one of the supplied globs.
    Paths(PathsArgs),
    /// Replace byte sequences inside every blob in history (from a YAML pattern file).
    Text(TextArgs),
    /// Same engine as `text`, plus a credential-rotation checklist in the runbook.
    Secrets(TextArgs),
}

/// Args for `spill paths`.
#[derive(Debug, Args)]
pub struct PathsArgs {
    /// Glob patterns to purge (repeatable, at least one required).
    #[arg(required = true, value_name = "GLOB")]
    pub patterns: Vec<String>,

    /// Common scrub options (execute, backup, report).
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Args for `spill text` and `spill secrets`.
#[derive(Debug, Args)]
pub struct TextArgs {
    /// YAML pattern file describing the replacements to apply.
    #[arg(value_name = "PATTERN_FILE")]
    pub pattern_file: PathBuf,

    /// Common scrub options.
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Shared options for every spill mode.
// CommonArgs is a CLI argument struct; every bool represents a distinct flag.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Actually rewrite history. Without this, only the plan is emitted.
    #[arg(long)]
    pub execute: bool,

    /// Audit mode (no rewrites; equivalent to omitting --execute).
    /// Provided for clarity in scripted use.
    #[arg(long, conflicts_with = "execute")]
    pub audit: bool,

    /// Where to write the mirror-clone backup. Defaults to platform cache dir.
    #[arg(long, value_name = "PATH")]
    pub backup_to: Option<PathBuf>,

    /// Skip the backup. Requires the longer flag.
    #[arg(long, requires = "really_no_backup_i_mean_it")]
    pub no_backup: bool,

    /// Confirmation flag for `--no-backup`.
    #[arg(long)]
    pub really_no_backup_i_mean_it: bool,

    /// Where to write the runbook markdown file. Defaults to platform cache dir.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
}

/// Entry-point invoked from `cli::Command::Spill`.
pub fn run(args: &SpillArgs, repo: Option<&Path>) -> Result<()> {
    match &args.mode {
        SpillMode::Paths(a) => run_paths(repo, a),
        SpillMode::Text(a) => run_blob(repo, a, BlobKind::Text),
        SpillMode::Secrets(a) => run_blob(repo, a, BlobKind::Secrets),
    }
}

#[derive(Debug, Clone, Copy)]
enum BlobKind {
    Text,
    Secrets,
}

impl BlobKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Text => "spill text",
            Self::Secrets => "spill secrets",
        }
    }
    const fn is_secrets(self) -> bool {
        matches!(self, Self::Secrets)
    }
}

fn run_paths(repo: Option<&Path>, args: &PathsArgs) -> Result<()> {
    let repo_dir = resolve_repo_dir(repo)?;
    info!(repo = %repo_dir.display(), pass = "spill paths", execute = args.common.execute, "starting");
    let preflight_report = preflight::run(&repo_dir).context("pre-flight checks")?;

    let cfg = file_patterns::FileConfig::default();
    let opts = FileMatcherOptions {
        include_extra: args.patterns.clone(),
        ..Default::default()
    };
    let matcher = FileMatcher::new(&cfg, &opts).context("compiling spill path patterns")?;
    if matcher.is_empty() {
        return Err(anyhow!("no path patterns supplied"));
    }

    let scan_stats =
        scan::scan(&repo_dir, None, Some(&matcher), None, None).context("dry-run scan")?;
    let plan_md = plan::render(&plan::PlanInputs {
        use_case: "spill paths",
        attribution_source: None,
        files_source: None,
        preflight: &preflight_report,
        stats: &scan_stats,
        dry_run: !args.common.execute,
    });
    println!("{plan_md}");

    let gh_ctx = gh::gather(&repo_dir);
    refuse_if_runs_active(&args.common, &gh_ctx)?;

    let backup_path = take_backup_if_requested(&args.common, &repo_dir)?;

    let exec_stats = if args.common.execute {
        let stats =
            engine::run(&repo_dir, None, Some(&matcher), None, None).context("rewrite engine")?;
        info!(?stats, "rewrite complete");
        let verify_stats =
            verify::run(&repo_dir, None, Some(&matcher), None, None).context("verification")?;
        info!(?verify_stats, "verification passed");
        Some(stats)
    } else {
        None
    };

    write_runbook(
        &args.common,
        &repo_dir,
        backup_path.as_deref(),
        &plan_md,
        &gh_ctx,
        /*secrets_section=*/ false,
    )?;

    warn_if_noop(exec_stats);
    Ok(())
}

fn run_blob(repo: Option<&Path>, args: &TextArgs, kind: BlobKind) -> Result<()> {
    let repo_dir = resolve_repo_dir(repo)?;
    info!(repo = %repo_dir.display(), pass = kind.label(), execute = args.common.execute, "starting");
    let preflight_report = preflight::run(&repo_dir).context("pre-flight checks")?;

    let yaml = std::fs::read_to_string(&args.pattern_file)
        .with_context(|| format!("reading pattern file {}", args.pattern_file.display()))?;
    let cfg = blob_patterns::parse_yaml(&yaml).context("parsing blob pattern YAML")?;
    let rewriter = BlobRewriter::new(&cfg).context("compiling blob patterns")?;
    if rewriter.is_empty() {
        return Err(anyhow!("pattern file declares no replacements"));
    }

    let scan_stats =
        scan::scan(&repo_dir, None, None, Some(&rewriter), None).context("dry-run scan")?;
    let plan_md = plan::render(&plan::PlanInputs {
        use_case: kind.label(),
        attribution_source: None,
        files_source: None,
        preflight: &preflight_report,
        stats: &scan_stats,
        dry_run: !args.common.execute,
    });
    println!("{plan_md}");

    let gh_ctx = gh::gather(&repo_dir);
    refuse_if_runs_active(&args.common, &gh_ctx)?;

    let backup_path = take_backup_if_requested(&args.common, &repo_dir)?;

    let exec_stats = if args.common.execute {
        let stats =
            engine::run(&repo_dir, None, None, Some(&rewriter), None).context("rewrite engine")?;
        info!(?stats, "rewrite complete");
        let verify_stats =
            verify::run(&repo_dir, None, None, Some(&rewriter), None).context("verification")?;
        info!(?verify_stats, "verification passed");
        Some(stats)
    } else {
        None
    };

    write_runbook(
        &args.common,
        &repo_dir,
        backup_path.as_deref(),
        &plan_md,
        &gh_ctx,
        kind.is_secrets(),
    )?;

    warn_if_noop(exec_stats);
    Ok(())
}

fn resolve_repo_dir(repo: Option<&Path>) -> Result<PathBuf> {
    repo.map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine repository directory"))
}

fn refuse_if_runs_active(common: &CommonArgs, gh_ctx: &gh::GhContext) -> Result<()> {
    if !common.execute || gh_ctx.in_flight_runs.is_empty() {
        return Ok(());
    }
    let summary: String = gh_ctx
        .in_flight_runs
        .iter()
        .map(|r| {
            format!(
                "  - {} (status: {}, branch: {}, {})",
                r.workflow_name, r.status, r.head_branch, r.url
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Err(anyhow!(
        "refusing to rewrite — {} GitHub Actions workflow run(s) currently in flight:\n{}\n\nWait for them to finish (or cancel) and retry.",
        gh_ctx.in_flight_runs.len(),
        summary,
    ))
}

fn take_backup_if_requested(common: &CommonArgs, repo_dir: &Path) -> Result<Option<PathBuf>> {
    if !common.execute || common.no_backup {
        return Ok(None);
    }
    let dest = resolve_backup_dest(common.backup_to.as_deref(), repo_dir)?;
    info!(dest = %dest.display(), "creating mirror-clone backup");
    backup::mirror_clone(repo_dir, &dest).context("mirror-clone backup")?;
    Ok(Some(dest))
}

fn write_runbook(
    common: &CommonArgs,
    repo_dir: &Path,
    backup_path: Option<&Path>,
    plan_md: &str,
    gh_ctx: &gh::GhContext,
    secrets_section: bool,
) -> Result<()> {
    if !common.execute && common.report.is_none() {
        return Ok(());
    }
    let runbook_md = runbook::render(&runbook::RunbookInputs {
        repo_dir,
        backup_path,
        plan_markdown: plan_md,
        dry_run: !common.execute,
        gh: Some(gh_ctx),
        spill_secrets: secrets_section,
        supply: None,
    });
    let report_path = resolve_report_path(common.report.as_deref(), repo_dir)?;
    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating runbook directory {}", parent.display()))?;
    }
    std::fs::write(&report_path, runbook_md)
        .with_context(|| format!("writing runbook to {}", report_path.display()))?;
    info!(path = %report_path.display(), "runbook written");
    Ok(())
}

fn warn_if_noop(stats: Option<EngineStats>) {
    let Some(s) = stats else { return };
    if s.commits_rewritten == 0 && s.file_ops_dropped == 0 && s.blobs_rewritten == 0 {
        warn!("rewrite engine made no changes (nothing matched)");
    }
}

fn resolve_backup_dest(explicit: Option<&Path>, repo_dir: &Path) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let cache = crate::patterns::discovery::cache_dir()
        .ok_or_else(|| anyhow!("cannot resolve platform cache directory"))?;
    let repo_name = repo_dir
        .file_name()
        .map_or_else(|| "repo".to_string(), |s| s.to_string_lossy().into_owned());
    let ts = timestamp_compact();
    Ok(cache.join("backups").join(format!("{repo_name}-{ts}")))
}

fn resolve_report_path(explicit: Option<&Path>, repo_dir: &Path) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let cache = crate::patterns::discovery::cache_dir()
        .ok_or_else(|| anyhow!("cannot resolve platform cache directory"))?;
    let repo_name = repo_dir
        .file_name()
        .map_or_else(|| "repo".to_string(), |s| s.to_string_lossy().into_owned());
    let ts = timestamp_compact();
    Ok(cache.join("runbooks").join(format!("{repo_name}-{ts}.md")))
}

fn timestamp_compact() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{secs}")
}
