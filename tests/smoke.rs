//  Project:      git-scrub
//  File:         tests/smoke.rs
//  Purpose:      Startup smoke test (mandatory).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Startup smoke test.
//!
//! Boots the binary with `--help` and asserts it exits successfully. Catches
//! `clap` derive-macro regressions, panics in `OnceLock` init, or missing
//! embedded assets at startup.

use std::process::Command;

/// Locate the binary built by Cargo for this test session.
fn bin_path() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> is set by cargo for integration tests in the same crate.
    let p = env!("CARGO_BIN_EXE_git-scrub");
    std::path::PathBuf::from(p)
}

#[test]
fn help_runs_without_panic() {
    let out = Command::new(bin_path())
        .arg("--help")
        .output()
        .expect("spawn binary");
    assert!(
        out.status.success(),
        "git-scrub --help exited non-zero: {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("git-scrub"),
        "help should mention the binary name"
    );
    assert!(
        stdout.contains("ai"),
        "help should advertise the ai subcommand"
    );
}

#[test]
fn version_runs_without_panic() {
    let out = Command::new(bin_path())
        .arg("--version")
        .output()
        .expect("spawn binary");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("git-scrub"));
}

#[test]
fn ai_help_runs() {
    let out = Command::new(bin_path())
        .args(["ai", "--help"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "git-scrub ai --help failed: {:?}",
        out.status
    );
}

#[test]
fn patterns_dump_runs_with_embedded_fallback() {
    // No repo required, no override path — exercises the embedded YAML path.
    let out = Command::new(bin_path())
        .args(["ai", "patterns"])
        .output()
        .expect("spawn binary");
    assert!(
        out.status.success(),
        "git-scrub ai patterns exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("attribution"));
    assert!(stdout.contains("files"));
    assert!(stdout.contains("trailers"));
    assert!(stdout.contains("purge"));
}

#[test]
fn supply_patterns_dump_runs() {
    let out = Command::new(bin_path())
        .args(["supply", "patterns"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "git-scrub supply patterns exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("compromised:") || stdout.contains("lockfiles:"),
        "expected YAML keys in dump output:\n{stdout}",
    );
}

#[test]
fn supply_help_runs() {
    let out = Command::new(bin_path())
        .args(["supply", "--help"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "git-scrub supply --help exited non-zero"
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("composite"), "expected 'composite' in help");
    assert!(stdout.contains("lockfiles"), "expected 'lockfiles' in help");
    assert!(stdout.contains("packages"), "expected 'packages' in help");
    assert!(
        stdout.contains("advisories"),
        "expected 'advisories' in help"
    );
    assert!(stdout.contains("patterns"), "expected 'patterns' in help");
}

#[test]
fn clean_help_runs() {
    let out = Command::new(bin_path())
        .args(["clean", "--help"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "git-scrub clean --help exited non-zero: {:?}",
        out.status
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("--ai"), "expected '--ai' in help output");
    assert!(
        stdout.contains("--spill-paths"),
        "expected '--spill-paths' in help output"
    );
    assert!(
        stdout.contains("--supply"),
        "expected '--supply' in help output"
    );
}

#[test]
fn audit_help_runs() {
    let out = Command::new(bin_path())
        .args(["audit", "--help"])
        .output()
        .expect("run");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    for flag in ["--ai", "--supply", "--curate", "--all", "--report"] {
        assert!(
            stdout.contains(flag),
            "missing {flag} in audit --help output"
        );
    }
}

#[test]
fn audit_against_empty_repo_succeeds_and_prints_report() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir)
        .status()
        .expect("git init");
    std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-qm",
            "init",
        ])
        .current_dir(dir)
        .status()
        .expect("commit");

    let out = Command::new(bin_path())
        .args(["-C", &dir.display().to_string(), "audit"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("# git-scrub audit report"),
        "expected report header: {stdout}"
    );
    assert!(
        stdout.contains("## AI residue (history)"),
        "expected AI section: {stdout}"
    );
    assert!(
        stdout.contains("## Supply chain (history)"),
        "expected supply section: {stdout}"
    );
    assert!(
        stdout.contains("## Working-tree curation"),
        "expected curate section: {stdout}"
    );
    assert!(
        stdout.contains("## Summary"),
        "expected summary section: {stdout}"
    );
}

#[test]
fn audit_scope_flag_limits_sections() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir)
        .status()
        .expect("git init");
    std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-qm",
            "init",
        ])
        .current_dir(dir)
        .status()
        .expect("commit");

    let out = Command::new(bin_path())
        .args(["-C", &dir.display().to_string(), "audit", "--curate"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("## Working-tree curation"),
        "expected curate section: {stdout}"
    );
    assert!(
        !stdout.contains("## AI residue (history)"),
        "should NOT contain AI section when --curate only: {stdout}"
    );
    assert!(
        !stdout.contains("## Supply chain (history)"),
        "should NOT contain supply section when --curate only: {stdout}"
    );
}

#[test]
fn clean_with_no_targets_errors() {
    // No --ai, --spill-paths, or --supply supplied → must exit non-zero
    // with a helpful error message.
    let out = Command::new(bin_path())
        .args(["clean"])
        .output()
        .expect("run");
    assert!(
        !out.status.success(),
        "expected non-zero exit when no transformers activated"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("no transformers")
            || stderr.contains("--ai")
            || stderr.contains("--supply"),
        "expected helpful error message, got: {stderr}",
    );
}
