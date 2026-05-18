//  Project:      git-scrub
//  File:         src/engine/backup.rs
//  Purpose:      Mirror-clone the target repo as a pre-rewrite backup.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Pre-rewrite backup via `git clone --mirror`.
//!
//! A mirror clone is the only safe rollback point: it captures every ref
//! (including tags, notes, remotes) and a complete object database. The
//! caller can `git push --mirror` the backup to restore.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Errors raised by the backup step.
#[derive(Debug, Error)]
pub enum BackupError {
    /// Could not invoke `git clone --mirror`.
    #[error("git clone --mirror failed: {0}")]
    Clone(#[source] std::io::Error),

    /// Subprocess exited with non-zero status.
    #[error("git clone --mirror exited with status {0}")]
    BadStatus(std::process::ExitStatus),

    /// Filesystem I/O error preparing the backup directory.
    #[error("backup directory I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Run `git clone --mirror <repo_dir> <dest>` and return the destination path.
pub fn mirror_clone(repo_dir: &Path, dest: &Path) -> Result<PathBuf, BackupError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let status = Command::new("git")
        .arg("clone")
        .arg("--mirror")
        .arg(repo_dir)
        .arg(dest)
        .status()
        .map_err(BackupError::Clone)?;

    if !status.success() {
        return Err(BackupError::BadStatus(status));
    }
    Ok(dest.to_path_buf())
}
