//  Project:      git-scrub
//  File:         src/cli/ai.rs
//  Purpose:      `ai` use-case command implementation (attribution + files + composite).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `ai` subcommand family.

use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use tracing::{info, warn};

use crate::engine::{self, EngineStats, backup};
use crate::patterns::{
    AttributionRewriter, FileMatcher, FileMatcherOptions, attribution, discovery, files,
};
use crate::{gh, plan, preflight, runbook, scan, verify};

/// Args for the `ai` subcommand and its children.
#[derive(Debug, Args)]
pub struct AiArgs {
    /// What flavour of AI scrub to run.
    #[command(subcommand)]
    pub mode: AiMode,
}

/// Modes for AI scrub.
#[derive(Debug, Subcommand)]
pub enum AiMode {
    /// Attribution trailers AND artefact files (default for `ai`).
    Composite(SharedArgs),
    /// Attribution trailers only — rewrite commit messages.
    Attribution(SharedArgs),
    /// Artefact files only — drop file operations on AI tool paths.
    Files(SharedArgs),
    /// Dump the active pattern library (read-only, no rewrite).
    Patterns(PatternsArgs),
}

/// Args shared by `attribution`, `files`, and `composite`.
#[derive(Debug, Args)]
pub struct SharedArgs {
    /// Actually rewrite history. Without this flag, we only emit the plan.
    #[arg(long)]
    pub execute: bool,

    /// Override path to the attribution YAML.
    #[arg(long, value_name = "PATH")]
    pub attribution_config: Option<PathBuf>,

    /// Override path to the files YAML.
    #[arg(long, value_name = "PATH")]
    pub files_config: Option<PathBuf>,

    /// Tool names to skip entirely (repeatable). E.g. `--exclude-tool cursor`.
    #[arg(long = "exclude-tool", value_name = "TOOL")]
    pub exclude_tools: Vec<String>,

    /// Extra paths/globs to add to the purge set (repeatable).
    #[arg(long = "include", value_name = "GLOB")]
    pub include_extra: Vec<String>,

    /// Extra paths/globs to subtract from the purge set (repeatable).
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude_extra: Vec<String>,

    /// Force-include a normally-protected state-file name (repeatable).
    /// E.g. `--force-include CLAUDE.md` to purge it despite the default protection.
    #[arg(long = "force-include", value_name = "NAME")]
    pub force_include: Vec<String>,

    /// Where to write the mirror-clone backup. Defaults to platform cache dir.
    #[arg(long, value_name = "PATH")]
    pub backup_to: Option<PathBuf>,

    /// Skip the backup. Requires the longer flag.
    #[arg(long, requires = "really_no_backup_i_mean_it")]
    pub no_backup: bool,

    /// Confirmation flag for `--no-backup`. Stops accidental skipping.
    #[arg(long)]
    pub really_no_backup_i_mean_it: bool,

    /// Where to write the runbook markdown file. Defaults to platform cache dir.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
}

/// Args for `ai patterns`.
#[derive(Debug, Args)]
pub struct PatternsArgs {
    /// Override path to the attribution YAML.
    #[arg(long, value_name = "PATH")]
    pub attribution_config: Option<PathBuf>,

    /// Override path to the files YAML.
    #[arg(long, value_name = "PATH")]
    pub files_config: Option<PathBuf>,
}

/// Entry-point invoked from `main.rs`.
pub fn run(args: &AiArgs, repo: Option<&std::path::Path>) -> Result<()> {
    match &args.mode {
        AiMode::Patterns(p) => run_patterns(p),
        AiMode::Attribution(s) => run_pass(repo, s, PassKind::Attribution),
        AiMode::Files(s) => run_pass(repo, s, PassKind::Files),
        AiMode::Composite(s) => run_pass(repo, s, PassKind::Composite),
    }
}

#[derive(Debug, Clone, Copy)]
enum PassKind {
    Attribution,
    Files,
    Composite,
}

impl PassKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Attribution => "ai attribution",
            Self::Files => "ai files",
            Self::Composite => "ai",
        }
    }
    const fn want_attribution(self) -> bool {
        matches!(self, Self::Attribution | Self::Composite)
    }
    const fn want_files(self) -> bool {
        matches!(self, Self::Files | Self::Composite)
    }
}

fn run_patterns(args: &PatternsArgs) -> Result<()> {
    let attr = discovery::load(
        discovery::ATTRIBUTION_FILE,
        discovery::EMBEDDED_ATTRIBUTION,
        args.attribution_config.as_deref(),
    )
    .context("loading attribution pattern file")?;
    let f = discovery::load(
        discovery::FILES_FILE,
        discovery::EMBEDDED_FILES,
        args.files_config.as_deref(),
    )
    .context("loading files pattern file")?;

    println!("# git-scrub: active pattern library\n");
    println!("## attribution ({})", attr.source.label());
    if let Some(p) = &attr.path {
        println!("source path: `{}`", p.display());
    }
    println!("\n```yaml\n{}\n```\n", attr.content);
    println!("## files ({})", f.source.label());
    if let Some(p) = &f.path {
        println!("source path: `{}`", p.display());
    }
    println!("\n```yaml\n{}\n```", f.content);
    Ok(())
}

fn run_pass(repo: Option<&std::path::Path>, args: &SharedArgs, kind: PassKind) -> Result<()> {
    let repo_dir = repo
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine repository directory"))?;

    info!(repo = %repo_dir.display(), pass = kind.label(), execute = args.execute, "starting");

    let preflight_report = preflight::run(&repo_dir).context("pre-flight checks")?;

    let attribution_loaded = if kind.want_attribution() {
        Some(
            discovery::load(
                discovery::ATTRIBUTION_FILE,
                discovery::EMBEDDED_ATTRIBUTION,
                args.attribution_config.as_deref(),
            )
            .context("loading attribution pattern file")?,
        )
    } else {
        None
    };
    let files_loaded = if kind.want_files() {
        Some(
            discovery::load(
                discovery::FILES_FILE,
                discovery::EMBEDDED_FILES,
                args.files_config.as_deref(),
            )
            .context("loading files pattern file")?,
        )
    } else {
        None
    };

    let attribution_rewriter = if let Some(loaded) = &attribution_loaded {
        let cfg = attribution::parse_yaml(&loaded.content).context("parsing attribution YAML")?;
        Some(AttributionRewriter::new(&cfg, &args.exclude_tools)?)
    } else {
        None
    };

    let file_matcher = if let Some(loaded) = &files_loaded {
        let cfg = files::parse_yaml(&loaded.content).context("parsing files YAML")?;
        let opts = FileMatcherOptions {
            exclude_tools: args.exclude_tools.clone(),
            include_extra: args.include_extra.clone(),
            exclude_extra: args.exclude_extra.clone(),
            force_include: args.force_include.clone(),
        };
        Some(FileMatcher::new(&cfg, &opts)?)
    } else {
        None
    };

    // Scan first to drive the plan markdown.
    let scan_stats = scan::scan(
        &repo_dir,
        attribution_rewriter.as_ref(),
        file_matcher.as_ref(),
        None,
        None,
    )
    .context("dry-run scan")?;

    let plan_md = plan::render(&plan::PlanInputs {
        use_case: kind.label(),
        attribution_source: attribution_loaded.as_ref().map(|l| l.source),
        files_source: files_loaded.as_ref().map(|l| l.source),
        preflight: &preflight_report,
        stats: &scan_stats,
        dry_run: !args.execute,
    });

    println!("{plan_md}");

    // Gather gh context once, up-front: we use it for the in-flight-runs
    // pre-check (refuses --execute) AND later for the runbook.
    let gh_ctx = gh::gather(&repo_dir);

    if args.execute && !gh_ctx.in_flight_runs.is_empty() {
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
        return Err(anyhow!(
            "refusing to rewrite — {} GitHub Actions workflow run(s) are \
             currently in flight on this repository:\n{}\n\nWait for them to \
             finish (or cancel them) and retry. Use `--no-backup \
             --really-no-backup-i-mean-it` is NOT a bypass for this check.",
            gh_ctx.in_flight_runs.len(),
            summary,
        ));
    }

    let backup_path = if args.execute && !args.no_backup {
        let dest = resolve_backup_dest(args.backup_to.as_deref(), &repo_dir)?;
        info!(dest = %dest.display(), "creating mirror-clone backup");
        backup::mirror_clone(&repo_dir, &dest).context("mirror-clone backup")?;
        Some(dest)
    } else {
        None
    };

    let post_stats: Option<EngineStats> = if args.execute {
        let stats = engine::run(
            &repo_dir,
            attribution_rewriter.as_ref(),
            file_matcher.as_ref(),
            None,
            None,
        )
        .context("rewrite engine run")?;
        info!(?stats, "rewrite complete");

        let verify_stats = verify::run(
            &repo_dir,
            attribution_rewriter.as_ref(),
            file_matcher.as_ref(),
            None,
            None,
        )
        .context("post-rewrite verification")?;
        info!(?verify_stats, "verification passed");
        Some(stats)
    } else {
        None
    };

    if args.execute || args.report.is_some() {
        let runbook_md = runbook::render(&runbook::RunbookInputs {
            repo_dir: &repo_dir,
            backup_path: backup_path.as_deref(),
            plan_markdown: &plan_md,
            dry_run: !args.execute,
            gh: Some(&gh_ctx),
            spill_secrets: false,
        });
        let report_path = resolve_report_path(args.report.as_deref(), &repo_dir)?;
        if let Some(parent) = report_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating runbook directory {}", parent.display()))?;
        }
        std::fs::write(&report_path, runbook_md)
            .with_context(|| format!("writing runbook to {}", report_path.display()))?;
        info!(path = %report_path.display(), "runbook written");
    }

    if let Some(stats) = post_stats
        && stats.commits_rewritten == 0
        && stats.file_ops_dropped == 0
    {
        warn!("rewrite engine made no changes (nothing matched)");
    }

    Ok(())
}

fn resolve_backup_dest(
    explicit: Option<&std::path::Path>,
    repo_dir: &std::path::Path,
) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let cache =
        discovery::cache_dir().ok_or_else(|| anyhow!("cannot resolve platform cache directory"))?;
    let repo_name = repo_dir
        .file_name()
        .map_or_else(|| "repo".to_string(), |s| s.to_string_lossy().into_owned());
    let ts = timestamp_compact();
    Ok(cache.join("backups").join(format!("{repo_name}-{ts}")))
}

fn resolve_report_path(
    explicit: Option<&std::path::Path>,
    repo_dir: &std::path::Path,
) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let cache =
        discovery::cache_dir().ok_or_else(|| anyhow!("cannot resolve platform cache directory"))?;
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
