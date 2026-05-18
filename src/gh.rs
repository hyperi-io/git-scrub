//  Project:      git-scrub
//  File:         src/gh.rs
//  Purpose:      Optional GitHub-CLI integration — forks, branch protection, PRs.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Optional GitHub integration via the `gh` CLI.
//!
//! Everything in this module is **best-effort and feature-detected** —
//! if `gh` is missing, unauthenticated, or the repo has no GitHub remote,
//! we record the gap and emit a placeholder in the runbook telling the
//! operator to handle that case manually. We never fail the scrub
//! because of a missing `gh`.
//!
//! Parsing is factored out from subprocess invocation so the JSON
//! parsers are unit-testable with offline fixtures.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// Context the `gh` integration gathers, or reasons it couldn't.
#[derive(Debug, Clone, Default)]
pub struct GhContext {
    /// Owner/repo (`octocat/Hello-World`) inferred from the `origin` remote.
    pub slug: Option<String>,
    /// Forks visible to the current `gh` auth.
    pub forks: Vec<ForkInfo>,
    /// Closed pull requests on the affected branches.
    pub closed_prs: Vec<PullInfo>,
    /// Whether branch protection is enabled on the default branch.
    pub default_branch_protected: Option<bool>,
    /// GitHub Actions runs currently in flight that touch tracked branches.
    pub in_flight_runs: Vec<WorkflowRun>,
    /// Non-fatal reasons something couldn't be queried.
    pub notes: Vec<String>,
}

/// A GitHub Actions workflow run reported by `gh run list`.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRun {
    /// Run database ID.
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    /// Workflow file name (e.g. `ci.yml`).
    #[serde(default, rename = "workflowName")]
    pub workflow_name: String,
    /// Display title for the run (commit subject usually).
    #[serde(default)]
    pub display_title: String,
    /// HEAD branch the run is operating on.
    #[serde(default, rename = "headBranch")]
    pub head_branch: String,
    /// One of `queued`, `in_progress`, `requested`, `waiting`, etc.
    #[serde(default)]
    pub status: String,
    /// Web URL of the run.
    #[serde(default)]
    pub url: String,
}

/// A single fork as reported by `gh api repos/:owner/:repo/forks`.
#[derive(Debug, Clone, Deserialize)]
pub struct ForkInfo {
    /// `owner/repo` slug.
    pub full_name: String,
    /// HTML URL of the fork.
    pub html_url: String,
    /// Whether the fork is archived.
    #[serde(default)]
    pub archived: bool,
}

/// A pull request as reported by `gh pr list --state closed --json ...`.
#[derive(Debug, Clone, Deserialize)]
pub struct PullInfo {
    /// PR number.
    pub number: u64,
    /// Title text.
    pub title: String,
    /// `https://github.com/<owner>/<repo>/pull/<n>`.
    pub url: String,
    /// Head ref name (the branch on the fork or in the same repo).
    #[serde(default, rename = "headRefName")]
    pub head_ref_name: String,
}

/// Result of `gh api repos/:owner/:repo/branches/:branch/protection` —
/// presence (200) means protection is on, 404 means off.
#[derive(Debug, Clone, Deserialize)]
struct BranchProtectionProbe {
    #[allow(dead_code)]
    url: String,
}

/// Gather best-effort GitHub context for `repo_dir`.
///
/// Each step is feature-detected: if a probe fails (gh missing, no
/// remote, no auth), it adds a note and skips. Returns a populated
/// [`GhContext`] regardless.
#[must_use]
pub fn gather(repo_dir: &Path) -> GhContext {
    let mut ctx = GhContext::default();

    if !gh_available() {
        ctx.notes.push(
            "`gh` CLI not found on PATH — fork enumeration and branch protection \
             checks were skipped. Install gh (https://cli.github.com/) for \
             richer runbooks."
                .into(),
        );
        return ctx;
    }

    if !gh_authenticated() {
        ctx.notes.push(
            "`gh` is installed but not authenticated. Run `gh auth login` to \
             enable fork enumeration and branch-protection checks."
                .into(),
        );
        return ctx;
    }

    let Some(slug) = detect_github_slug(repo_dir) else {
        ctx.notes.push(
            "No GitHub remote detected on this repository. Skipping fork / \
             PR / branch-protection probes."
                .into(),
        );
        return ctx;
    };
    ctx.slug = Some(slug.clone());

    match query_forks(&slug) {
        Ok(forks) => ctx.forks = forks,
        Err(e) => ctx.notes.push(format!("Fork enumeration failed: {e}")),
    }

    match query_closed_prs(&slug) {
        Ok(prs) => ctx.closed_prs = prs,
        Err(e) => ctx.notes.push(format!("Closed-PR enumeration failed: {e}")),
    }

    let default_branch = detect_default_branch(repo_dir);
    if let Some(branch) = default_branch.as_deref() {
        match query_branch_protection(&slug, branch) {
            Ok(b) => ctx.default_branch_protected = Some(b),
            Err(e) => ctx.notes.push(format!(
                "Branch protection probe for `{branch}` failed: {e}"
            )),
        }
    } else {
        ctx.notes.push(
            "Could not infer the default branch from the working tree — \
             skipping branch-protection probe."
                .into(),
        );
    }

    match query_in_flight_runs(&slug) {
        Ok(runs) => ctx.in_flight_runs = runs,
        Err(e) => ctx
            .notes
            .push(format!("In-flight workflow probe failed: {e}")),
    }

    ctx
}

fn query_in_flight_runs(slug: &str) -> Result<Vec<WorkflowRun>, String> {
    // `gh run list` accepts `--status` filters. We query each "active" status
    // separately and merge — the gh JSON output is a single array per call.
    let mut all: Vec<WorkflowRun> = Vec::new();
    for status in ["in_progress", "queued", "requested", "waiting"] {
        let out = Command::new("gh")
            .args([
                "run",
                "list",
                "--repo",
                slug,
                "--status",
                status,
                "--limit",
                "50",
                "--json",
                "databaseId,workflowName,displayTitle,headBranch,status,url",
            ])
            .output()
            .map_err(|e| format!("gh subprocess failed: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        let chunk = parse_runs_json(&String::from_utf8_lossy(&out.stdout))?;
        all.extend(chunk);
    }
    Ok(all)
}

/// Parse `gh run list ... --json` output.
pub fn parse_runs_json(json: &str) -> Result<Vec<WorkflowRun>, String> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).map_err(|e| format!("runs parse: {e}"))
}

fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn gh_authenticated() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn detect_github_slug(repo_dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    parse_github_slug(&url)
}

fn detect_default_branch(repo_dir: &Path) -> Option<String> {
    // Prefer `gh repo view` if available — gives the actual GitHub default
    // branch. Fall back to `git symbolic-ref refs/remotes/origin/HEAD`.
    let out = Command::new("gh")
        .arg("-R")
        .arg(detect_github_slug(repo_dir)?)
        .args([
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ])
        .output()
        .ok()?;
    if out.status.success() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    s.strip_prefix("origin/").map(str::to_string)
}

fn query_forks(slug: &str) -> Result<Vec<ForkInfo>, String> {
    let out = Command::new("gh")
        .args(["api", "--paginate", &format!("repos/{slug}/forks")])
        .output()
        .map_err(|e| format!("gh subprocess failed: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    parse_forks_json(&String::from_utf8_lossy(&out.stdout))
}

fn query_closed_prs(slug: &str) -> Result<Vec<PullInfo>, String> {
    let out = Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            slug,
            "--state",
            "closed",
            "--limit",
            "200",
            "--json",
            "number,title,url,headRefName",
        ])
        .output()
        .map_err(|e| format!("gh subprocess failed: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    parse_pulls_json(&String::from_utf8_lossy(&out.stdout))
}

fn query_branch_protection(slug: &str, branch: &str) -> Result<bool, String> {
    let out = Command::new("gh")
        .args(["api", &format!("repos/{slug}/branches/{branch}/protection")])
        .output()
        .map_err(|e| format!("gh subprocess failed: {e}"))?;
    if out.status.success() {
        let _: BranchProtectionProbe = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("branch-protection JSON parse: {e}"))?;
        return Ok(true);
    }
    // gh returns non-zero on 404; treat that as "not protected".
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("404") || stderr.contains("Branch not protected") {
        return Ok(false);
    }
    Err(stderr.trim().to_string())
}

/// Parse a GitHub remote URL (SSH or HTTPS) into `owner/repo`.
///
/// Accepts:
/// - `https://github.com/<owner>/<repo>.git`
/// - `https://github.com/<owner>/<repo>`
/// - `git@github.com:<owner>/<repo>.git`
/// - `ssh://git@github.com/<owner>/<repo>.git`
///
/// Returns `None` for any other host.
#[must_use]
pub fn parse_github_slug(url: &str) -> Option<String> {
    let url = url.trim();
    let stripped = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let slug = stripped.strip_suffix(".git").unwrap_or(stripped);
    // owner/repo only — reject anything with extra slashes.
    let mut parts = slug.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Parse the JSON output of `gh api repos/:o/:r/forks` (paginated array form).
pub fn parse_forks_json(json: &str) -> Result<Vec<ForkInfo>, String> {
    // `gh api --paginate` concatenates JSON arrays without re-wrapping;
    // e.g. `[{...}][{...}]`. Handle both single-array and concatenated.
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(direct) = serde_json::from_str::<Vec<ForkInfo>>(trimmed) {
        return Ok(direct);
    }
    // Try concatenated arrays.
    let mut out: Vec<ForkInfo> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in trimmed.bytes().enumerate() {
        match b {
            b'[' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let slice = &trimmed[start..=i];
                    let chunk: Vec<ForkInfo> =
                        serde_json::from_str(slice).map_err(|e| format!("forks parse: {e}"))?;
                    out.extend(chunk);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Parse the JSON output of `gh pr list --json number,title,url,headRefName`.
pub fn parse_pulls_json(json: &str) -> Result<Vec<PullInfo>, String> {
    serde_json::from_str(json.trim()).map_err(|e| format!("pulls parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_remote_url() {
        assert_eq!(
            parse_github_slug("https://github.com/hyperi-io/git-scrub.git"),
            Some("hyperi-io/git-scrub".to_string())
        );
        assert_eq!(
            parse_github_slug("https://github.com/hyperi-io/git-scrub"),
            Some("hyperi-io/git-scrub".to_string())
        );
    }

    #[test]
    fn parses_ssh_remote_url() {
        assert_eq!(
            parse_github_slug("git@github.com:hyperi-io/git-scrub.git"),
            Some("hyperi-io/git-scrub".to_string())
        );
        assert_eq!(
            parse_github_slug("ssh://git@github.com/hyperi-io/git-scrub.git"),
            Some("hyperi-io/git-scrub".to_string())
        );
    }

    #[test]
    fn rejects_non_github_urls() {
        assert!(parse_github_slug("https://gitlab.com/foo/bar.git").is_none());
        assert!(parse_github_slug("https://github.com/foo").is_none());
        assert!(parse_github_slug("https://github.com/foo/bar/baz").is_none());
        assert!(parse_github_slug("").is_none());
    }

    #[test]
    fn parses_forks_array() {
        let json = r#"[
            {"full_name":"alice/git-scrub","html_url":"https://github.com/alice/git-scrub","archived":false},
            {"full_name":"bob/git-scrub","html_url":"https://github.com/bob/git-scrub","archived":true}
        ]"#;
        let forks = parse_forks_json(json).unwrap();
        assert_eq!(forks.len(), 2);
        assert_eq!(forks[0].full_name, "alice/git-scrub");
        assert!(forks[1].archived);
    }

    #[test]
    fn parses_concatenated_forks_pages() {
        let json = r#"[{"full_name":"alice/git-scrub","html_url":"https://github.com/alice/git-scrub","archived":false}][{"full_name":"bob/git-scrub","html_url":"https://github.com/bob/git-scrub","archived":false}]"#;
        let forks = parse_forks_json(json).unwrap();
        assert_eq!(forks.len(), 2);
    }

    #[test]
    fn parses_empty_forks() {
        assert!(parse_forks_json("[]").unwrap().is_empty());
        assert!(parse_forks_json("").unwrap().is_empty());
    }

    #[test]
    fn parses_workflow_runs() {
        let json = r#"[
            {"databaseId":1,"workflowName":"CI","displayTitle":"Fix bug","headBranch":"main","status":"in_progress","url":"https://github.com/o/r/actions/runs/1"},
            {"databaseId":2,"workflowName":"Deploy","displayTitle":"Release","headBranch":"release","status":"queued","url":"https://github.com/o/r/actions/runs/2"}
        ]"#;
        let runs = parse_runs_json(json).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].database_id, 1);
        assert_eq!(runs[0].head_branch, "main");
        assert_eq!(runs[1].status, "queued");
    }

    #[test]
    fn parses_pull_requests() {
        let json = r#"[
            {"number":42,"title":"Add feature","url":"https://github.com/o/r/pull/42","headRefName":"feat/x"},
            {"number":43,"title":"Fix bug","url":"https://github.com/o/r/pull/43","headRefName":"fix/y"}
        ]"#;
        let prs = parse_pulls_json(json).unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0].number, 42);
        assert_eq!(prs[1].head_ref_name, "fix/y");
    }
}
