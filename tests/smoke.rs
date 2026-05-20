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
