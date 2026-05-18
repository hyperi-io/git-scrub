//  Project:      git-scrub
//  File:         src/engine/fast_export.rs
//  Purpose:      Spawn `git fast-export` and expose its stdout as a byte stream.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `git fast-export` subprocess wrapper.
//!
//! Invokes `git -C <repo> fast-export --all --reencode=yes
//! --signed-tags=warn-strip --tag-of-filtered-object=rewrite` to emit a
//! deterministic, byte-exact stream of the repository's reachable objects.
//!
//! Cross-platform: identical invocation on Linux, macOS, and Windows (the
//! latter via Git for Windows providing `git.exe`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// `git fast-export` subprocess builder.
#[derive(Debug, Clone)]
pub struct Exporter {
    repo_dir: PathBuf,
}

impl Exporter {
    /// Create an exporter for the given repository working tree.
    #[must_use]
    pub fn new(repo_dir: &Path) -> Self {
        Self {
            repo_dir: repo_dir.to_path_buf(),
        }
    }

    /// Spawn the subprocess and return the [`Child`] with stdout piped.
    pub fn spawn(&self) -> std::io::Result<Child> {
        Command::new("git")
            .arg("-C")
            .arg(&self.repo_dir)
            .arg("fast-export")
            .arg("--all")
            .arg("--reencode=yes")
            .arg("--signed-tags=warn-strip")
            .arg("--tag-of-filtered-object=rewrite")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
    }
}
