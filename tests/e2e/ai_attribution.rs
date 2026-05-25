//  Project:      git-scrub
//  File:         tests/e2e/ai_attribution.rs
//  Purpose:      E2e: attribution-only scrub against the fixtures/ai-v1 corpus.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Attribution-only scrub against the seeded `fixtures/ai-v1` history.
//!
//! This test builds an `AttributionRewriter` only -- no `FileMatcher` -- and
//! asserts that:
//!   - commit messages lose all AI attribution trailers
//!   - AI artefact files (`.claude/`, `.cursor/`, etc.) are STILL present in
//!     history (attribution-only mode does not touch file ops)

use git_scrub::patterns::{AttributionRewriter, attribution, discovery};

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/ai-v1";

#[test]
fn ai_attribution_only_strips_trailers_leaves_files_intact() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    // Build attribution rewriter only -- no file matcher.
    let attr_loaded = discovery::load(
        discovery::ATTRIBUTION_FILE,
        discovery::EMBEDDED_ATTRIBUTION,
        None,
    )?;
    let attr_cfg = attribution::parse_yaml(&attr_loaded.content)?;
    let attribution_rewriter = AttributionRewriter::new(&attr_cfg, &[])?;

    let stats = git_scrub::engine::run(
        working.path(),
        Some(&attribution_rewriter),
        None, // no file matcher
        None,
        None,
    )?;

    assert!(
        stats.commits_rewritten >= 3,
        "expected >=3 commits rewritten (all three AI-residue commits have trailers); got {}",
        stats.commits_rewritten,
    );

    // Confirm no trailer strings remain in rewritten commit bodies.
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

    let stripped_trailers = [
        "Co-Authored-By: Claude",
        "co-authored-by: Cursor",
        "Co-authored-by: GitHub Copilot",
        "Co-Authored-By: Codex",
        "Co-Authored-By: aider",
        "Generated with",
    ];
    for needle in stripped_trailers {
        assert!(
            !bodies.contains(needle),
            "rewritten history still contains trailer: {needle}",
        );
    }

    // AI artefact FILES must still be present -- attribution-only mode
    // does NOT drop file ops.
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

    for artefact_path in [".claude/", ".cursor/", ".codex/", ".aider"] {
        assert!(
            paths.contains(artefact_path),
            "artefact path '{artefact_path}' should still be in history after attribution-only scrub",
        );
    }

    Ok(())
}
