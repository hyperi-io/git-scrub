//  Project:      git-scrub
//  File:         tests/e2e/ai_composite.rs
//  Purpose:      E2e: AI composite scrub against the fixtures/ai-v1 corpus.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! AI composite scrub against the seeded `fixtures/ai-v1` history.
//!
//! The fixture seeds AI artefact files under `ai-residue-app/` (e.g.
//! `ai-residue-app/.claude/**`) and AI attribution trailers in commit
//! messages using both lowercase (`co-authored-by:`) and the GitHub
//! default uppercase (`Co-Authored-By:`) form.
//!
//! This test uses the bundled patterns directly -- no `include_extra`
//! workarounds. After the bug fixes:
//!  - Attribution patterns carry `(?i)` so all capitalisation variants match.
//!  - File patterns expand to `**/<pattern>` so nested paths like
//!    `ai-residue-app/.claude/notes.md` are purged alongside root-level ones.

use git_scrub::patterns::{
    AttributionRewriter, FileMatcher, FileMatcherOptions, attribution, discovery, files,
};

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/ai-v1";

#[test]
fn ai_composite_strips_seeded_attribution_and_artefacts() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    // Build the attribution rewriter from the bundled pattern library.
    let attr_loaded = discovery::load(
        discovery::ATTRIBUTION_FILE,
        discovery::EMBEDDED_ATTRIBUTION,
        None,
    )?;
    let attr_cfg = attribution::parse_yaml(&attr_loaded.content)?;
    let attribution_rewriter = AttributionRewriter::new(&attr_cfg, &[])?;

    // Build the file matcher from the bundled patterns only -- no include_extra.
    // The bundled patterns now expand to match nested paths via the `**/<pat>`
    // sibling added at load time, so ai-residue-app/.claude/** is covered.
    let files_loaded = discovery::load(discovery::FILES_FILE, discovery::EMBEDDED_FILES, None)?;
    let files_cfg = files::parse_yaml(&files_loaded.content)?;
    let file_matcher = FileMatcher::new(&files_cfg, &FileMatcherOptions::default())?;

    let stats = git_scrub::engine::run(
        working.path(),
        Some(&attribution_rewriter),
        Some(&file_matcher),
        None,
        None,
    )?;
    assert!(
        stats.commits_rewritten >= 3,
        "expected >=3 commits rewritten (claude, cursor+copilot, codex+aider all have matching attribution); got {}",
        stats.commits_rewritten,
    );
    assert!(
        stats.file_ops_dropped >= 5,
        "expected >=5 file ops dropped across .claude/, .cursor/, .codex/, .aider*; got {}",
        stats.file_ops_dropped,
    );

    // Walk rewritten history and confirm no AI attribution strings remain.
    let log = std::process::Command::new("git")
        .args([
            "-C",
            &working.path().display().to_string(),
            "log",
            "--all",
            "--pretty=format:%B",
        ])
        .output()?;
    anyhow::ensure!(log.status.success(), "git log failed");
    let bodies = String::from_utf8_lossy(&log.stdout);
    // All these forms are now matched by the case-insensitive bundled patterns.
    let stripped = [
        "Co-Authored-By: Claude",
        "co-authored-by: Cursor",
        "Co-authored-by: GitHub Copilot",
        "Co-Authored-By: Codex",
        "Co-Authored-By: aider",
        "Generated with",
    ];
    for needle in stripped {
        assert!(
            !bodies.contains(needle),
            "rewritten history still contains: {needle}",
        );
    }

    // Walk file ops in history and confirm AI artefact paths are gone.
    let names = std::process::Command::new("git")
        .args([
            "-C",
            &working.path().display().to_string(),
            "log",
            "--all",
            "--pretty=format:",
            "--name-only",
            "--",
            "ai-residue-app/",
        ])
        .output()?;
    let paths = String::from_utf8_lossy(&names.stdout);
    for forbidden_path in [".claude/", ".cursor/", ".codex/", ".aider"] {
        assert!(
            !paths.contains(forbidden_path),
            "rewritten history still references path containing: {forbidden_path}",
        );
    }

    // Verify pass -- no remaining matches.
    git_scrub::verify::run(
        working.path(),
        Some(&attribution_rewriter),
        Some(&file_matcher),
        None,
        None,
    )?;
    Ok(())
}
