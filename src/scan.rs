//  Project:      git-scrub
//  File:         src/scan.rs
//  Purpose:      Read-only scan of history to count what WOULD be scrubbed.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Read-only history scan.
//!
//! Drives a `fast-export` stream through the same transform stage as the
//! real rewrite, but discards the output. The returned statistics drive
//! the dry-run plan.

use std::io;
use std::path::Path;

use crate::engine::transform;
use crate::engine::{EngineError, EngineStats, fast_export};
use crate::patterns::{AttributionRewriter, BlobRewriter, FileMatcher, LockfileRewriter};

/// Run a read-only scan against `repo_dir` and return the statistics that
/// would result from a real rewrite.
pub fn scan(
    repo_dir: &Path,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
    lockfiles: Option<&[Box<dyn LockfileRewriter>]>,
) -> Result<EngineStats, EngineError> {
    let exporter = fast_export::Exporter::new(repo_dir);
    let mut child = exporter.spawn().map_err(EngineError::Export)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| EngineError::Export(io::Error::other("fast-export stdout missing")))?;

    let stats = transform::run_stream(stdout, io::sink(), attribution, files, blob, lockfiles)?;

    let exit = child.wait().map_err(EngineError::Export)?;
    if !exit.success() {
        return Err(EngineError::Export(io::Error::other(format!(
            "git fast-export exited with status {exit}"
        ))));
    }
    Ok(stats)
}
