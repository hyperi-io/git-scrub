//  Project:      git-scrub
//  File:         tools/refresh-advisories/src/fetch_ghsa.rs
//  Purpose:      GHSA advisory fetcher (v1 stub).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! GHSA advisory fetcher.
//!
//! **v1 stub — returns empty.**
//!
//! The full v1.x implementation will use the GitHub Advisory Database GraphQL
//! API (`gh api graphql`) to query `securityVulnerabilities` and map each
//! entry to a [`NormalisedAdvisory`]. Requires a `GITHUB_TOKEN` with
//! `read:security_events` scope.
//!
//! Until that lands, this module returns an empty `Vec` and logs a warning
//! so maintainers know the GHSA data is absent from any generated snapshot.

use anyhow::Result;
use tracing::warn;

use crate::merge::NormalisedAdvisory;

/// Fetch advisories from the GitHub Advisory Database.
///
/// **v1 stub.** Returns an empty `Vec` and emits a warning. The full GHSA
/// integration is v1.x work.
///
/// # Errors
///
/// Currently infallible (always returns `Ok([])`). Will return network errors
/// once implemented.
#[allow(dead_code)]
pub fn fetch() -> Result<Vec<NormalisedAdvisory>> {
    warn!("fetch_ghsa: stub — returning empty (full GHSA integration is v1.x work)");
    Ok(Vec::new())
}
