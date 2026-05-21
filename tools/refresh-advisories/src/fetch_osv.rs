//  Project:      git-scrub
//  File:         tools/refresh-advisories/src/fetch_osv.rs
//  Purpose:      OSV advisory fetcher (v1 stub).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! OSV advisory fetcher.
//!
//! **v1 stub — returns empty.**
//!
//! The full v1.x implementation will query <https://osv.dev/v1/query> for
//! supply-chain compromise advisories across all ecosystems. The OSV API
//! accepts a JSON body specifying ecosystem and package name, and returns
//! affected version ranges in a normalised schema that maps cleanly to
//! [`NormalisedAdvisory`].
//!
//! Until that lands, this module returns an empty `Vec` and logs a warning.

use anyhow::Result;
use tracing::warn;

use crate::merge::NormalisedAdvisory;

/// Fetch advisories from the OSV vulnerability database.
///
/// **v1 stub.** Returns an empty `Vec` and emits a warning. The full OSV
/// integration is v1.x work.
///
/// # Errors
///
/// Currently infallible (always returns `Ok([])`). Will return network errors
/// once implemented.
#[allow(dead_code)]
pub fn fetch() -> Result<Vec<NormalisedAdvisory>> {
    warn!("fetch_osv: stub — returning empty (full OSV integration is v1.x work)");
    Ok(Vec::new())
}
