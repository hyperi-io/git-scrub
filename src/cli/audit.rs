//  Project:      git-scrub
//  File:         src/cli/audit.rs
//  Purpose:      `audit` umbrella subcommand — read-only multi-source survey.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `audit` subcommand.
//!
//! Read-only umbrella that runs every available audit pass and emits
//! a single consolidated markdown report. Designed for agentic
//! consumption — one invocation, full picture.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::Args;
use tracing::info;

use crate::patterns::lockfile::{
    BunLockRewriter, CargoLockRewriter, ComposerLockRewriter, GoSumRewriter, NpmLockRewriter,
    PipLockRewriter, PnpmLockRewriter, YarnLockRewriter,
};
use crate::patterns::supply::{CompromisedPackage, PurgeTarget, SupplyConfig};
use crate::patterns::{
    AttributionRewriter, FileMatcher, FileMatcherOptions, LockfileRewriter, attribution, discovery,
    files,
};
use crate::scan;

/// Args for `git-scrub audit`.
// AuditArgs is a CLI argument struct; every bool represents a distinct flag.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Audit AI residue in history (attribution trailers + artefact files).
    #[arg(long)]
    pub ai: bool,

    /// Audit supply chain residue in history (lockfile entries).
    #[arg(long)]
    pub supply: bool,

    /// Audit working-tree curation state (.gitignore, policy files, stray refs).
    #[arg(long)]
    pub curate: bool,

    /// Run every audit pass (default if no scope flag is set).
    #[arg(long)]
    pub all: bool,

    /// Override path to the supply-chain advisory YAML.
    #[arg(long, value_name = "PATH")]
    pub supply_config: Option<PathBuf>,

    /// Override path to the curate YAML.
    #[arg(long, value_name = "PATH")]
    pub curate_config: Option<PathBuf>,

    /// Override path to the attribution YAML.
    #[arg(long, value_name = "PATH")]
    pub attribution_config: Option<PathBuf>,

    /// Override path to the files YAML.
    #[arg(long, value_name = "PATH")]
    pub files_config: Option<PathBuf>,

    /// Write the report to a file instead of stdout.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
}

/// Run `git-scrub audit` to completion.
///
/// Runs every selected audit pass in read-only mode and emits a consolidated
/// markdown report to stdout or `--report <PATH>`.
pub fn run(args: &AuditArgs, repo: Option<&Path>) -> Result<()> {
    let repo_dir = resolve_repo_dir(repo)?;
    let scopes = resolve_scopes(args);
    info!(
        repo = %repo_dir.display(),
        ai = scopes.ai,
        supply = scopes.supply,
        curate = scopes.curate,
        "starting audit",
    );

    let mut report = String::with_capacity(4096);
    let _ = writeln!(report, "# git-scrub audit report");
    let _ = writeln!(report);
    let _ = writeln!(report, "**Repository:** `{}`", repo_dir.display());
    let _ = writeln!(report, "**Mode:** read-only audit");
    let _ = writeln!(report);

    let mut recommendations: Vec<String> = Vec::new();

    if scopes.ai {
        audit_ai(&repo_dir, args, &mut report, &mut recommendations)?;
    }
    if scopes.supply {
        audit_supply(&repo_dir, args, &mut report, &mut recommendations)?;
    }
    if scopes.curate {
        audit_curate(&repo_dir, args, &mut report, &mut recommendations)?;
    }

    // Summary
    let _ = writeln!(report, "## Summary");
    let _ = writeln!(report);
    if recommendations.is_empty() {
        let _ = writeln!(report, "No issues found.");
    } else {
        for rec in &recommendations {
            let _ = writeln!(report, "- {rec}");
        }
    }
    let _ = writeln!(report);

    if let Some(path) = &args.report {
        std::fs::write(path, &report)
            .with_context(|| format!("writing audit report to {}", path.display()))?;
        info!(path = %path.display(), "audit report written");
    } else {
        print!("{report}");
    }
    Ok(())
}

// ── Scope resolution ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct Scopes {
    ai: bool,
    supply: bool,
    curate: bool,
}

fn resolve_scopes(args: &AuditArgs) -> Scopes {
    // If --all or no flag set, audit everything.
    if args.all || (!args.ai && !args.supply && !args.curate) {
        return Scopes {
            ai: true,
            supply: true,
            curate: true,
        };
    }
    Scopes {
        ai: args.ai,
        supply: args.supply,
        curate: args.curate,
    }
}

// ── AI residue pass ───────────────────────────────────────────────────────────

fn audit_ai(
    repo_dir: &Path,
    args: &AuditArgs,
    report: &mut String,
    recommendations: &mut Vec<String>,
) -> Result<()> {
    let _ = writeln!(report, "## AI residue (history)");
    let _ = writeln!(report);

    let attribution_loaded = discovery::load(
        discovery::ATTRIBUTION_FILE,
        discovery::EMBEDDED_ATTRIBUTION,
        args.attribution_config.as_deref(),
    )
    .context("loading attribution pattern file")?;
    let attr_cfg =
        attribution::parse_yaml(&attribution_loaded.content).context("parsing attribution YAML")?;
    let attr_rewriter = AttributionRewriter::new(&attr_cfg, &[])?;

    let files_loaded = discovery::load(
        discovery::FILES_FILE,
        discovery::EMBEDDED_FILES,
        args.files_config.as_deref(),
    )
    .context("loading files pattern file")?;
    let files_cfg = files::parse_yaml(&files_loaded.content).context("parsing files YAML")?;
    let file_matcher = FileMatcher::new(&files_cfg, &FileMatcherOptions::default())?;

    let stats = scan::scan(
        repo_dir,
        Some(&attr_rewriter),
        Some(&file_matcher),
        None,
        None,
    )
    .context("scanning AI residue")?;

    let _ = writeln!(
        report,
        "- Commits with attribution trailers: {}",
        stats.commits_rewritten,
    );
    let _ = writeln!(
        report,
        "- File operations targeting artefact paths: {}",
        stats.file_ops_dropped,
    );
    let _ = writeln!(report);

    if stats.commits_rewritten > 0 || stats.file_ops_dropped > 0 {
        recommendations.push(format!(
            "Run `git-scrub ai composite --execute` to rewrite history \
             ({} commits, {} file ops affected)",
            stats.commits_rewritten, stats.file_ops_dropped,
        ));
    }
    Ok(())
}

// ── Supply chain pass ─────────────────────────────────────────────────────────

fn audit_supply(
    repo_dir: &Path,
    args: &AuditArgs,
    report: &mut String,
    recommendations: &mut Vec<String>,
) -> Result<()> {
    let _ = writeln!(report, "## Supply chain (history)");
    let _ = writeln!(report);

    let loaded = discovery::load(
        discovery::SUPPLY_CHAIN_FILE,
        discovery::EMBEDDED_SUPPLY_CHAIN,
        args.supply_config.as_deref(),
    )
    .context("loading supply-chain pattern file")?;
    let cfg: SupplyConfig =
        serde_yaml_ng::from_str(&loaded.content).context("parsing supply-chain.yaml")?;

    let rewriters = build_lockfile_rewriters(&cfg)?;

    if rewriters.is_empty() {
        let _ = writeln!(
            report,
            "- No supply-chain rewriters active (empty advisory snapshot).",
        );
        let _ = writeln!(report);
        return Ok(());
    }

    let stats = scan::scan(repo_dir, None, None, None, Some(&rewriters))
        .context("scanning supply chain")?;

    let _ = writeln!(
        report,
        "- Lockfile blobs matching advisories: {}",
        stats.blobs_rewritten,
    );
    let _ = writeln!(report);

    if stats.blobs_rewritten > 0 {
        recommendations.push(format!(
            "Run `git-scrub supply --execute` to rewrite lockfile history \
             ({} blobs affected)",
            stats.blobs_rewritten,
        ));
    }
    Ok(())
}

// ── Curate pass ───────────────────────────────────────────────────────────────

fn audit_curate(
    repo_dir: &Path,
    args: &AuditArgs,
    report: &mut String,
    recommendations: &mut Vec<String>,
) -> Result<()> {
    let _ = writeln!(report, "## Working-tree curation");
    let _ = writeln!(report);

    let summary = crate::cli::curate::audit_only(repo_dir, args.curate_config.as_deref())
        .context("running curate audit")?;

    let _ = writeln!(
        report,
        "- .gitignore missing entries: {}",
        summary.gitignore_missing.len(),
    );
    for path in &summary.gitignore_missing {
        let _ = writeln!(report, "  - `{path}`");
    }

    let _ = writeln!(
        report,
        "- Policy files missing: {}",
        summary.policy_files_missing.len(),
    );
    for name in &summary.policy_files_missing {
        let _ = writeln!(report, "  - `{name}`");
    }

    let _ = writeln!(
        report,
        "- Stray reference findings: {}",
        summary.stray_reference_findings.len(),
    );
    for (path, line, snippet) in summary.stray_reference_findings.iter().take(20) {
        let _ = writeln!(report, "  - `{}:{line}` — {snippet}", path.display());
    }
    if summary.stray_reference_findings.len() > 20 {
        let _ = writeln!(
            report,
            "  - (… {} more, truncated)",
            summary.stray_reference_findings.len() - 20,
        );
    }

    let _ = writeln!(
        report,
        "- Tracked files that should be gitignored: {}",
        summary.tracked_conflicts.len(),
    );
    for path in &summary.tracked_conflicts {
        let _ = writeln!(report, "  - `{}`", path.display());
    }
    let _ = writeln!(report);

    let needs_curate = !summary.gitignore_missing.is_empty()
        || !summary.policy_files_missing.is_empty()
        || !summary.stray_reference_findings.is_empty();
    if needs_curate {
        recommendations
            .push("Run `git-scrub ai curate --execute` to fix the working tree".to_string());
    }
    if !summary.tracked_conflicts.is_empty() {
        recommendations.push(format!(
            "{} file(s) need `git rm --cached <path>` AND a history rewrite via \
             `git-scrub ai files --execute`",
            summary.tracked_conflicts.len(),
        ));
    }
    Ok(())
}

// ── Lockfile rewriter factory ─────────────────────────────────────────────────

fn build_lockfile_rewriters(cfg: &SupplyConfig) -> Result<Vec<Box<dyn LockfileRewriter>>> {
    use std::collections::BTreeMap;

    let mut by_eco: BTreeMap<&str, Vec<&CompromisedPackage>> = BTreeMap::new();
    for p in &cfg.compromised {
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

    if let Some(pkgs) = by_eco.get("npm") {
        rewriters.push(Box::new(NpmLockRewriter::new(pkgs)?));
    }
    if let Some(pkgs) = by_eco.get("pnpm") {
        rewriters.push(Box::new(PnpmLockRewriter::new(pkgs)?));
    }
    if let Some(pkgs) = by_eco.get("yarn") {
        rewriters.push(Box::new(YarnLockRewriter::new(pkgs)?));
    }
    if let Some(pkgs) = by_eco.get("bun") {
        rewriters.push(Box::new(BunLockRewriter::new(pkgs)?));
    }
    if let Some(pkgs) = by_eco.get("pip") {
        rewriters.push(Box::new(PipLockRewriter::new(pkgs)?));
    }
    if let Some(pkgs) = by_eco.get("go") {
        rewriters.push(Box::new(GoSumRewriter::new(pkgs)?));
    }
    if let Some(pkgs) = by_eco.get("composer") {
        rewriters.push(Box::new(ComposerLockRewriter::new(pkgs)?));
    }

    Ok(rewriters)
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn resolve_repo_dir(repo: Option<&Path>) -> Result<PathBuf> {
    repo.map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine repository directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_scopes_defaults_to_all_when_no_flags() {
        let args = AuditArgs {
            ai: false,
            supply: false,
            curate: false,
            all: false,
            supply_config: None,
            curate_config: None,
            attribution_config: None,
            files_config: None,
            report: None,
        };
        let s = resolve_scopes(&args);
        assert!(s.ai);
        assert!(s.supply);
        assert!(s.curate);
    }

    #[test]
    fn resolve_scopes_all_flag_sets_everything() {
        let args = AuditArgs {
            ai: false,
            supply: false,
            curate: false,
            all: true,
            supply_config: None,
            curate_config: None,
            attribution_config: None,
            files_config: None,
            report: None,
        };
        let s = resolve_scopes(&args);
        assert!(s.ai);
        assert!(s.supply);
        assert!(s.curate);
    }

    #[test]
    fn resolve_scopes_curate_only() {
        let args = AuditArgs {
            ai: false,
            supply: false,
            curate: true,
            all: false,
            supply_config: None,
            curate_config: None,
            attribution_config: None,
            files_config: None,
            report: None,
        };
        let s = resolve_scopes(&args);
        assert!(!s.ai);
        assert!(!s.supply);
        assert!(s.curate);
    }

    #[test]
    fn resolve_repo_dir_uses_cwd_when_none() {
        let result = resolve_repo_dir(None);
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_repo_dir_uses_explicit_path() {
        let result = resolve_repo_dir(Some(Path::new("/tmp")));
        assert_eq!(result.unwrap(), PathBuf::from("/tmp"));
    }
}
