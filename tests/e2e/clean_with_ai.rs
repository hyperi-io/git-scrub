//  Project:      git-scrub
//  File:         tests/e2e/clean_with_ai.rs
//  Purpose:      E2e: clean --ai against the fixtures/ai-v1 corpus via binary.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `clean --ai` end-to-end test against `fixtures/ai-v1`.
//!
//! Invokes the binary directly via `Command` and asserts that the rewritten
//! history contains neither attribution trailers nor AI artefact paths.

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/ai-v1";

#[test]
fn clean_with_ai_strips_trailers_and_artefacts() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_git-scrub"))
        .args([
            "-C",
            &working.path().display().to_string(),
            "clean",
            "--ai",
            "--execute",
            "--no-backup",
            "--really-no-backup-i-mean-it",
        ])
        .output()?;

    anyhow::ensure!(
        output.status.success(),
        "clean --ai failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
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

    // Confirm AI artefact file paths are gone.
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
            "artefact path '{forbidden_path}' should be gone after clean --ai",
        );
    }

    Ok(())
}
