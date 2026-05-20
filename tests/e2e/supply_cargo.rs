//  Project:      git-scrub
//  File:         tests/e2e/supply_cargo.rs
//  Purpose:      E2e: Cargo.lock supply chain scrub against the fixture repo tag.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Cargo supply chain scrub against `fixtures/supply-v1/cargo`.

use git_scrub::patterns::LockfileRewriter;
use git_scrub::patterns::lockfile::CargoLockRewriter;
use git_scrub::patterns::supply::{CompromisedPackage, PurgeTarget};

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/supply-v1/cargo";

#[test]
fn supply_cargo_strips_seeded_bad_package_from_fixture_history() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };

    let working = snapshot_fixture(&upstream, FIXTURE_TAG)?;

    let bad = CompromisedPackage {
        name: "fake-malware-pkg-v1".to_string(),
        ecosystem: "cargo".to_string(),
        // Seed has both 1.6.1 and 1.6.2 in history; cover both with a range.
        versions: vec![">=1.6.0, <1.7.0".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(CargoLockRewriter::new(&[&bad])?);
    let rewriters = vec![rewriter];

    let stats =
        git_scrub::engine::run(working.path(), None, None, None, Some(rewriters.as_slice()))?;
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one blob rewritten (fixture history has the bad package in multiple commits): {stats:?}",
    );

    // Walk all of history and verify the bad package name has been purged
    // from every Cargo.lock blob.
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
    // The [[package]] entry for fake-malware-pkg-v1 is gone when the name
    // field line is absent. Note: the string may still appear in the
    // `dependencies` array of fixture-cargo-app (orphan references are
    // intentionally left intact per the design — see cargo.rs module doc).
    assert!(
        !history.contains("name = \"fake-malware-pkg-v1\""),
        "[[package]] entry for fake-malware-pkg-v1 still present in rewritten history",
    );

    // The innocent package's NAME LINE should still appear in some commits
    // (it appears in commits 1 and 3 in the fixture).
    assert!(
        history.contains("innocent-utils"),
        "innocent-utils should still appear in history",
    );

    // verify::run should pass — no remaining matches.
    git_scrub::verify::run(working.path(), None, None, None, Some(rewriters.as_slice()))?;

    Ok(())
}
