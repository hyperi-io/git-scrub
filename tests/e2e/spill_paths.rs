//  Project:      git-scrub
//  File:         tests/e2e/spill_paths.rs
//  Purpose:      E2e: spill-paths scrub against the fixtures/spill-v1 corpus.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Spill-paths scrub against the seeded `fixtures/spill-v1` history.

use git_scrub::patterns::{FileMatcher, FileMatcherOptions, files};

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/spill-v1";

#[test]
fn spill_paths_strips_seeded_credential_files() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    // Operator-supplied paths to drop (matches `git-scrub spill paths <pattern>...`).
    let cfg = files::FileConfig::default();
    let opts = FileMatcherOptions {
        include_extra: vec![
            "spill-app/config.py".to_string(),
            "spill-app/customer-notes.md".to_string(),
        ],
        ..Default::default()
    };
    let matcher = FileMatcher::new(&cfg, &opts)?;

    let stats = git_scrub::engine::run(working.path(), None, Some(&matcher), None, None)?;
    assert!(
        stats.file_ops_dropped >= 1,
        "expected >=1 file op dropped; got {}",
        stats.file_ops_dropped,
    );

    // Walk all file paths touched in spill-app/ history. After scrub the two
    // credential paths must not appear in any commit's file list.
    let names = std::process::Command::new("git")
        .args([
            "-C",
            &working.path().display().to_string(),
            "log",
            "--all",
            "--pretty=format:",
            "--name-only",
            "--",
            "spill-app/",
        ])
        .output()?;
    let paths = String::from_utf8_lossy(&names.stdout);
    assert!(
        !paths.contains("spill-app/config.py"),
        "config.py should be gone from history",
    );
    assert!(
        !paths.contains("spill-app/customer-notes.md"),
        "customer-notes.md should be gone from history",
    );

    // Innocent files should still be reachable in history.
    assert!(
        paths.contains("spill-app/main.py"),
        "main.py should remain in history",
    );
    assert!(
        paths.contains("spill-app/README.md"),
        "README.md should remain in history",
    );

    git_scrub::verify::run(working.path(), None, Some(&matcher), None, None)?;
    Ok(())
}
