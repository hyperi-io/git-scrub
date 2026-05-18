//  Project:      git-scrub
//  File:         src/engine/fast_import.rs
//  Purpose:      Spawn `git fast-import` to consume the transformed stream.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `git fast-import` subprocess wrapper.
//!
//! Invokes `git -C <repo> fast-import --force --quiet`. The `--force` flag
//! is required because we are rewriting the existing object database in
//! place; without it git refuses to update refs that don't fast-forward.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// `git fast-import` subprocess builder.
#[derive(Debug, Clone)]
pub struct Importer {
    repo_dir: PathBuf,
}

impl Importer {
    /// Create an importer for the given repository working tree.
    #[must_use]
    pub fn new(repo_dir: &Path) -> Self {
        Self {
            repo_dir: repo_dir.to_path_buf(),
        }
    }

    /// Spawn the subprocess and return the [`Child`] with stdin piped.
    pub fn spawn(&self) -> std::io::Result<Child> {
        Command::new("git")
            .arg("-C")
            .arg(&self.repo_dir)
            .arg("fast-import")
            .arg("--force")
            .arg("--quiet")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
    }
}
