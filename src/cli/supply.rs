//  Project:      git-scrub
//  File:         src/cli/supply.rs
//  Purpose:      `supply` subcommand — supply chain scrub.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `supply` subcommand family — supply chain scrub.
//!
//! Five sub-modes:
//!
//! - **`supply composite`** — lockfile entries + (eventually) vendored source + bundled advisories.
//! - **`supply lockfiles`** — lockfile entries only.
//! - **`supply packages <pkg>...`** — incident-specific list, `[<eco>:]<name>[@<ver>]` syntax.
//! - **`supply advisories`** — apply the full bundled advisory snapshot.
//! - **`supply patterns`** — dump the active pattern library (read-only).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use tracing::{info, warn};

use crate::engine::{self, EngineStats, backup};
use crate::patterns::lockfile::{CargoLockRewriter, NpmLockRewriter, PnpmLockRewriter};
use crate::patterns::supply::{CompromisedPackage, PurgeTarget, SupplyConfig};
use crate::patterns::{LockfileRewriter, discovery};
use crate::{gh, plan, preflight, runbook, scan, verify};

/// Args for `supply <mode>`.
#[derive(Debug, Args)]
pub struct SupplyArgs {
    /// What flavour of supply chain scrub to run.
    #[command(subcommand)]
    pub mode: SupplyMode,
}

/// Modes for supply chain scrub.
#[derive(Debug, Subcommand)]
pub enum SupplyMode {
    /// Composite: lockfile entries + vendored source + bundled advisories.
    Composite(CommonArgs),
    /// Lockfile entries only (no vendored purge).
    Lockfiles(CommonArgs),
    /// Strip specific packages by `[<eco>:]<name>[@<ver>]` syntax.
    Packages(PackagesArgs),
    /// Apply the full bundled advisory snapshot.
    Advisories(CommonArgs),
    /// Dump the active supply-chain pattern library (read-only).
    Patterns,
}

/// Args for `supply packages`.
#[derive(Debug, Args)]
pub struct PackagesArgs {
    /// One or more packages in `[<eco>:]<name>[@<ver>]` form.
    #[arg(required = true, value_name = "PACKAGE")]
    pub packages: Vec<String>,

    /// Common scrub options (execute, backup, report).
    #[command(flatten)]
    pub common: CommonArgs,
}

/// Shared options for every supply mode (except `patterns`).
#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Actually rewrite history. Without this, only the plan is emitted.
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

    /// Override path to the supply-chain advisory YAML.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Where to write the runbook markdown file. Defaults to platform cache dir.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,

    /// Ecosystems to skip entirely (repeatable). E.g. `--exclude-ecosystem pip`.
    #[arg(long = "exclude-ecosystem", value_name = "NAME")]
    pub exclude_ecosystem: Vec<String>,
}

/// Entry-point invoked from `cli::Command::Supply`.
pub fn run(args: &SupplyArgs, repo: Option<&Path>) -> Result<()> {
    match &args.mode {
        SupplyMode::Composite(c) | SupplyMode::Lockfiles(c) | SupplyMode::Advisories(c) => {
            run_pass(repo, c, None)
        }
        SupplyMode::Packages(p) => run_pass(repo, &p.common, Some(&p.packages)),
        SupplyMode::Patterns => run_patterns_dump(),
    }
}

fn run_patterns_dump() -> Result<()> {
    let loaded = discovery::load(
        discovery::SUPPLY_CHAIN_FILE,
        discovery::EMBEDDED_SUPPLY_CHAIN,
        None,
    )
    .context("loading supply-chain pattern file")?;
    println!("# git-scrub: active supply-chain pattern library\n");
    println!("source: {}", loaded.source.label());
    if let Some(p) = &loaded.path {
        println!("source path: `{}`", p.display());
    }
    println!("\n```yaml\n{}\n```", loaded.content);
    Ok(())
}

fn load_config(explicit: Option<&Path>) -> Result<SupplyConfig> {
    let loaded = discovery::load(
        discovery::SUPPLY_CHAIN_FILE,
        discovery::EMBEDDED_SUPPLY_CHAIN,
        explicit,
    )
    .context("loading supply-chain pattern file")?;
    serde_yaml_ng::from_str(&loaded.content).context("parsing supply-chain.yaml")
}

fn build_lockfile_rewriters(
    cfg: &SupplyConfig,
    exclude_ecosystems: &[String],
) -> Result<Vec<Box<dyn LockfileRewriter>>> {
    use std::collections::BTreeMap;

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

    // CargoLockRewriter covers cargo, uv, poetry (TOML [[package]] array shape).
    let mut toml_array_pkgs: Vec<&CompromisedPackage> = Vec::new();
    for eco in ["cargo", "uv", "poetry"] {
        if let Some(v) = by_eco.get(eco) {
            toml_array_pkgs.extend(v.iter().copied());
        }
    }
    if !toml_array_pkgs.is_empty() {
        rewriters.push(Box::new(CargoLockRewriter::new(&toml_array_pkgs)?));
    }

    // npm: package-lock.json + npm-shrinkwrap.json (lockfile v1, v2, v3).
    if let Some(npm_pkgs) = by_eco.get("npm") {
        rewriters.push(Box::new(NpmLockRewriter::new(npm_pkgs)?));
    }

    // pnpm: pnpm-lock.yaml (lockfileVersion 6.0+).
    if let Some(pnpm_pkgs) = by_eco.get("pnpm") {
        rewriters.push(Box::new(PnpmLockRewriter::new(pnpm_pkgs)?));
    }

    // Other ecosystems ship in Phases 10-16. Warn so the operator knows we're
    // skipping entries for ecosystems that aren't yet implemented.
    for eco in ["yarn", "bun", "pip", "go", "composer"] {
        if let Some(v) = by_eco.get(eco) {
            warn!(
                ecosystem = eco,
                count = v.len(),
                "supply-chain entries for ecosystem are not yet rewritten in v1; skipped"
            );
        }
    }

    Ok(rewriters)
}

/// Parse a `[<eco>:]<name>[@<ver>]` package identifier into a `CompromisedPackage`.
///
/// # Errors
///
/// Returns an error if the package name is empty.
fn parse_package_id(s: &str) -> Result<CompromisedPackage> {
    // Split on the first ':' only when it precedes any '@' — ecosystem prefix.
    // Note: npm scoped names start with '@', e.g. `npm:@types/node@1.0.0`.
    let (eco, rest) = if let Some(colon_idx) = s.find(':') {
        let before_colon = &s[..colon_idx];
        let after_colon = &s[colon_idx + 1..];
        // Only treat it as an ecosystem prefix if no '@' appears before the ':'.
        if before_colon.contains('@') {
            (None, s)
        } else {
            (Some(before_colon.to_string()), after_colon)
        }
    } else {
        (None, s)
    };

    // Split name from version on the last '@'. For npm scoped names like
    // `@types/node@1.0.0` the last '@' is at position 11 (the version separator),
    // not position 0 (the scope marker).
    let (name, version) = match rest.rsplit_once('@') {
        Some((n, v)) if !n.is_empty() => (n.to_string(), v.to_string()),
        _ => (rest.to_string(), "*".to_string()),
    };

    if name.is_empty() {
        return Err(anyhow!("empty package name in identifier: {s}"));
    }

    Ok(CompromisedPackage {
        name,
        ecosystem: eco.unwrap_or_else(|| "*".to_string()),
        versions: vec![version],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    })
}

/// Run a supply-chain scrub pass.
///
/// If `packages_override` is `Some`, the bundled advisory list is replaced by
/// the parsed package identifiers (used by `supply packages`).
fn run_pass(
    repo: Option<&Path>,
    args: &CommonArgs,
    packages_override: Option<&Vec<String>>,
) -> Result<()> {
    let repo_dir = resolve_repo_dir(repo)?;
    info!(repo = %repo_dir.display(), pass = "supply", execute = args.execute, "starting");

    let preflight_report = preflight::run(&repo_dir).context("pre-flight checks")?;

    let mut cfg = load_config(args.config.as_deref())?;

    if let Some(pkgs) = packages_override {
        let parsed: Vec<CompromisedPackage> = pkgs
            .iter()
            .map(|p| parse_package_id(p))
            .collect::<Result<_>>()?;
        cfg.compromised = parsed;
    }

    let lockfiles = build_lockfile_rewriters(&cfg, &args.exclude_ecosystem)?;
    if lockfiles.is_empty() {
        warn!(
            "no supply-chain rewriters active (empty advisory snapshot, \
             all ecosystems excluded, or all matched ecosystems unimplemented)"
        );
    }

    let lockfiles_slice: Vec<Box<dyn LockfileRewriter>> = lockfiles;

    let scan_stats =
        scan::scan(&repo_dir, None, None, None, Some(&lockfiles_slice)).context("dry-run scan")?;

    let plan_md = plan::render(&plan::PlanInputs {
        use_case: "supply",
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

    let exec_stats = if args.execute {
        let stats = engine::run(&repo_dir, None, None, None, Some(&lockfiles_slice))
            .context("rewrite engine")?;
        info!(?stats, "rewrite complete");
        let verify_stats = verify::run(&repo_dir, None, None, None, Some(&lockfiles_slice))
            .context("verification")?;
        info!(?verify_stats, "verification passed");
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

// =====================================================================
// Helper functions — same structure as in src/cli/spill.rs, adapted for supply.
// =====================================================================

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
    exec_stats: Option<&EngineStats>,
) -> Result<()> {
    if !common.execute && common.report.is_none() {
        return Ok(());
    }
    let supply_summary = exec_stats.map(|s| runbook::SupplySummary {
        lockfile_blobs_rewritten: s.blobs_rewritten,
    });
    let runbook_md = runbook::render(&runbook::RunbookInputs {
        repo_dir,
        backup_path,
        plan_markdown: plan_md,
        dry_run: !common.execute,
        gh: Some(gh_ctx),
        spill_secrets: false,
        supply: supply_summary,
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

    #[test]
    fn parses_eco_name_version() {
        let p = parse_package_id("npm:axios@1.6.1").unwrap();
        assert_eq!(p.ecosystem, "npm");
        assert_eq!(p.name, "axios");
        assert_eq!(p.versions, vec!["1.6.1"]);
    }

    #[test]
    fn parses_scoped_npm_name_with_eco_prefix() {
        let p = parse_package_id("npm:@types/node@1.0.0").unwrap();
        assert_eq!(p.ecosystem, "npm");
        assert_eq!(p.name, "@types/node");
        assert_eq!(p.versions, vec!["1.0.0"]);
    }

    #[test]
    fn parses_eco_optional() {
        let p = parse_package_id("axios@1.6.1").unwrap();
        assert_eq!(p.ecosystem, "*");
        assert_eq!(p.name, "axios");
        assert_eq!(p.versions, vec!["1.6.1"]);
    }

    #[test]
    fn parses_version_optional() {
        let p = parse_package_id("axios").unwrap();
        assert_eq!(p.ecosystem, "*");
        assert_eq!(p.name, "axios");
        assert_eq!(p.versions, vec!["*"]);
    }

    #[test]
    fn rejects_empty_name() {
        // Empty string — no name at all
        assert!(parse_package_id("").is_err());
    }

    #[test]
    fn parses_cargo_package() {
        let p = parse_package_id("cargo:serde@1.0.0").unwrap();
        assert_eq!(p.ecosystem, "cargo");
        assert_eq!(p.name, "serde");
        assert_eq!(p.versions, vec!["1.0.0"]);
    }

    #[test]
    fn parses_package_no_version_with_eco() {
        let p = parse_package_id("cargo:serde").unwrap();
        assert_eq!(p.ecosystem, "cargo");
        assert_eq!(p.name, "serde");
        assert_eq!(p.versions, vec!["*"]);
    }
}
