//  Project:      git-scrub
//  File:         tests/e2e/clean_combined.rs
//  Purpose:      E2e: clean umbrella with --supply against the cargo fixture.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! E2e: `clean --supply` dispatches through the umbrella pipeline and
//! produces identical results to `supply composite` for the cargo fixture.
//!
//! Phase 17 scope: exercises the `clean` → `engine::run` → `verify::run`
//! pipeline via the binary. Full multi-transformer composite tests (--ai +
//! --spill-paths + --supply together) land in Phase 19 once the AI fixture
//! branch exists.

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/supply-v1/cargo";

#[test]
fn clean_with_supply_strips_bad_package_from_cargo_fixture() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };

    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    // Write the advisory YAML to a temp dir that is OUTSIDE the repo working
    // tree. The preflight check rejects untracked files inside the repo, so the
    // config file must not land there.
    let config_dir = tempfile::tempdir()?;
    let advisory_yaml = r#"generated_at: "2026-05-22T00:00:00Z"
sources: []
compromised:
  - name: "fake-malware-pkg-v1"
    ecosystem: "cargo"
    versions: [">=1.6.0, <1.7.0"]
    purge_targets: [lockfile_entry]
lockfiles:
  cargo: ["Cargo.lock"]
vendored_paths:
  cargo: ["vendor/{name}/"]
"#;
    let config_path = config_dir.path().join("clean-fixture-advisory.yaml");
    std::fs::write(&config_path, advisory_yaml)?;

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_git-scrub"))
        .args([
            "-C",
            &working.path().display().to_string(),
            "clean",
            "--supply",
            "--supply-config",
            &config_path.display().to_string(),
            "--execute",
            "--no-backup",
            "--really-no-backup-i-mean-it",
        ])
        .output()?;

    anyhow::ensure!(
        output.status.success(),
        "clean --supply failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Walk all of history and assert the bad package name is gone from every
    // Cargo.lock blob.
    let log_output = std::process::Command::new("git")
        .args([
            "-C",
            &working.path().display().to_string(),
            "log",
            "--all",
            "-p",
            "--",
            "Cargo.lock",
        ])
        .output()?;
    anyhow::ensure!(log_output.status.success(), "git log failed");
    let history = String::from_utf8(log_output.stdout)?;

    assert!(
        !history.contains("name = \"fake-malware-pkg-v1\""),
        "seeded bad package still present after clean --supply pass",
    );

    // The innocent package should remain untouched throughout history.
    assert!(
        history.contains("innocent-utils"),
        "innocent-utils should still appear in rewritten history",
    );

    Ok(())
}
