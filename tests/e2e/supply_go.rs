//  Project:      git-scrub
//  File:         tests/e2e/supply_go.rs
//  Purpose:      E2e: go.sum supply chain scrub against the fixture repo tag.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! go supply chain scrub against `fixtures/supply-v1/go`.

use git_scrub::patterns::LockfileRewriter;
use git_scrub::patterns::lockfile::GoSumRewriter;
use git_scrub::patterns::supply::{CompromisedPackage, PurgeTarget};

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const FIXTURE_TAG: &str = "fixtures/supply-v1/go";

#[test]
fn supply_go_strips_seeded_bad_package_from_fixture_history() -> anyhow::Result<()> {
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
        name: "github.com/fake-malware-pkg-v1/pkg".to_string(),
        ecosystem: "go".to_string(),
        // Fixture has both 1.6.1 and 1.6.2 in history; cover both with a range.
        versions: vec![">=1.6.0, <1.7.0".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(GoSumRewriter::new(&[&bad])?);
    let rewriters = vec![rewriter];

    let stats =
        git_scrub::engine::run(working.path(), None, None, None, Some(rewriters.as_slice()))?;
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one blob rewritten in go fixture history: {stats:?}",
    );

    // Walk all history and verify the bad module's entries are gone.
    let log_output = std::process::Command::new("git")
        .args([
            "-C",
            &working.path().display().to_string(),
            "log",
            "--all",
            "-p",
            "--",
            "go-app/go.sum",
        ])
        .output()?;
    anyhow::ensure!(log_output.status.success(), "git log failed");
    let history = String::from_utf8(log_output.stdout)?;
    assert!(
        !history.contains("github.com/fake-malware-pkg-v1/pkg"),
        "seeded bad module's entry still present in rewritten history",
    );

    // verify::run should pass — no remaining matches.
    git_scrub::verify::run(working.path(), None, None, None, Some(rewriters.as_slice()))?;

    Ok(())
}
