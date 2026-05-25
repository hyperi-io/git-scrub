//  Project:      git-scrub
//  File:         tests/e2e/ai_files.rs
//  Purpose:      E2e: files-only scrub against the fixtures/ai-v1 corpus.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Files-only scrub against the seeded `fixtures/ai-v1` history.
//!
//! This test builds a `FileMatcher` only -- no `AttributionRewriter` -- and
//! asserts that:
//!   - AI artefact paths (`.claude/`, `.cursor/`, etc.) are gone from history
//!   - Attribution trailers ARE STILL in commit messages (files-only mode
//!     does NOT touch commit messages)

use git_scrub::patterns::{FileMatcher, FileMatcherOptions, discovery, files};

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/ai-v1";

#[test]
fn ai_files_only_drops_artefacts_leaves_attribution_intact() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    // Build file matcher only -- no attribution rewriter.
    let files_loaded = discovery::load(discovery::FILES_FILE, discovery::EMBEDDED_FILES, None)?;
    let files_cfg = files::parse_yaml(&files_loaded.content)?;
    let file_matcher = FileMatcher::new(&files_cfg, &FileMatcherOptions::default())?;

    let stats = git_scrub::engine::run(
        working.path(),
        None, // no attribution rewriter
        Some(&file_matcher),
        None,
        None,
    )?;

    assert!(
        stats.file_ops_dropped >= 5,
        "expected >=5 file ops dropped across .claude/, .cursor/, .codex/, .aider*; got {}",
        stats.file_ops_dropped,
    );

    // AI artefact file paths must be gone from history.
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
            "artefact path '{forbidden_path}' should be gone after files-only scrub",
        );
    }

    // Attribution trailers must STILL be present -- files-only mode does NOT
    // touch commit messages.
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

    // At least one of the original attribution trailer forms must survive.
    let trailer_strings = [
        "Co-Authored-By: Claude",
        "co-authored-by: Cursor",
        "Co-authored-by: GitHub Copilot",
        "Co-Authored-By: Codex",
        "Co-Authored-By: aider",
    ];
    let any_trailer_present = trailer_strings.iter().any(|needle| bodies.contains(needle));
    assert!(
        any_trailer_present,
        "all attribution trailers are gone after files-only scrub -- files mode should not touch commit messages",
    );

    Ok(())
}
