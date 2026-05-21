//  Project:      git-scrub
//  File:         tools/refresh-advisories/src/fetch_pypa.rs
//  Purpose:      PyPA advisory fetcher (v1 stub).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! PyPA advisory fetcher.
//!
//! **v1 stub — returns empty.**
//!
//! The full v1.x implementation will clone
//! <https://github.com/pypa/advisory-database> (shallow) and parse the YAML
//! advisory files under `vulns/`. Each file contains an `aliases` field
//! (OSV/CVE IDs), `package.name`, and `versions.specifier` giving the
//! affected range. These map directly to [`NormalisedAdvisory`] with
//! `ecosystem = "pip"`.
//!
//! Until that lands, this module returns an empty `Vec` and logs a warning.

use anyhow::Result;
use tracing::warn;

use crate::merge::NormalisedAdvisory;

/// Fetch advisories from the PyPA advisory database.
///
/// **v1 stub.** Returns an empty `Vec` and emits a warning. The full PyPA
/// integration is v1.x work.
///
/// # Errors
///
/// Currently infallible (always returns `Ok([])`). Will return network errors
/// once implemented.
#[allow(dead_code)]
pub fn fetch() -> Result<Vec<NormalisedAdvisory>> {
    warn!("fetch_pypa: stub — returning empty (full PyPA integration is v1.x work)");
    Ok(Vec::new())
}
