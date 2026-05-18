//  Project:      git-scrub
//  File:         src/preflight.rs
//  Purpose:      Pre-rewrite safety checks against the target repository.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Pre-flight checks.
//!
//! Run before any rewrite (and in `--dry-run` too, so operators catch
//! gotchas early). Checks are ordered cheapest-first and fail-fast.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Errors raised by pre-flight.
#[derive(Debug, Error)]
pub enum PreflightError {
    /// `git` is not on `PATH`.
    #[error("`git` not found on PATH — git-scrub requires git to be installed")]
    GitMissing,

    /// `git` is on `PATH` but doesn't behave as expected.
    #[error("`git` invocation failed: {0}")]
    GitInvoke(#[source] std::io::Error),

    /// The target directory is not a git working tree.
    #[error("'{0}' is not a git repository")]
    NotARepo(PathBuf),

    /// Working tree has uncommitted changes.
    #[error("working tree has uncommitted changes — commit or stash before scrubbing")]
    DirtyWorkingTree,

    /// Working tree has untracked files we won't preserve.
    #[error("working tree has untracked files — commit or remove them before scrubbing")]
    UntrackedFiles,
}

/// Pre-flight result, with non-fatal observations the runbook should call out.
#[derive(Debug, Default, Clone)]
pub struct PreflightReport {
    /// Submodules detected at HEAD.
    pub submodules: Vec<String>,
    /// Whether any commit on the inspected refs is GPG-signed.
    pub has_signed_commits: bool,
    /// Number of branches that will be affected.
    pub branch_count: usize,
}

/// Run pre-flight checks against `repo_dir`.
///
/// On success returns a [`PreflightReport`] with informational findings.
/// Hard failures (dirty tree, not a repo) return an error.
pub fn run(repo_dir: &Path) -> Result<PreflightReport, PreflightError> {
    ensure_git_available()?;
    ensure_is_repo(repo_dir)?;
    ensure_clean_tree(repo_dir)?;

    Ok(PreflightReport {
        submodules: detect_submodules(repo_dir),
        has_signed_commits: detect_signed_commits(repo_dir),
        branch_count: count_branches(repo_dir),
    })
}

fn ensure_git_available() -> Result<(), PreflightError> {
    let out = Command::new("git").arg("--version").output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => Err(PreflightError::GitMissing),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(PreflightError::GitMissing),
        Err(e) => Err(PreflightError::GitInvoke(e)),
    }
}

fn ensure_is_repo(repo_dir: &Path) -> Result<(), PreflightError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .map_err(PreflightError::GitInvoke)?;
    if !out.status.success() {
        return Err(PreflightError::NotARepo(repo_dir.to_path_buf()));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim() != "true" {
        return Err(PreflightError::NotARepo(repo_dir.to_path_buf()));
    }
    Ok(())
}

fn ensure_clean_tree(repo_dir: &Path) -> Result<(), PreflightError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("status")
        .arg("--porcelain")
        .output()
        .map_err(PreflightError::GitInvoke)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut any_tracked = false;
    let mut any_untracked = false;
    for line in stdout.lines() {
        if line.starts_with("??") {
            any_untracked = true;
        } else if !line.is_empty() {
            any_tracked = true;
        }
    }
    if any_tracked {
        return Err(PreflightError::DirtyWorkingTree);
    }
    if any_untracked {
        return Err(PreflightError::UntrackedFiles);
    }
    Ok(())
}

fn detect_submodules(repo_dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("submodule")
        .arg("status")
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start_matches(['-', '+', 'U', ' ']);
            let mut parts = trimmed.split_whitespace();
            let _sha = parts.next()?;
            let path = parts.next()?;
            Some(path.to_string())
        })
        .collect()
}

fn detect_signed_commits(repo_dir: &Path) -> bool {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("log")
        .arg("--all")
        .arg("--pretty=%G?")
        .output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().any(|line| {
        let c = line.trim();
        c == "G" || c == "U" || c == "X" || c == "Y" || c == "R" || c == "E"
    })
}

fn count_branches(repo_dir: &Path) -> usize {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("for-each-ref")
        .arg("--format=%(refname)")
        .arg("refs/heads/")
        .output();
    let Ok(out) = out else { return 0 };
    if !out.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&out.stdout).lines().count()
}
