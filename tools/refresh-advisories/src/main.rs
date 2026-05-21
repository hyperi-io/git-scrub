//  Project:      git-scrub
//  File:         tools/refresh-advisories/src/main.rs
//  Purpose:      Refresh config/patterns/supply-chain.yaml from upstream advisory feeds.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Refresh advisory snapshot.
//!
//! Pulls from public advisory sources (RustSec, GHSA, OSV, PyPA) and
//! merges into `config/patterns/supply-chain.yaml`. Maintainer runs
//! this before tagging a git-scrub release.
//!
//! # Usage
//!
//! ```text
//! cargo run -p refresh-advisories
//! ```
//!
//! Requires `git` on `PATH` (used to shallow-clone advisory repositories).
//! Network access is required; CI should NOT run this tool automatically.
//!
//! # Adding new sources
//!
//! Implement a `fetch()` function returning `Result<Vec<NormalisedAdvisory>>`
//! in a new `fetch_<name>.rs` module, then add the call and merge here.
//!
//! # v1 status
//!
//! RustSec: fully implemented.
//! GHSA, OSV, PyPA: stubs returning empty (v1.x work).

mod emit;
mod fetch_ghsa;
mod fetch_osv;
mod fetch_pypa;
mod fetch_rustsec;
mod merge;

use anyhow::{Context, Result};
use tracing::info;

pub use merge::NormalisedAdvisory;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let workspace_root = workspace_root()?;
    let output_path = workspace_root.join("config/patterns/supply-chain.yaml");

    info!("fetching from RustSec...");
    let rustsec = fetch_rustsec::fetch().context("rustsec fetch")?;
    info!("rustsec: {} advisories", rustsec.len());

    info!("fetching from GHSA...");
    let ghsa = fetch_ghsa::fetch().context("ghsa fetch")?;
    info!("ghsa: {} advisories", ghsa.len());

    info!("fetching from OSV...");
    let osv = fetch_osv::fetch().context("osv fetch")?;
    info!("osv: {} advisories", osv.len());

    info!("fetching from PyPA...");
    let pypa = fetch_pypa::fetch().context("pypa fetch")?;
    info!("pypa: {} advisories", pypa.len());

    let merged = merge::merge(vec![rustsec, ghsa, osv, pypa]);
    info!("merged: {} unique compromised entries", merged.len());

    emit::write_yaml(&output_path, &merged)
        .with_context(|| format!("writing {}", output_path.display()))?;
    info!("wrote {}", output_path.display());

    Ok(())
}

/// Resolve the workspace root from `CARGO_MANIFEST_DIR`.
///
/// The workspace member lives at `<workspace_root>/tools/refresh-advisories/`,
/// so walking up two parent levels yields the workspace root.
///
/// # Errors
///
/// Returns an error if `CARGO_MANIFEST_DIR` is not set or the path has no
/// grandparent.
fn workspace_root() -> Result<std::path::PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR not set")?;
    let path = std::path::PathBuf::from(manifest_dir);
    let workspace = path
        .parent()
        .and_then(|p| p.parent())
        .context("manifest dir has no grandparent")?
        .to_path_buf();
    Ok(workspace)
}
