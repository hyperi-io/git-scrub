//  Project:      git-scrub
//  File:         tests/common/mod.rs
//  Purpose:      Shared test helpers — mint tmp git repos with seeded content.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Test helpers for minting throwaway git repos with known content.
//!
//! Per HyperI's no-mocks policy, integration tests drive a real `git`
//! binary against a real working tree under `tempfile::TempDir`.

#![allow(dead_code)] // Helpers used selectively by integration tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A minted temporary git repository.
pub struct TempRepo {
    /// Holds the temp directory alive for the test's lifetime.
    pub dir: TempDir,
}

impl TempRepo {
    /// Initialise a new git repository in a fresh temp dir.
    #[must_use]
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        run_git(dir.path(), &["init", "--quiet", "--initial-branch=main"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(dir.path(), &["config", "user.email", "test@example.com"]);
        run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
        Self { dir }
    }

    /// Root of the working tree.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Write a file at `rel_path` containing `content`, creating parent dirs.
    pub fn write_file(&self, rel_path: &str, content: &str) -> PathBuf {
        let full = self.path().join(rel_path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parents");
        }
        std::fs::write(&full, content).expect("write file");
        full
    }

    /// `git add <rel_path>` then `git commit -m <message>`.
    pub fn add_commit(&self, rel_path: &str, message: &str) {
        run_git(self.path(), &["add", rel_path]);
        run_git(self.path(), &["commit", "--quiet", "-m", message]);
    }

    /// Add all and commit with the given message (allowing multi-paragraph bodies).
    pub fn add_all_commit(&self, message: &str) {
        run_git(self.path(), &["add", "-A"]);
        run_git(self.path(), &["commit", "--quiet", "-m", message]);
    }

    /// Return `git log --all --pretty=format:%B` joined with newlines — useful for
    /// asserting that attribution lines are gone after rewrite.
    #[must_use]
    pub fn log_messages(&self) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .arg("log")
            .arg("--all")
            .arg("--pretty=format:%B%n----")
            .output()
            .expect("git log");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Return `git log --all --raw` — lets tests assert what paths history references.
    #[must_use]
    pub fn log_raw(&self) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .arg("log")
            .arg("--all")
            .arg("--raw")
            .output()
            .expect("git log --raw");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Default for TempRepo {
    fn default() -> Self {
        Self::new()
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed in {cwd:?}");
}
