//  Project:      git-scrub
//  File:         src/patterns/versions.rs
//  Purpose:      Version expression matching for supply chain advisories.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Version expression matcher.
//!
//! Supply-chain advisories specify which versions of a package are
//! compromised. This module turns version expression strings into a
//! `Spec` enum and provides a uniform `matches(version: &str) -> bool`
//! check across ecosystems.

use anyhow::{Context, Result};

/// A version expression. Parsed from the YAML `versions:` list.
#[derive(Debug, Clone)]
pub enum Spec {
    /// `"*"` — matches every version.
    Any,
    /// `"1.6.1"` — exact string match.
    Exact(String),
    /// `SemVer` comparator string like `">=1.6.0, <1.7.0"`.
    Range(semver::VersionReq),
}

impl Spec {
    /// Parse a single expression string.
    ///
    /// # Errors
    /// Returns an error if a non-`*`, non-exact expression fails
    /// `SemVer` parsing.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed == "*" {
            return Ok(Self::Any);
        }
        if !needs_semver(trimmed) {
            return Ok(Self::Exact(trimmed.to_string()));
        }
        let req = semver::VersionReq::parse(trimmed)
            .with_context(|| format!("invalid version expression: {trimmed}"))?;
        Ok(Self::Range(req))
    }

    /// Whether `version` matches this spec.
    ///
    /// For `Range`, the input is parsed as `SemVer`; if parsing fails
    /// (non-`SemVer` ecosystems), the comparison falls through to `false`.
    #[must_use]
    pub fn matches(&self, version: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(s) => s == version,
            Self::Range(req) => semver::Version::parse(version).is_ok_and(|v| req.matches(&v)),
        }
    }
}

fn needs_semver(input: &str) -> bool {
    // Any expression containing a comparator operator or comma is a range/comparator.
    input.contains(['>', '<', '=', '^', '~', ','])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_matches_everything() {
        let s = Spec::parse("*").unwrap();
        assert!(s.matches("1.0.0"));
        assert!(s.matches("garbage"));
    }

    #[test]
    fn exact_matches_only_its_string() {
        let s = Spec::parse("1.6.1").unwrap();
        assert!(s.matches("1.6.1"));
        assert!(!s.matches("1.6.0"));
        assert!(!s.matches("1.6.2"));
    }

    #[test]
    fn range_matches_semver_intervals() {
        let s = Spec::parse(">=1.6.0, <1.7.0").unwrap();
        assert!(s.matches("1.6.0"));
        assert!(s.matches("1.6.9"));
        assert!(!s.matches("1.5.99"));
        assert!(!s.matches("1.7.0"));
    }

    #[test]
    fn range_with_caret() {
        let s = Spec::parse("^1.6").unwrap();
        assert!(s.matches("1.6.0"));
        assert!(s.matches("1.9.0"));
        assert!(!s.matches("2.0.0"));
    }

    #[test]
    fn range_against_non_semver_returns_false() {
        let s = Spec::parse(">=1.6.0").unwrap();
        assert!(!s.matches("v1.2.3-not-semver"));
    }

    #[test]
    fn invalid_range_errors() {
        assert!(Spec::parse(">=not-a-version").is_err());
    }
}
