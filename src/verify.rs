//  Project:      git-scrub
//  File:         src/verify.rs
//  Purpose:      Post-rewrite verification: re-scan history, assert zero matches.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Post-rewrite verification.
//!
//! Runs the same scan pass [`crate::scan::scan`] performs, but as an
//! assertion: any non-zero count is a hard fail. The rewrite must leave
//! zero matches behind.

use std::path::Path;

use thiserror::Error;

use crate::engine::{EngineError, EngineStats};
use crate::patterns::{AttributionRewriter, BlobRewriter, FileMatcher};
use crate::scan;

/// Errors raised by post-rewrite verification.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// Re-scan after rewrite still found pattern matches.
    #[error(
        "verification failed: {commits} commit message(s), {files} file op(s), \
         and {blobs} blob(s) still match scrub patterns after rewrite"
    )]
    StillMatching {
        /// Number of commits whose message still matches an attribution pattern.
        commits: usize,
        /// Number of file ops still referencing a purge path.
        files: usize,
        /// Number of blobs whose content still contains a `spill` pattern.
        blobs: usize,
    },

    /// Engine error during the verify scan.
    #[error(transparent)]
    Engine(#[from] EngineError),
}

/// Run a verify pass and return Ok only if zero matches remain.
pub fn run(
    repo_dir: &Path,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
) -> Result<EngineStats, VerifyError> {
    let stats = scan::scan(repo_dir, attribution, files, blob)?;
    if stats.commits_rewritten > 0 || stats.file_ops_dropped > 0 || stats.blobs_rewritten > 0 {
        return Err(VerifyError::StillMatching {
            commits: stats.commits_rewritten,
            files: stats.file_ops_dropped,
            blobs: stats.blobs_rewritten,
        });
    }
    Ok(stats)
}
