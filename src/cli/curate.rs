//  Project:      git-scrub
//  File:         src/cli/curate.rs
//  Purpose:      `ai curate` working-tree cleanup subcommand.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Working-tree curation — add gitignore entries, canonical policy files,
//! and scrub stray tool-specific references from tracked source files.
//!
//! This subcommand operates on the **working tree**, not on git history.
//! It complements the history-rewrite modes (`composite`, `attribution`,
//! `files`); it does not replace them.

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use tracing::{info, warn};

use crate::patterns::discovery;

// ── CLI args ──────────────────────────────────────────────────────────────────

/// Args for `ai curate`.
#[derive(Debug, Args)]
pub struct CurateArgs {
    /// Apply the curation. Without this flag the command is a dry-run report.
    #[arg(long)]
    pub execute: bool,

    /// Skip all confirmation prompts and apply every stage's changes.
    /// Implies `--execute` for each individual stage but NOT for stage 4
    /// (tracked conflicts), which always requires manual action.
    #[arg(long)]
    pub force: bool,

    /// Override path to the `ai-curate.yaml` config.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Directory to source canonical policy files from.
    /// Defaults to bundled fallback content shipped with the binary.
    #[arg(long, value_name = "PATH")]
    pub policy_source: Option<PathBuf>,

    /// Write the report to this path instead of stdout.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
}

// ── Stage result types ────────────────────────────────────────────────────────

/// Stage 1 result: gitignore gap analysis.
#[derive(Debug, Default)]
pub struct GitignoreStage {
    /// Paths from the curate config that are absent from `.gitignore`.
    pub missing: Vec<String>,
    /// Number of paths already present (informational).
    pub already_present: usize,
}

/// Stage 2 result: policy file gap analysis.
#[derive(Debug, Default)]
pub struct PolicyStage {
    /// Policy file names that do not yet exist at the repo root.
    pub missing: Vec<String>,
    /// Number of policy files already present (informational).
    pub already_present: usize,
}

/// A single stray-reference finding.
#[derive(Debug)]
pub struct ScrubFinding {
    /// Repo-relative path to the file.
    pub file: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// The matched line text (trimmed, for display).
    pub matched_text: String,
    /// The `find` string from the pattern (needed to perform the replacement).
    pub find: String,
    /// The `replace` string from the pattern.
    pub replace: String,
}

/// Stage 3 result: stray reference scan.
#[derive(Debug, Default)]
pub struct ReferenceStage {
    /// All findings across all tracked files.
    pub findings: Vec<ScrubFinding>,
}

/// Stage 4 result: tracked-but-should-be-ignored conflict detection.
#[derive(Debug, Default)]
pub struct TrackedConflictStage {
    /// Tracked paths that should be ignored per the curate config.
    pub tracked_conflicts: Vec<PathBuf>,
}

// ── Audit-only public API ─────────────────────────────────────────────────────

/// Summary returned by [`audit_only`] — results of all four curate audit passes.
#[derive(Debug, Default)]
pub struct CurateAuditSummary {
    /// Gitignore paths missing from `.gitignore`.
    pub gitignore_missing: Vec<String>,
    /// Policy file names absent from the repo root.
    pub policy_files_missing: Vec<String>,
    /// Stray tool-reference findings: `(repo-relative path, 1-based line, matched text)`.
    pub stray_reference_findings: Vec<(PathBuf, usize, String)>,
    /// Tracked paths that should be gitignored per the curate config.
    pub tracked_conflicts: Vec<PathBuf>,
}

/// Run all four curate audit passes in read-only mode and return a summary.
///
/// This is called by the `audit` umbrella subcommand so it can include curate
/// results in the consolidated report without re-implementing the scan logic.
pub fn audit_only(repo_dir: &Path, config: Option<&Path>) -> Result<CurateAuditSummary> {
    let cfg = load_config(config)?;
    let stage1 = scan_gitignore_additions(repo_dir, &cfg)?;
    let stage2 = scan_policy_files(repo_dir, &cfg);
    let stage3 = scan_stray_references(repo_dir, &cfg)?;
    let stage4 = scan_tracked_conflicts(repo_dir, &cfg)?;
    Ok(CurateAuditSummary {
        gitignore_missing: stage1.missing,
        policy_files_missing: stage2.missing,
        stray_reference_findings: stage3
            .findings
            .into_iter()
            .map(|f| (f.file, f.line, f.matched_text))
            .collect(),
        tracked_conflicts: stage4.tracked_conflicts,
    })
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run `ai curate` against the given repo directory.
///
/// Dry-run unless `args.execute` is set.
pub fn run(args: &CurateArgs, repo: Option<&Path>) -> Result<()> {
    let repo_dir = resolve_repo_dir(repo)?;
    info!(
        repo = %repo_dir.display(),
        execute = args.execute,
        force = args.force,
        "starting ai curate"
    );

    let cfg = load_config(args.config.as_deref())?;

    // ── Scan all four stages ──────────────────────────────────────────────────
    let stage1 = scan_gitignore_additions(&repo_dir, &cfg)?;
    let stage2 = scan_policy_files(&repo_dir, &cfg);
    let stage3 = scan_stray_references(&repo_dir, &cfg)?;
    let stage4 = scan_tracked_conflicts(&repo_dir, &cfg)?;

    // ── Render and emit the report ────────────────────────────────────────────
    let report = render_report(&repo_dir, &stage1, &stage2, &stage3, &stage4, args.execute);
    if let Some(path) = &args.report {
        std::fs::write(path, &report)
            .with_context(|| format!("writing report to {}", path.display()))?;
        info!(path = %path.display(), "report written");
    } else {
        println!("{report}");
    }

    if !args.execute {
        return Ok(());
    }

    // ── Apply stage 1: gitignore additions ───────────────────────────────────
    if !stage1.missing.is_empty() && confirm("Add missing paths to .gitignore?", args.force)? {
        apply_gitignore_additions(&repo_dir, &stage1.missing)?;
        info!(count = stage1.missing.len(), "gitignore additions applied");
    }

    // ── Apply stage 2: policy files ───────────────────────────────────────────
    if !stage2.missing.is_empty() && confirm("Add canonical policy files?", args.force)? {
        apply_policy_files(&repo_dir, &stage2.missing, args.policy_source.as_deref())?;
        info!(count = stage2.missing.len(), "policy files added");
    }

    // ── Apply stage 3: reference scrubs ───────────────────────────────────────
    if !stage3.findings.is_empty()
        && confirm(
            &format!("Strip {} stray reference(s)?", stage3.findings.len()),
            args.force,
        )?
    {
        apply_reference_scrubs(&repo_dir, &stage3.findings)?;
        info!(count = stage3.findings.len(), "stray references scrubbed");
    }

    // ── Stage 4: tracked conflicts need manual intervention ───────────────────
    if !stage4.tracked_conflicts.is_empty() {
        warn!(
            count = stage4.tracked_conflicts.len(),
            "files are currently tracked that should be gitignored; \
             history rewrite needed — see runbook for `git rm --cached` procedure"
        );
    }

    Ok(())
}

// ── Config loading ────────────────────────────────────────────────────────────

fn load_config(override_path: Option<&Path>) -> Result<crate::patterns::curate::CurateConfig> {
    let loaded = discovery::load(
        discovery::AI_CURATE_FILE,
        discovery::EMBEDDED_AI_CURATE,
        override_path,
    )
    .context("loading ai-curate.yaml")?;
    let cfg = serde_yaml_ng::from_str(&loaded.content).context("parsing ai-curate.yaml")?;
    Ok(cfg)
}

// ── Stage 1: gitignore additions ──────────────────────────────────────────────

fn scan_gitignore_additions(
    repo_dir: &Path,
    cfg: &crate::patterns::curate::CurateConfig,
) -> Result<GitignoreStage> {
    let gitignore_path = repo_dir.join(".gitignore");
    let existing_lines: std::collections::HashSet<String> = if gitignore_path.is_file() {
        let content = std::fs::read_to_string(&gitignore_path).context("reading .gitignore")?;
        content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let mut stage = GitignoreStage::default();
    for path in cfg.all_gitignore_paths() {
        if existing_lines.contains(path) {
            stage.already_present += 1;
        } else {
            stage.missing.push(path.to_string());
        }
    }
    Ok(stage)
}

fn apply_gitignore_additions(repo_dir: &Path, missing: &[String]) -> Result<()> {
    let gitignore_path = repo_dir.join(".gitignore");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore_path)
        .context("opening .gitignore for append")?;
    writeln!(file, "\n# Added by git-scrub ai curate")?;
    for path in missing {
        writeln!(file, "{path}")?;
    }
    Ok(())
}

// ── Stage 2: policy files ─────────────────────────────────────────────────────

fn scan_policy_files(repo_dir: &Path, cfg: &crate::patterns::curate::CurateConfig) -> PolicyStage {
    let mut stage = PolicyStage::default();
    for name in &cfg.policy_files {
        if repo_dir.join(name).is_file() {
            stage.already_present += 1;
        } else {
            stage.missing.push(name.clone());
        }
    }
    stage
}

fn apply_policy_files(
    repo_dir: &Path,
    missing: &[String],
    policy_source: Option<&Path>,
) -> Result<()> {
    for name in missing {
        let dest = repo_dir.join(name);
        if let Some(src_dir) = policy_source {
            let src = src_dir.join(name);
            std::fs::copy(&src, &dest)
                .with_context(|| format!("copying {name} from {}", src_dir.display()))?;
        } else {
            let content = embedded_policy_content(name)
                .ok_or_else(|| anyhow!("no embedded content for policy file '{name}'"))?;
            std::fs::write(&dest, content).with_context(|| format!("writing {name}"))?;
        }
        info!(file = %name, "policy file added");
    }
    Ok(())
}

fn embedded_policy_content(name: &str) -> Option<&'static str> {
    match name {
        "AI-TRAINING-POLICY.md" => Some(discovery::EMBEDDED_AI_TRAINING_POLICY),
        "robots.txt" => Some(discovery::EMBEDDED_ROBOTS_TXT),
        _ => None,
    }
}

// ── Stage 3: stray reference scrub ────────────────────────────────────────────

fn scan_stray_references(
    repo_dir: &Path,
    cfg: &crate::patterns::curate::CurateConfig,
) -> Result<ReferenceStage> {
    if cfg.reference_scrub.patterns.is_empty() {
        return Ok(ReferenceStage::default());
    }

    let tracked = git_ls_files(repo_dir)?;
    let mut stage = ReferenceStage::default();

    for rel_path in &tracked {
        let full_path = repo_dir.join(rel_path);
        if !full_path.is_file() {
            continue;
        }
        let Ok(content) = read_text_file(&full_path) else {
            continue; // binary or unreadable — skip
        };

        for (line_idx, line) in content.lines().enumerate() {
            for pattern in &cfg.reference_scrub.patterns {
                if line.contains(pattern.find.as_str()) {
                    stage.findings.push(ScrubFinding {
                        file: PathBuf::from(rel_path),
                        line: line_idx + 1,
                        matched_text: line.trim().to_string(),
                        find: pattern.find.clone(),
                        replace: pattern.replace.clone(),
                    });
                }
            }
        }
    }
    Ok(stage)
}

/// Read a file and return its content as a UTF-8 string, or an error if it
/// looks like a binary file (contains null bytes in the first 8 KiB).
fn read_text_file(path: &Path) -> Result<String> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    // Heuristic: check first 8 KiB for null bytes.
    let probe = &raw[..raw.len().min(8192)];
    if probe.contains(&0u8) {
        return Err(anyhow!("binary file"));
    }
    String::from_utf8(raw).map_err(|_| anyhow!("non-UTF-8 file"))
}

fn apply_reference_scrubs(repo_dir: &Path, findings: &[ScrubFinding]) -> Result<()> {
    // Group findings by file to avoid re-reading/writing a file multiple times.
    let mut by_file: std::collections::BTreeMap<&Path, Vec<&ScrubFinding>> =
        std::collections::BTreeMap::new();
    for f in findings {
        by_file.entry(f.file.as_path()).or_default().push(f);
    }

    for (rel_path, file_findings) in by_file {
        let full_path = repo_dir.join(rel_path);
        let content = read_text_file(&full_path)?;
        let mut new_content = content.clone();
        for finding in file_findings {
            new_content = new_content.replace(&finding.find, &finding.replace);
        }
        if new_content != content {
            std::fs::write(&full_path, &new_content)
                .with_context(|| format!("writing {}", full_path.display()))?;
            info!(file = %rel_path.display(), "stray references replaced");
        }
    }
    Ok(())
}

// ── Stage 4: tracked conflicts ────────────────────────────────────────────────

fn scan_tracked_conflicts(
    repo_dir: &Path,
    cfg: &crate::patterns::curate::CurateConfig,
) -> Result<TrackedConflictStage> {
    let paths = cfg.all_gitignore_paths();
    if paths.is_empty() {
        return Ok(TrackedConflictStage::default());
    }

    // Build a globset from the curate paths.
    let mut builder = globset::GlobSetBuilder::new();
    for p in &paths {
        // Normalise:
        //   ".claude/"  → ".claude/**"   (directory prefix — match any child)
        //   "STATE.md"  → "STATE.md"     (exact basename)
        //   ".aider*"   → ".aider*"      (wildcard — pass through as glob)
        let pattern = if p.ends_with('/') {
            let trimmed = p.trim_end_matches('/');
            format!("{trimmed}/**")
        } else {
            (*p).to_string()
        };
        let glob = globset::Glob::new(&pattern)
            .with_context(|| format!("building glob for pattern '{p}'"))?;
        builder.add(glob);
    }
    let set = builder.build().context("building globset")?;

    let tracked = git_ls_files(repo_dir)?;
    let mut stage = TrackedConflictStage::default();
    for rel in &tracked {
        if set.is_match(rel) {
            stage.tracked_conflicts.push(PathBuf::from(rel));
        }
    }
    Ok(stage)
}

// ── Report rendering ──────────────────────────────────────────────────────────

fn render_report(
    repo_dir: &Path,
    stage1: &GitignoreStage,
    stage2: &PolicyStage,
    stage3: &ReferenceStage,
    stage4: &TrackedConflictStage,
    execute: bool,
) -> String {
    let mode = if execute { "execute" } else { "dry-run" };
    let mut out = format!(
        "# git-scrub ai curate — {mode}\nrepo: {}\n\n",
        repo_dir.display()
    );

    // Stage 1
    out.push_str("## Stage 1: .gitignore additions\n");
    if stage1.missing.is_empty() {
        out.push_str("  ✓ nothing to add\n");
    } else {
        let _ = writeln!(
            out,
            "  {} path(s) missing ({} already present):",
            stage1.missing.len(),
            stage1.already_present
        );
        for p in &stage1.missing {
            let _ = writeln!(out, "    + {p}");
        }
    }
    out.push('\n');

    // Stage 2
    out.push_str("## Stage 2: canonical policy files\n");
    if stage2.missing.is_empty() {
        out.push_str("  ✓ nothing to add\n");
    } else {
        let _ = writeln!(
            out,
            "  {} file(s) missing ({} already present):",
            stage2.missing.len(),
            stage2.already_present
        );
        for f in &stage2.missing {
            let _ = writeln!(out, "    + {f}");
        }
    }
    out.push('\n');

    // Stage 3
    out.push_str("## Stage 3: stray tool references\n");
    if stage3.findings.is_empty() {
        out.push_str("  ✓ no stray references found\n");
    } else {
        let _ = writeln!(out, "  {} finding(s):", stage3.findings.len());
        for f in &stage3.findings {
            let _ = writeln!(
                out,
                "    {}:{} — {}",
                f.file.display(),
                f.line,
                f.matched_text
            );
        }
    }
    out.push('\n');

    // Stage 4
    out.push_str("## Stage 4: tracked files that should be gitignored\n");
    if stage4.tracked_conflicts.is_empty() {
        out.push_str("  ✓ no tracked conflicts\n");
    } else {
        let _ = writeln!(
            out,
            "  {} file(s) tracked but should be gitignored \
             (requires `git rm --cached` or history rewrite):",
            stage4.tracked_conflicts.len()
        );
        for p in &stage4.tracked_conflicts {
            let _ = writeln!(out, "    ! {}", p.display());
        }
    }
    out.push('\n');

    let needs_action = !stage1.missing.is_empty()
        || !stage2.missing.is_empty()
        || !stage3.findings.is_empty()
        || !stage4.tracked_conflicts.is_empty();

    if needs_action {
        if execute {
            out.push_str(
                "Stages 1-3 applied (if confirmed). \
                 Stage 4 requires manual intervention.\n",
            );
        } else {
            out.push_str("Dry-run complete. Re-run with --execute to apply stages 1-3.\n");
        }
    } else {
        out.push_str("Repository is already clean — nothing to do.\n");
    }

    out
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Run `git ls-files` and return repo-relative paths as strings.
fn git_ls_files(repo_dir: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("ls-files")
        .output()
        .context("running git ls-files")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let paths = out
        .stdout
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.is_empty())
        .collect();
    Ok(paths)
}

/// Resolve the repo directory from an optional explicit path, falling back to
/// the current working directory.
fn resolve_repo_dir(repo: Option<&Path>) -> Result<PathBuf> {
    repo.map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine repository directory"))
}

/// Show a Y/n prompt on stdin and return true if the user accepted.
///
/// - If `force` is set, always returns `true` without prompting.
/// - If stdin is not a TTY (CI environment), returns `false` unless `force`.
fn confirm(question: &str, force: bool) -> Result<bool> {
    if force {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        warn!(
            question = %question,
            "stdin is not a TTY; skipping stage (use --force to apply anyway)"
        );
        return Ok(false);
    }
    print!("{question} [Y/n] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    Ok(answer.is_empty() || answer == "y" || answer == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_policy_content_returns_known_files() {
        assert!(embedded_policy_content("AI-TRAINING-POLICY.md").is_some());
        assert!(embedded_policy_content("robots.txt").is_some());
        assert!(embedded_policy_content("unknown.txt").is_none());
    }

    #[test]
    fn render_report_clean_state_message() {
        let stage1 = GitignoreStage {
            missing: vec![],
            already_present: 3,
        };
        let stage2 = PolicyStage {
            missing: vec![],
            already_present: 2,
        };
        let stage3 = ReferenceStage { findings: vec![] };
        let stage4 = TrackedConflictStage {
            tracked_conflicts: vec![],
        };
        let report = render_report(
            Path::new("/tmp/testrepo"),
            &stage1,
            &stage2,
            &stage3,
            &stage4,
            false,
        );
        assert!(
            report.contains("nothing to do"),
            "Expected 'nothing to do' in: {report}"
        );
    }

    #[test]
    fn render_report_shows_missing_gitignore_entries() {
        let stage1 = GitignoreStage {
            missing: vec![".claude/".to_string()],
            already_present: 0,
        };
        let stage2 = PolicyStage::default();
        let stage3 = ReferenceStage::default();
        let stage4 = TrackedConflictStage::default();
        let report = render_report(
            Path::new("/tmp/testrepo"),
            &stage1,
            &stage2,
            &stage3,
            &stage4,
            false,
        );
        assert!(report.contains(".claude/"));
        assert!(report.contains("--execute"));
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
