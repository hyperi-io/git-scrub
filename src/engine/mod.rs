//  Project:      git-scrub
//  File:         src/engine/mod.rs
//  Purpose:      History-rewrite engine: fast-export → transform → fast-import.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Stream-based history rewrite engine.
//!
//! Pipeline:
//!
//! ```text
//! git fast-export --all --reencode=yes
//!   -> Rust transform (message rewriting + path filtering)
//!   -> git fast-import --force
//! ```
//!
//! See [`fast_export`], [`transform`], and [`fast_import`] for the per-stage
//! detail. The top-level [`run`] function wires them together for the common
//! "apply patterns to the current repo" case.

pub mod backup;
pub mod blob_cache;
pub mod fast_export;
pub mod fast_import;
pub mod transform;

pub use blob_cache::BlobCache;

use std::path::Path;

use thiserror::Error;
use tracing::info;

use crate::patterns::{AttributionRewriter, BlobRewriter, FileMatcher, LockfileRewriter};

/// Errors raised by the engine pipeline.
#[derive(Debug, Error)]
pub enum EngineError {
    /// fast-export subprocess failed to start or exited with non-zero.
    #[error("git fast-export failed: {0}")]
    Export(#[source] std::io::Error),

    /// fast-import subprocess failed to start or exited with non-zero.
    #[error("git fast-import failed: {0}")]
    Import(#[source] std::io::Error),

    /// Stream transform stage encountered malformed fast-export input.
    #[error("fast-export stream malformed at offset {offset}: {message}")]
    StreamParse {
        /// Byte offset where parsing gave up.
        offset: usize,
        /// What went wrong.
        message: String,
    },

    /// Generic I/O error connecting stages.
    #[error("engine I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Statistics returned from a single engine pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EngineStats {
    /// Number of commit records seen.
    pub commits_seen: usize,
    /// Number of commit records whose message was rewritten.
    pub commits_rewritten: usize,
    /// Number of file operations (M/D/R/C) dropped from commits.
    pub file_ops_dropped: usize,
    /// Number of blob records seen.
    pub blobs_seen: usize,
    /// Number of blob records whose content was rewritten (`spill text`).
    pub blobs_rewritten: usize,
}

/// Run the full pipeline against a git repository.
///
/// `repo_dir` is the working tree root. Every pattern source is optional;
/// passing `None` for all four is a no-op that returns early without
/// invoking `git`.
pub fn run(
    repo_dir: &Path,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
    lockfiles: Option<&[Box<dyn LockfileRewriter>]>,
) -> Result<EngineStats, EngineError> {
    let want_attribution = attribution.is_some_and(|a| !a.is_empty());
    let want_files = files.is_some_and(|f| !f.is_empty());
    let want_blob = blob.is_some_and(|b| !b.is_empty());
    let want_lockfiles = lockfiles.is_some_and(|lfs| !lfs.is_empty());
    if !want_attribution && !want_files && !want_blob && !want_lockfiles {
        info!("engine: nothing to rewrite (empty pattern sets)");
        return Ok(EngineStats::default());
    }

    let exporter = fast_export::Exporter::new(repo_dir);
    let importer = fast_import::Importer::new(repo_dir);

    let mut export_child = exporter.spawn()?;
    let mut import_child = importer.spawn()?;

    let export_stdout = export_child
        .stdout
        .take()
        .ok_or_else(|| EngineError::Export(std::io::Error::other("fast-export stdout missing")))?;
    let import_stdin = import_child
        .stdin
        .take()
        .ok_or_else(|| EngineError::Import(std::io::Error::other("fast-import stdin missing")))?;

    let stats = transform::run_stream(
        export_stdout,
        import_stdin,
        attribution,
        files,
        blob,
        lockfiles,
    )?;

    let export_status = export_child.wait().map_err(EngineError::Export)?;
    let import_status = import_child.wait().map_err(EngineError::Import)?;

    if !export_status.success() {
        return Err(EngineError::Export(std::io::Error::other(format!(
            "git fast-export exited with status {export_status}"
        ))));
    }
    if !import_status.success() {
        return Err(EngineError::Import(std::io::Error::other(format!(
            "git fast-import exited with status {import_status}"
        ))));
    }

    Ok(stats)
}
