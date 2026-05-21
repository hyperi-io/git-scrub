//  Project:      git-scrub
//  File:         src/cli/clean.rs
//  Purpose:      `clean` umbrella subcommand — single-pass ai+spill+supply.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `clean` umbrella subcommand.
//!
//! Activates any combination of AI scrub, spill scrub (paths only for v1),
//! and supply chain scrub transformers in a single engine pass. One
//! backup, one verify, one runbook.
//!
//! When both `--ai` and `--spill-paths` are active the two file-matcher sets
//! are merged into a single [`FileMatcher`]: the AI tool [`FileConfig`] is
//! retained (so default AI file patterns still apply) and the spill-path globs
//! are appended to `include_extra`. This gives the single engine pass the union
//! of both sets without any extra overhead.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use tracing::{info, warn};

use crate::engine::{self, EngineStats, backup};
use crate::patterns::lockfile::{
    BunLockRewriter, CargoLockRewriter, ComposerLockRewriter, GoSumRewriter, NpmLockRewriter,
    PipLockRewriter, PnpmLockRewriter, YarnLockRewriter,
};
use crate::patterns::supply::{CompromisedPackage, PurgeTarget, SupplyConfig};
use crate::patterns::{
    AttributionRewriter, FileConfig, FileMatcher, FileMatcherOptions, LockfileRewriter,
    attribution, discovery, files,
};
use crate::{gh, plan, preflight, runbook, scan, verify};

/// Args for `git-scrub clean`.
// CleanArgs has five boolean fields because it is a CLI argument struct where
// every bool represents a distinct flag. The lint fires here by design.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Activate AI scrub (attribution trailers + artefact files).
    #[arg(long)]
    pub ai: bool,

    /// Activate spill-paths transformer with the given globs (repeatable).
    #[arg(long = "spill-paths", value_name = "GLOB")]
    pub spill_paths: Vec<String>,

    /// Activate supply chain transformers (lockfile entries).
    #[arg(long)]
    pub supply: bool,

    /// Actually rewrite history. Without this flag, only the plan is emitted.
    #[arg(long)]
    pub execute: bool,

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

    /// Override path to the AI attribution YAML.
    #[arg(long, value_name = "PATH")]
    pub attribution_config: Option<PathBuf>,

    /// Override path to the AI files YAML.
    #[arg(long, value_name = "PATH")]
    pub files_config: Option<PathBuf>,

    /// Override path to the supply-chain advisory YAML.
    #[arg(long, value_name = "PATH")]
    pub supply_config: Option<PathBuf>,

    /// Tool names to skip entirely (repeatable). E.g. `--exclude-tool cursor`.
    #[arg(long = "exclude-tool", value_name = "NAME")]
    pub exclude_tools: Vec<String>,

    /// Ecosystems to skip entirely (repeatable). E.g. `--exclude-ecosystem pip`.
    #[arg(long = "exclude-ecosystem", value_name = "NAME")]
    pub exclude_ecosystems: Vec<String>,
}

/// Entry-point invoked from `cli::Command::Clean`.
///
/// # Errors
///
/// Returns an error if no transformers are activated, if any pattern file
/// fails to load or parse, or if the rewrite engine encounters a problem.
pub fn run(args: &CleanArgs, repo: Option<&Path>) -> Result<()> {
    if !args.ai && args.spill_paths.is_empty() && !args.supply {
        anyhow::bail!(
            "no transformers activated — pass at least one of: \
             --ai, --spill-paths <GLOB>, --supply"
        );
    }

    let repo_dir = resolve_repo_dir(repo)?;
    info!(
        repo = %repo_dir.display(),
        ai = args.ai,
        spill_paths = args.spill_paths.len(),
        supply = args.supply,
        execute = args.execute,
        "starting clean umbrella pass",
    );

    let preflight_report = preflight::run(&repo_dir).context("pre-flight checks")?;

    // ── AI attribution rewriter ───────────────────────────────────────────────
    let attribution_rewriter: Option<AttributionRewriter> = if args.ai {
        let loaded = discovery::load(
            discovery::ATTRIBUTION_FILE,
            discovery::EMBEDDED_ATTRIBUTION,
            args.attribution_config.as_deref(),
        )
        .context("loading attribution pattern file")?;
        let cfg = attribution::parse_yaml(&loaded.content).context("parsing attribution YAML")?;
        Some(AttributionRewriter::new(&cfg, &args.exclude_tools)?)
    } else {
        None
    };

    // ── File matcher: merge AI files + spill-paths into one pass ─────────────
    //
    // Strategy: use the AI FileConfig as the base (so default AI file patterns
    // stay active when --ai is set) and fold the spill-path globs into
    // `include_extra`. When only --spill-paths is active, start from an empty
    // FileConfig so no AI tool patterns are added accidentally.
    let file_matcher: Option<FileMatcher> = build_file_matcher(args)?;

    // ── Supply lockfile rewriters ─────────────────────────────────────────────
    let lockfile_rewriters: Vec<Box<dyn LockfileRewriter>> = if args.supply {
        build_lockfile_rewriters(args.supply_config.as_deref(), &args.exclude_ecosystems)
            .context("building supply lockfile rewriters")?
    } else {
        Vec::new()
    };

    if args.supply && lockfile_rewriters.is_empty() {
        warn!(
            "no supply-chain rewriters active (empty advisory snapshot, \
             all ecosystems excluded, or all matched ecosystems unimplemented)"
        );
    }

    let lockfiles_opt: Option<&[Box<dyn LockfileRewriter>]> = if lockfile_rewriters.is_empty() {
        None
    } else {
        Some(&lockfile_rewriters)
    };

    // ── Dry-run scan → plan ──────────────────────────────────────────────────
    let scan_stats = scan::scan(
        &repo_dir,
        attribution_rewriter.as_ref(),
        file_matcher.as_ref(),
        None,
        lockfiles_opt,
    )
    .context("dry-run scan")?;

    let plan_md = plan::render(&plan::PlanInputs {
        use_case: "clean",
        attribution_source: None,
        files_source: None,
        preflight: &preflight_report,
        stats: &scan_stats,
        dry_run: !args.execute,
    });
    println!("{plan_md}");

    let gh_ctx = gh::gather(&repo_dir);
    refuse_if_runs_active(args, &gh_ctx)?;

    let backup_path = take_backup_if_requested(args, &repo_dir)?;

    // ── Execute ───────────────────────────────────────────────────────────────
    let exec_stats: Option<EngineStats> = if args.execute {
        let stats = engine::run(
            &repo_dir,
            attribution_rewriter.as_ref(),
            file_matcher.as_ref(),
            None,
            lockfiles_opt,
        )
        .context("rewrite engine")?;
        info!(?stats, "rewrite complete");

        verify::run(
            &repo_dir,
            attribution_rewriter.as_ref(),
            file_matcher.as_ref(),
            None,
            lockfiles_opt,
        )
        .context("verification")?;
        info!("verification passed");

        Some(stats)
    } else {
        None
    };

    write_runbook(
        args,
        &repo_dir,
        backup_path.as_deref(),
        &plan_md,
        &gh_ctx,
        exec_stats.as_ref(),
    )?;
    warn_if_noop(exec_stats);
    Ok(())
}

/// Build a combined [`FileMatcher`] covering both AI file patterns and
/// operator-supplied spill-path globs.
///
/// - `--ai` only → AI `FileConfig` + AI-focused `FileMatcherOptions`
/// - `--spill-paths` only → empty `FileConfig` with spill globs in `include_extra`
/// - Both → AI `FileConfig` + `include_extra` = spill globs (union of both sets)
/// - Neither → `Ok(None)`
fn build_file_matcher(args: &CleanArgs) -> Result<Option<FileMatcher>> {
    if !args.ai && args.spill_paths.is_empty() {
        return Ok(None);
    }

    // Load the AI file config only when --ai is active; otherwise use a blank config
    // so no AI tool patterns are injected into a spill-only run.
    let file_cfg: FileConfig = if args.ai {
        let loaded = discovery::load(
            discovery::FILES_FILE,
            discovery::EMBEDDED_FILES,
            args.files_config.as_deref(),
        )
        .context("loading files pattern file")?;
        files::parse_yaml(&loaded.content).context("parsing files YAML")?
    } else {
        FileConfig::default()
    };

    let opts = FileMatcherOptions {
        exclude_tools: args.exclude_tools.clone(),
        // Spill-path globs are appended on top of whatever the AI config contributes.
        include_extra: args.spill_paths.clone(),
        ..Default::default()
    };

    Ok(Some(
        FileMatcher::new(&file_cfg, &opts).context("compiling file patterns")?,
    ))
}

/// Build the supply-chain lockfile rewriter set from the advisory YAML.
fn build_lockfile_rewriters(
    explicit_config: Option<&Path>,
    exclude_ecosystems: &[String],
) -> Result<Vec<Box<dyn LockfileRewriter>>> {
    use std::collections::BTreeMap;

    let loaded = discovery::load(
        discovery::SUPPLY_CHAIN_FILE,
        discovery::EMBEDDED_SUPPLY_CHAIN,
        explicit_config,
    )
    .context("loading supply-chain pattern file")?;
    let cfg: SupplyConfig =
        serde_yaml_ng::from_str(&loaded.content).context("parsing supply-chain.yaml")?;

    let mut by_eco: BTreeMap<&str, Vec<&CompromisedPackage>> = BTreeMap::new();
    for p in &cfg.compromised {
        if exclude_ecosystems.iter().any(|x| x == &p.ecosystem) {
            continue;
        }
        if !p.purge_targets.contains(&PurgeTarget::LockfileEntry) {
            continue;
        }
        by_eco.entry(p.ecosystem.as_str()).or_default().push(p);
    }

    let mut rewriters: Vec<Box<dyn LockfileRewriter>> = Vec::new();

    // CargoLockRewriter handles cargo, uv, and poetry (TOML [[package]] array shape).
    let mut toml_array_pkgs: Vec<&CompromisedPackage> = Vec::new();
    for eco in ["cargo", "uv", "poetry"] {
        if let Some(v) = by_eco.get(eco) {
            toml_array_pkgs.extend(v.iter().copied());
        }
    }
    if !toml_array_pkgs.is_empty() {
        rewriters.push(Box::new(CargoLockRewriter::new(&toml_array_pkgs)?));
    }
    if let Some(npm_pkgs) = by_eco.get("npm") {
        rewriters.push(Box::new(NpmLockRewriter::new(npm_pkgs)?));
    }
    if let Some(pnpm_pkgs) = by_eco.get("pnpm") {
        rewriters.push(Box::new(PnpmLockRewriter::new(pnpm_pkgs)?));
    }
    if let Some(yarn_pkgs) = by_eco.get("yarn") {
        rewriters.push(Box::new(YarnLockRewriter::new(yarn_pkgs)?));
    }
    if let Some(bun_pkgs) = by_eco.get("bun") {
        rewriters.push(Box::new(BunLockRewriter::new(bun_pkgs)?));
    }
    if let Some(pip_pkgs) = by_eco.get("pip") {
        rewriters.push(Box::new(PipLockRewriter::new(pip_pkgs)?));
    }
    if let Some(go_pkgs) = by_eco.get("go") {
        rewriters.push(Box::new(GoSumRewriter::new(go_pkgs)?));
    }
    if let Some(composer_pkgs) = by_eco.get("composer") {
        rewriters.push(Box::new(ComposerLockRewriter::new(composer_pkgs)?));
    }

    Ok(rewriters)
}

// ── Helper functions ──────────────────────────────────────────────────────────

fn resolve_repo_dir(repo: Option<&Path>) -> Result<PathBuf> {
    repo.map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine repository directory"))
}

fn refuse_if_runs_active(args: &CleanArgs, gh_ctx: &gh::GhContext) -> Result<()> {
    if !args.execute || gh_ctx.in_flight_runs.is_empty() {
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

fn take_backup_if_requested(args: &CleanArgs, repo_dir: &Path) -> Result<Option<PathBuf>> {
    if !args.execute || args.no_backup {
        return Ok(None);
    }
    let dest = resolve_backup_dest(args.backup_to.as_deref(), repo_dir)?;
    info!(dest = %dest.display(), "creating mirror-clone backup");
    backup::mirror_clone(repo_dir, &dest).context("mirror-clone backup")?;
    Ok(Some(dest))
}

fn write_runbook(
    args: &CleanArgs,
    repo_dir: &Path,
    backup_path: Option<&Path>,
    plan_md: &str,
    gh_ctx: &gh::GhContext,
    exec_stats: Option<&EngineStats>,
) -> Result<()> {
    if !args.execute && args.report.is_none() {
        return Ok(());
    }
    // Only populate the supply summary when --supply was active and the pass ran.
    let supply_summary = if args.supply {
        exec_stats.map(|s| runbook::SupplySummary {
            lockfile_blobs_rewritten: s.blobs_rewritten,
        })
    } else {
        None
    };
    let runbook_md = runbook::render(&runbook::RunbookInputs {
        repo_dir,
        backup_path,
        plan_markdown: plan_md,
        dry_run: !args.execute,
        gh: Some(gh_ctx),
        spill_secrets: false,
        supply: supply_summary,
    });
    let report_path = resolve_report_path(args.report.as_deref(), repo_dir)?;
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
    let cache =
        discovery::cache_dir().ok_or_else(|| anyhow!("cannot resolve platform cache directory"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `CleanArgs` with sensible defaults for unit testing.
    fn base_args() -> CleanArgs {
        CleanArgs {
            ai: false,
            spill_paths: vec![],
            supply: false,
            execute: false,
            backup_to: None,
            no_backup: false,
            really_no_backup_i_mean_it: false,
            report: None,
            attribution_config: None,
            files_config: None,
            supply_config: None,
            exclude_tools: vec![],
            exclude_ecosystems: vec![],
        }
    }

    #[test]
    fn no_transformers_returns_error() {
        let args = base_args();
        // resolve_repo_dir doesn't matter here — we bail before touching the repo.
        let result = run(&args, None);
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("no transformers activated"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn spill_only_builds_non_ai_file_matcher() {
        let mut args = base_args();
        args.spill_paths = vec!["secrets/**".to_string()];
        // build_file_matcher should succeed and return Some matcher
        let matcher = build_file_matcher(&args).expect("build_file_matcher");
        let m = matcher.expect("expected Some(FileMatcher)");
        // The glob we passed should match a path under secrets/
        assert!(m.should_purge("secrets/api_key.txt"));
        // A random src path should not be purged
        assert!(!m.should_purge("src/main.rs"));
    }

    #[test]
    fn ai_and_spill_merges_patterns() {
        // When both --ai and --spill-paths are set, spill globs land in include_extra
        // alongside AI file patterns. The resulting FileMatcher covers both.
        let mut args = base_args();
        args.ai = true;
        args.spill_paths = vec!["dump/**".to_string()];
        // Uses embedded config; should not error
        let matcher = build_file_matcher(&args).expect("build_file_matcher");
        let m = matcher.expect("expected Some(FileMatcher)");
        // Spill glob must match
        assert!(m.should_purge("dump/heap.bin"));
    }

    #[test]
    fn neither_ai_nor_spill_returns_none_matcher() {
        let args = base_args();
        let result = build_file_matcher(&args).expect("should not error");
        assert!(result.is_none(), "expected None when no file flags set");
    }
}
