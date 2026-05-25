//  Project:      git-scrub
//  File:         tests/e2e/audit_against_fixtures.rs
//  Purpose:      E2e: audit subcommand against ai-v1 and supply-v1/cargo fixtures.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `audit` end-to-end tests against the fixture corpus.
//!
//! Two scenarios:
//!   1. `audit --ai` against `fixtures/ai-v1` -- confirms the report lists
//!      attribution findings and recommends the composite fix command.
//!   2. `audit --supply` against `fixtures/supply-v1/cargo` with a temp advisory
//!      YAML containing `fake-malware-pkg-v1` -- confirms the report lists
//!      lockfile matches and recommends the supply fix command.
//!
//! `audit` is read-only and always exits 0, even when findings are present.

use crate::common::{FixtureRepoOutcome, fixture_repo, snapshot_fixture};

const AI_FIXTURE_TAG: &str = "fixtures/ai-v1";
const CARGO_FIXTURE_TAG: &str = "fixtures/supply-v1/cargo";

#[test]
fn audit_ai_reports_attribution_findings_and_recommends_fix() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, AI_FIXTURE_TAG)?;

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_git-scrub"))
        .args(["-C", &working.path().display().to_string(), "audit", "--ai"])
        .output()?;

    // audit is always read-only; exit code must be 0 even with findings.
    anyhow::ensure!(
        output.status.success(),
        "audit --ai exited non-zero:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("# git-scrub audit report"),
        "expected report header in stdout; got:\n{stdout}",
    );

    assert!(
        stdout.contains("## AI residue (history)"),
        "expected AI residue section; got:\n{stdout}",
    );

    // The fixture has three commits with attribution trailers.
    // The report line is: "- Commits with attribution trailers: N"
    // We check that N is >= 3 by parsing the value out of the line.
    let trailer_count = stdout
        .lines()
        .find(|l| l.contains("Commits with attribution trailers:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    assert!(
        trailer_count >= 3,
        "expected >= 3 commits with attribution trailers in report; found {trailer_count}; report:\n{stdout}",
    );

    // Should recommend the composite fix.
    assert!(
        stdout.contains("git-scrub ai composite --execute"),
        "expected recommendation for 'git-scrub ai composite --execute'; got:\n{stdout}",
    );

    Ok(())
}

#[test]
fn audit_supply_reports_lockfile_findings_and_recommends_fix() -> anyhow::Result<()> {
    let upstream = match fixture_repo() {
        FixtureRepoOutcome::Resolved(p) => p,
        FixtureRepoOutcome::Skipped(reason) => {
            eprintln!("SKIP: {reason}");
            return Ok(());
        }
        FixtureRepoOutcome::Failed(msg) => anyhow::bail!("fixture resolution failed: {msg}"),
    };
    let working = snapshot_fixture(&upstream, CARGO_FIXTURE_TAG)?;

    // Write the advisory YAML to a temp dir OUTSIDE the repo working tree.
    // The preflight check rejects untracked files inside the repo, so the
    // config must not land there.
    let config_dir = tempfile::tempdir()?;
    let advisory_yaml = r#"generated_at: "2026-05-25T00:00:00Z"
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
    let config_path = config_dir.path().join("audit-advisory.yaml");
    std::fs::write(&config_path, advisory_yaml)?;

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_git-scrub"))
        .args([
            "-C",
            &working.path().display().to_string(),
            "audit",
            "--supply",
            "--supply-config",
            &config_path.display().to_string(),
        ])
        .output()?;

    // audit is always read-only; exit code must be 0 even with findings.
    anyhow::ensure!(
        output.status.success(),
        "audit --supply exited non-zero:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The fixture has the bad package in at least one Cargo.lock blob.
    let blob_count = stdout
        .lines()
        .find(|l| l.contains("Lockfile blobs matching advisories:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    assert!(
        blob_count >= 1,
        "expected >= 1 lockfile blob matching advisories; found {blob_count}; report:\n{stdout}",
    );

    // Should recommend the supply fix.
    assert!(
        stdout.contains("git-scrub supply --execute"),
        "expected recommendation for 'git-scrub supply --execute'; got:\n{stdout}",
    );

    Ok(())
}
