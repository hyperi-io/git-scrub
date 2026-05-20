//  Project:      git-scrub
//  File:         src/runbook.rs
//  Purpose:      Markdown runbook emission (one file per scrub run).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Runbook output.
//!
//! After `--execute` (or after `--dry-run` if `--report <path>` is supplied)
//! we write a markdown runbook to the operator's cache directory. The
//! runbook captures the plan, backup location, force-push commands,
//! collaborator notification templates, and the post-rewrite checklist.

use std::fmt::Write;
use std::path::Path;

use crate::gh::GhContext;

/// Summary of supply-chain transformations performed during this run.
///
/// Emitted in the runbook when a supply chain pass produced rewrites.
/// v1 reports counts only; per-entry enumeration of which packages were
/// stripped (ecosystem/name/version) is deferred to a later phase that
/// plumbs structured data through the engine.
#[derive(Debug, Default)]
pub struct SupplySummary {
    /// Number of lockfile blobs rewritten across history.
    pub lockfile_blobs_rewritten: usize,
}

/// Inputs to runbook rendering.
#[derive(Debug)]
pub struct RunbookInputs<'a> {
    /// Repository working tree.
    pub repo_dir: &'a Path,
    /// Where the backup mirror clone lives (None if `--no-backup`).
    pub backup_path: Option<&'a Path>,
    /// Plan markdown to embed.
    pub plan_markdown: &'a str,
    /// Whether this was a dry-run or execute pass.
    pub dry_run: bool,
    /// Optional GitHub context (forks, PRs, branch protection).
    /// `None` means gh integration was not attempted.
    pub gh: Option<&'a GhContext>,
    /// Emit a credential-rotation section. Set by `spill secrets`; anything
    /// that leaked must be considered compromised regardless of rewrite success.
    pub spill_secrets: bool,
    /// Supply-chain summary. Present only when a supply pass ran with `--execute`
    /// and produced at least one rewrite. `None` for dry-run or non-supply passes.
    pub supply: Option<SupplySummary>,
}

/// Render a runbook markdown document.
#[must_use]
pub fn render(inputs: &RunbookInputs<'_>) -> String {
    let mut out = String::with_capacity(4096);
    let _ = writeln!(out, "# git-scrub runbook");
    let _ = writeln!(out);
    let _ = writeln!(out, "**Repository:** `{}`", inputs.repo_dir.display());
    let _ = writeln!(
        out,
        "**Mode:** {}",
        if inputs.dry_run {
            "dry-run (no rewrite performed)"
        } else {
            "execute (rewrite performed)"
        }
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## Plan");
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", inputs.plan_markdown);
    let _ = writeln!(out);

    let _ = writeln!(out, "## Backup");
    let _ = writeln!(out);
    if let Some(p) = inputs.backup_path {
        let _ = writeln!(out, "Mirror clone written to `{}`.", p.display());
        let _ = writeln!(out);
        let _ = writeln!(out, "To restore from this backup:");
        let _ = writeln!(out);
        let _ = writeln!(out, "```bash");
        let _ = writeln!(
            out,
            "git -C {} push --mirror <restore-target-remote>",
            p.display()
        );
        let _ = writeln!(out, "```");
    } else {
        let _ = writeln!(
            out,
            "**No backup was taken** (`--no-backup` was used). Rollback is \
             not possible from git-scrub. If you have an external mirror or \
             a fresh clone elsewhere, use that."
        );
    }
    let _ = writeln!(out);

    if inputs.spill_secrets {
        let _ = writeln!(out, "## Credential rotation");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "**Anything that leaked must be considered compromised.** History \
             rewrite removes the secret from the repository but does not \
             revoke or rotate it. The window between leak and rewrite was \
             enough for an attacker to copy the value."
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "Before pushing, for every leaked credential:");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "1. **Revoke** the existing credential at the issuing service.\n\
             2. **Issue a new credential** and update every consumer (CI, \
                deployment, local devs).\n\
             3. **Audit usage logs** for unexpected activity between leak \
                time and revocation.\n\
             4. **Document** the rotation in your incident record so the \
                next on-call has the timeline."
        );
        let _ = writeln!(out);
    }

    if let Some(summary) = &inputs.supply
        && summary.lockfile_blobs_rewritten > 0
    {
        let _ = writeln!(out, "## Supply chain summary");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "**Lockfile blobs rewritten:** {} across history.",
            summary.lockfile_blobs_rewritten,
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "To inspect per-entry detail across history:");
        let _ = writeln!(out);
        let _ = writeln!(out, "```bash");
        let _ = writeln!(
            out,
            "git -C {} log --all -p -- Cargo.lock",
            inputs.repo_dir.display(),
        );
        let _ = writeln!(out, "```");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Per-package enumeration of stripped entries is not yet \
             reported in the runbook; that requires engine instrumentation \
             planned for a later release.",
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Force-push commands");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "After verifying the rewritten history is correct, push the \
         rewritten refs to your remote. Use `--force-with-lease` (not \
         `--force`) so concurrent updates abort cleanly:"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "```bash");
    let _ = writeln!(
        out,
        "git -C {} push --force-with-lease --all",
        inputs.repo_dir.display()
    );
    let _ = writeln!(
        out,
        "git -C {} push --force-with-lease --tags",
        inputs.repo_dir.display()
    );
    let _ = writeln!(out, "```");
    let _ = writeln!(out);

    let _ = writeln!(out, "## Collaborator notification");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Send the following to every collaborator with a local clone:"
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> History on `<repo>` has been rewritten by `git-scrub` to remove \
         AI tool residue. Your local clone is stale. To get back to a clean \
         state, the fastest path is:\n>\n> ```bash\n> git fetch --all --prune\n\
         > git reset --hard origin/<your-branch>\n> ```\n>\n> If you have \
         local work in progress, stash it first or rebase onto the new \
         history. Open issues stay reachable by ID; PR diff URLs for closed \
         PRs may still surface old SHAs."
    );
    let _ = writeln!(out);

    render_gh_section(&mut out, inputs.gh);

    let _ = writeln!(out, "## What this tool cannot fix");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- **`refs/pull/N/head`** — immutable on GitHub. Commits on closed \
         PR branches stay reachable. To delete: open a GitHub Support ticket."
    );
    let _ = writeln!(
        out,
        "- **Forks** — independent clones. Public-repo cleanup is lossy. \
         See the *Forks* section above (if `gh` was available)."
    );
    let _ = writeln!(
        out,
        "- **Contributors graph** — GitHub caches the contributor list. AI \
         co-authors may persist even after rewrite. Reach out to support if \
         this matters."
    );
    let _ = writeln!(
        out,
        "- **Commit signatures** — all signatures are invalidated by \
         rewrite. Re-signing requires the original maintainer's private key."
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## Verification");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "git-scrub ran a post-rewrite verification pass that re-greps the \
         scrubbed history for the configured patterns. Zero matches expected. \
         If the operator suspects something slipped through:"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "```bash");
    let _ = writeln!(
        out,
        "git -C {} log --all --pretty=format:'%H %s%n%b' \\",
        inputs.repo_dir.display()
    );
    let _ = writeln!(out, "    | grep -iE 'claude|cursor|codex|copilot|aider'");
    let _ = writeln!(out, "```");
    out
}

/// Emit a GitHub-specific section if any `gh` context was gathered.
fn render_gh_section(out: &mut String, gh: Option<&GhContext>) {
    let Some(gh) = gh else {
        return;
    };

    let _ = writeln!(out, "## GitHub follow-up");
    let _ = writeln!(out);

    if let Some(slug) = &gh.slug {
        let _ = writeln!(out, "**Repository:** [`{slug}`](https://github.com/{slug})");
        let _ = writeln!(out);
    }

    // In-flight workflow runs — these block --execute upstream of here, so
    // for the runbook this is informational. If we got here in dry-run with
    // active runs, the operator needs to know before they retry --execute.
    if !gh.in_flight_runs.is_empty() {
        let _ = writeln!(out, "### Active GitHub Actions runs");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} workflow run(s) are currently active on this repository. \
             `git-scrub --execute` will refuse to rewrite while any are in \
             flight. Wait for them to finish or cancel them before retrying:\n",
            gh.in_flight_runs.len(),
        );
        for run in &gh.in_flight_runs {
            let _ = writeln!(
                out,
                "- [{}]({}) — `{}` on `{}` (status: `{}`)",
                run.display_title, run.url, run.workflow_name, run.head_branch, run.status,
            );
        }
        let _ = writeln!(out);
    }

    // Branch protection — operator may need to lift before pushing.
    if let Some(protected) = gh.default_branch_protected {
        if protected {
            let _ = writeln!(
                out,
                "### Branch protection: **ON**\n\n\
                 The default branch is protected. Force-push will be \
                 rejected until protection is lifted. After verifying the \
                 rewrite, temporarily relax protection, push, and restore:\n"
            );
            let _ = writeln!(out, "```bash");
            let _ = writeln!(
                out,
                "# Web UI: Settings → Branches → Default branch → Edit\n\
                 # Or via API (requires admin scope):\n\
                 # gh api -X DELETE repos/<owner>/<repo>/branches/<branch>/protection\n\
                 # ...push...\n\
                 # gh api -X PUT repos/<owner>/<repo>/branches/<branch>/protection -F ..."
            );
            let _ = writeln!(out, "```");
            let _ = writeln!(out);
        } else {
            let _ = writeln!(
                out,
                "Branch protection: not enabled on the default branch.\n"
            );
        }
    }

    // Forks — public-repo cleanup is lossy; list owners to notify.
    let _ = writeln!(out, "### Forks");
    let _ = writeln!(out);
    if gh.forks.is_empty() {
        let _ = writeln!(out, "No forks reported.");
    } else {
        let _ = writeln!(out, "{} fork(s) detected:\n", gh.forks.len());
        for fork in &gh.forks {
            let suffix = if fork.archived { " *(archived)*" } else { "" };
            let _ = writeln!(out, "- [{}]({}){}", fork.full_name, fork.html_url, suffix);
        }
        let _ = writeln!(
            out,
            "\nForks are independent clones. **You cannot clean these.** \
             Reach out to fork owners and ask them to either delete their \
             fork or re-fork after your rewrite is upstream. Template:"
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "> Hi, the upstream repository's history has been rewritten by \
             `git-scrub` to remove [AI tool residue / leaked content]. Your \
             fork still contains the pre-rewrite history including the \
             content we removed. Please either re-sync your fork against \
             the rewritten upstream (`git push --force-with-lease` after \
             rebasing) or delete the fork if you no longer need it. \
             Apologies for the disruption."
        );
    }
    let _ = writeln!(out);

    // Closed PRs — diff URLs may still surface pre-rewrite SHAs.
    let _ = writeln!(out, "### Closed pull requests");
    let _ = writeln!(out);
    if gh.closed_prs.is_empty() {
        let _ = writeln!(out, "No closed pull requests reported.");
    } else {
        let _ = writeln!(
            out,
            "{} closed PR(s) detected. Commit SHAs referenced from closed-PR \
             diff URLs (`refs/pull/N/head`) remain reachable until GitHub GC \
             runs. To get them removed earlier, contact GitHub Support \
             referencing this list:\n",
            gh.closed_prs.len(),
        );
        for pr in gh.closed_prs.iter().take(50) {
            let _ = writeln!(
                out,
                "- [#{} {}]({}) — head: `{}`",
                pr.number, pr.title, pr.url, pr.head_ref_name
            );
        }
        if gh.closed_prs.len() > 50 {
            let _ = writeln!(
                out,
                "\n_…and {} more (truncated for brevity). Use \
                 `gh pr list --state closed --limit 999` for the full list._",
                gh.closed_prs.len() - 50,
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "**GitHub Support template:**\n");
        let _ = writeln!(
            out,
            "> Subject: Request removal of pre-rewrite SHAs from closed-PR refs\n>\n\
             > Hello, we rewrote the history of `{}` on {} using `git-scrub` \
             to remove [AI tool residue / leaked content]. The new history is \
             clean, but `refs/pull/N/head` for the closed PRs listed below \
             still references the pre-rewrite commits. Please remove these \
             refs so the leaked SHAs are no longer URL-reachable.\n>\n\
             > [paste the closed-PR list above]\n>\n\
             > Thanks.",
            gh.slug.as_deref().unwrap_or("<owner>/<repo>"),
            chrono_today(),
        );
    }
    let _ = writeln!(out);

    // Surface anything gh couldn't determine.
    if !gh.notes.is_empty() {
        let _ = writeln!(out, "### gh notes");
        let _ = writeln!(out);
        for note in &gh.notes {
            let _ = writeln!(out, "- {note}");
        }
        let _ = writeln!(out);
    }
}

/// Return a YYYY-MM-DD string for today (UTC). Lightweight — no chrono dep.
fn chrono_today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Days since 1970-01-01.
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (y, m, d) = ymd_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Convert days-since-1970-01-01 to (year, month, day). Civil-from-days
/// algorithm by Howard Hinnant, public-domain reference implementation.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ymd_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::{ForkInfo, GhContext, PullInfo};
    use std::path::PathBuf;

    fn make_inputs<'a>(repo: &'a PathBuf, gh: Option<&'a GhContext>) -> RunbookInputs<'a> {
        RunbookInputs {
            repo_dir: repo,
            backup_path: None,
            plan_markdown: "[plan]",
            dry_run: false,
            gh,
            spill_secrets: false,
            supply: None,
        }
    }

    #[test]
    fn renders_without_gh_context() {
        let repo = PathBuf::from("/tmp/repo");
        let out = render(&make_inputs(&repo, None));
        assert!(out.contains("# git-scrub runbook"));
        assert!(!out.contains("GitHub follow-up"));
    }

    #[test]
    fn renders_forks_when_present() {
        let repo = PathBuf::from("/tmp/repo");
        let gh = GhContext {
            slug: Some("alice/proj".into()),
            forks: vec![ForkInfo {
                full_name: "bob/proj".into(),
                html_url: "https://github.com/bob/proj".into(),
                archived: false,
            }],
            ..Default::default()
        };
        let out = render(&make_inputs(&repo, Some(&gh)));
        assert!(out.contains("GitHub follow-up"));
        assert!(out.contains("alice/proj"));
        assert!(out.contains("bob/proj"));
        assert!(out.contains("Reach out to fork owners"));
    }

    #[test]
    fn renders_closed_prs_with_support_template() {
        let repo = PathBuf::from("/tmp/repo");
        let gh = GhContext {
            slug: Some("alice/proj".into()),
            closed_prs: vec![PullInfo {
                number: 42,
                title: "Add feature".into(),
                url: "https://github.com/alice/proj/pull/42".into(),
                head_ref_name: "feat/x".into(),
            }],
            ..Default::default()
        };
        let out = render(&make_inputs(&repo, Some(&gh)));
        assert!(out.contains("#42"));
        assert!(out.contains("GitHub Support template"));
        assert!(out.contains("refs/pull/N/head"));
    }

    #[test]
    fn renders_branch_protection_warning_when_protected() {
        let repo = PathBuf::from("/tmp/repo");
        let gh = GhContext {
            slug: Some("alice/proj".into()),
            default_branch_protected: Some(true),
            ..Default::default()
        };
        let out = render(&make_inputs(&repo, Some(&gh)));
        assert!(out.contains("Branch protection: **ON**"));
    }

    #[test]
    fn surfaces_gh_notes() {
        let repo = PathBuf::from("/tmp/repo");
        let gh = GhContext {
            notes: vec!["gh not authenticated".into()],
            ..Default::default()
        };
        let out = render(&make_inputs(&repo, Some(&gh)));
        assert!(out.contains("gh not authenticated"));
    }

    #[test]
    fn renders_credential_rotation_when_spill_secrets() {
        let repo = PathBuf::from("/tmp/repo");
        let mut inputs = make_inputs(&repo, None);
        inputs.spill_secrets = true;
        let out = render(&inputs);
        assert!(out.contains("## Credential rotation"));
        assert!(out.contains("compromised"));
    }

    #[test]
    fn omits_credential_rotation_by_default() {
        let repo = PathBuf::from("/tmp/repo");
        let out = render(&make_inputs(&repo, None));
        assert!(!out.contains("## Credential rotation"));
    }

    #[test]
    fn ymd_from_days_known_dates() {
        // 2026-05-15 was day 20_588 since epoch.
        // Just sanity-check it parses to (2026, ?, ?).
        let (y, _m, _d) = ymd_from_days(20_588);
        assert_eq!(y, 2026);
    }

    #[test]
    fn renders_supply_summary_when_blobs_rewritten() {
        let repo = PathBuf::from("/tmp/repo");
        let mut inputs = make_inputs(&repo, None);
        inputs.supply = Some(SupplySummary {
            lockfile_blobs_rewritten: 5,
        });
        let out = render(&inputs);
        assert!(out.contains("## Supply chain summary"));
        assert!(out.contains("5 across history"));
        assert!(out.contains("git -C /tmp/repo log"));
    }

    #[test]
    fn omits_supply_summary_when_no_blobs_rewritten() {
        let repo = PathBuf::from("/tmp/repo");
        let mut inputs = make_inputs(&repo, None);
        inputs.supply = Some(SupplySummary {
            lockfile_blobs_rewritten: 0,
        });
        let out = render(&inputs);
        assert!(!out.contains("## Supply chain summary"));
    }

    #[test]
    fn omits_supply_summary_when_none() {
        let repo = PathBuf::from("/tmp/repo");
        let out = render(&make_inputs(&repo, None));
        assert!(!out.contains("## Supply chain summary"));
    }
}
