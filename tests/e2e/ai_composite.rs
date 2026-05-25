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
//! `ai-residue-app/.claude/**`). The bundled patterns match paths at the
//! repo root (`.claude/**`), so the test supplies `include_extra` globs
//! scoped to the fixture subdirectory to exercise the file-drop path.
//! Attribution rewriting uses the bundled YAML directly, which matches the
//! seeded commit-message trailers regardless of path prefix.

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

    // Build the file matcher from the bundled patterns PLUS fixture-scoped
    // extras. The embedded patterns target repo-root paths (.claude/**); the
    // fixture nests artefacts under ai-residue-app/, so we add prefixed globs
    // via include_extra so the engine's file-drop path is fully exercised.
    let files_loaded = discovery::load(discovery::FILES_FILE, discovery::EMBEDDED_FILES, None)?;
    let files_cfg = files::parse_yaml(&files_loaded.content)?;
    let opts = FileMatcherOptions {
        include_extra: vec![
            "ai-residue-app/.claude/**".to_string(),
            "ai-residue-app/.cursor/**".to_string(),
            "ai-residue-app/.codex/**".to_string(),
            "ai-residue-app/.aider*".to_string(),
        ],
        ..Default::default()
    };
    let file_matcher = FileMatcher::new(&files_cfg, &opts)?;

    let stats = git_scrub::engine::run(
        working.path(),
        Some(&attribution_rewriter),
        Some(&file_matcher),
        None,
        None,
    )?;
    assert!(
        stats.commits_rewritten >= 2,
        "expected >=2 commits rewritten (claude commit and cursor+copilot commit had matching attribution); got {}",
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
    // These lines ARE matched by the bundled patterns and must be gone.
    // (Codex/aider use uppercase Co-Authored-By which doesn't match the
    // lowercase-only bundled patterns; cursor uses a non-standard email.
    // The fixture intentionally tests these edge cases -- the engine strips
    // only what the patterns match.)
    let stripped = [
        "Co-Authored-By: Claude",
        "Co-authored-by: GitHub Copilot",
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
