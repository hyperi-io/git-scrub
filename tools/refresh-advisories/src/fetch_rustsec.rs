//  Project:      git-scrub
//  File:         tools/refresh-advisories/src/fetch_rustsec.rs
//  Purpose:      RustSec advisory-db fetcher.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! RustSec advisory-db fetcher.
//!
//! Clones <https://github.com/rustsec/advisory-db> (shallow, depth 1) and
//! parses the TOML front-matter at the top of each `.md` advisory file under
//! `crates/`.
//!
//! # Front-matter formats
//!
//! RustSec advisories use a fenced TOML block at the top of the file:
//!
//! ````text
//! ```toml
//! [advisory]
//! id = "RUSTSEC-2024-0001"
//! ...
//! ```
//! ````
//!
//! Older entries may use `---` delimiters. Both are supported.
//!
//! # Patched-range inversion (v1 limitation)
//!
//! The `[versions].patched` field describes the *fixed* version range.
//! git-scrub needs the *compromised* range (the inverse). In v1 we record
//! the raw patched range as-is inside the `notes` field and emit `*` as the
//! version when no patched range is listed. Accurate range inversion is
//! deferred to v1.x. Operators can override via `--config`.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::merge::NormalisedAdvisory;

/// Fetch advisories from the RustSec advisory-db.
///
/// Requires `git` on `PATH`. Clones into a temporary directory which is
/// cleaned up automatically on return.
///
/// # Errors
///
/// Returns an error if `git clone` fails or the filesystem cannot be read.
pub fn fetch() -> Result<Vec<NormalisedAdvisory>> {
    let tmp = tempfile::tempdir().context("creating temp dir")?;
    let dest = tmp.path();

    let status = std::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--quiet",
            "--",
            "https://github.com/rustsec/advisory-db",
            &dest.display().to_string(),
        ])
        .status()
        .context("invoking git clone")?;

    if !status.success() {
        anyhow::bail!("git clone of rustsec/advisory-db failed");
    }

    let crates_dir = dest.join("crates");
    if !crates_dir.is_dir() {
        warn!("crates/ directory not found in advisory-db; returning empty");
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(&crates_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        match parse_advisory_file(path) {
            Ok(Some(adv)) => out.push(adv),
            Ok(None) => {}
            Err(e) => debug!(?path, error = %e, "skipped advisory file"),
        }
    }
    Ok(out)
}

fn parse_advisory_file(path: &Path) -> Result<Option<NormalisedAdvisory>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let Some(toml_str) = extract_front_matter(&content) else {
        return Ok(None);
    };

    let parsed: toml::Value =
        toml::from_str(toml_str).with_context(|| format!("parsing TOML in {}", path.display()))?;

    let advisory = parsed.get("advisory").context("missing [advisory] table")?;

    let id = advisory
        .get("id")
        .and_then(|v| v.as_str())
        .context("missing advisory.id")?;
    let pkg_name = advisory
        .get("package")
        .and_then(|v| v.as_str())
        .context("missing advisory.package")?;
    let url = advisory
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // `[versions].patched` lists the FIX ranges. git-scrub's `versions:` field
    // means COMPROMISED ranges, so we invert each patched expression before
    // storing. Without inversion the matcher treats patched (safe) versions
    // as compromised and misses the actual vulnerable ones.
    let patched: Vec<String> = parsed
        .get("versions")
        .and_then(|v| v.get("patched"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let versions = if patched.is_empty() {
        vec!["*".to_string()]
    } else {
        invert_patched_ranges(&patched)
    };

    let raw_patched = parsed
        .get("versions")
        .and_then(|v| v.get("patched"))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    Ok(Some(NormalisedAdvisory {
        name: pkg_name.to_string(),
        ecosystem: "cargo".to_string(),
        versions,
        advisory_id: id.to_string(),
        url,
        notes: Some(format!(
            "Imported from RustSec (raw patched range: {raw_patched})"
        )),
    }))
}

/// Invert a list of "patched" SemVer ranges into a list of "compromised"
/// SemVer ranges.
///
/// RustSec advisories specify which versions are FIXED. git-scrub's matcher
/// wants COMPROMISED versions. The relationship is:
///
/// - `patched = [">= 1.2.3"]` → compromised = `< 1.2.3`
/// - `patched = [">= 1.2.3, < 2.0.0"]` → compromised = `< 1.2.3` (we keep
///   only the lower bound; the upper bound implies versions ≥ 2.0.0 may be
///   re-vulnerable, but RustSec advisories that span branches issue
///   separate `patched` entries for each branch)
/// - `patched = [">= 1.2.3", ">= 2.0.1"]` (multiple branches) →
///   compromised = `< 1.2.3` joined with `>= 2.0.0, < 2.0.1` etc.
///   For v1 we emit `< 1.2.3` from the first branch and trust that
///   maintainers re-emit advisories per branch.
///
/// Handled operators: `>=`, `>`. Comma-separated sub-clauses (multi-bound
/// ranges) are tolerated; we extract the LOWER bound and invert it.
/// Patched ranges we can't parse fall back to `"*"` (match all versions).
fn invert_patched_ranges(patched: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for range in patched {
        if let Some(inverted) = invert_single_range(range) {
            out.push(inverted);
        }
    }
    if out.is_empty() {
        // Couldn't invert any branch — record "*" so the operator at least
        // sees the advisory and can refine via `--config`.
        out.push("*".to_string());
    }
    out.sort();
    out.dedup();
    out
}

/// Invert a single comma-separated patched range. Picks the lower-bound
/// comparator and flips its operator.
///
/// - `">= 1.2.3"` → `"< 1.2.3"`
/// - `"> 1.2.3"` → `"<= 1.2.3"`
/// - `">= 1.2.3, < 2.0.0"` → `"< 1.2.3"` (lower bound only)
/// - `"< 1.2.3"` (rare; reverse range) → `None` (can't sensibly invert)
fn invert_single_range(range: &str) -> Option<String> {
    let lower_bound = range
        .split(',')
        .map(str::trim)
        .find(|clause| clause.starts_with(">=") || clause.starts_with('>'))?;
    if let Some(rest) = lower_bound.strip_prefix(">=") {
        Some(format!("< {}", rest.trim()))
    } else if let Some(rest) = lower_bound.strip_prefix('>') {
        Some(format!("<= {}", rest.trim()))
    } else {
        None
    }
}

/// Extract front-matter TOML from a RustSec advisory file.
///
/// Supports both the fenced ` ```toml ` style and `---` delimiters.
fn extract_front_matter(content: &str) -> Option<&str> {
    // Fenced ```toml block (current RustSec format)
    if let Some(rest) = content.strip_prefix("```toml\n")
        && let Some(end) = rest.find("\n```")
    {
        return Some(&rest[..end]);
    }
    // --- delimited (older format)
    if let Some(rest) = content.strip_prefix("---\n")
        && let Some(end) = rest.find("\n---\n")
    {
        return Some(&rest[..end]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_fenced_toml_block() {
        let content = "```toml\n[advisory]\nid = \"RUSTSEC-2024-0001\"\n```\n\nBody text.";
        let fm = extract_front_matter(content);
        assert!(fm.is_some());
        assert!(fm.unwrap().contains("RUSTSEC-2024-0001"));
    }

    #[test]
    fn extract_dashes_delimited() {
        let content = "---\n[advisory]\nid = \"RUSTSEC-2023-0001\"\n---\n\nBody.";
        let fm = extract_front_matter(content);
        assert!(fm.is_some());
        assert!(fm.unwrap().contains("RUSTSEC-2023-0001"));
    }

    #[test]
    fn extract_returns_none_for_plain_markdown() {
        let content = "# Just a heading\n\nNo front-matter here.";
        assert!(extract_front_matter(content).is_none());
    }

    #[test]
    fn parse_advisory_file_inverts_patched_range() {
        let tmp = tempfile::NamedTempFile::with_suffix(".md").unwrap();
        std::fs::write(
            tmp.path(),
            "```toml\n[advisory]\nid = \"RUSTSEC-2024-0099\"\npackage = \"test-crate\"\n\
             [versions]\npatched = [\">= 1.2.0\"]\n```\n\nAdvisory body.",
        )
        .unwrap();
        let adv = parse_advisory_file(tmp.path()).unwrap().unwrap();
        assert_eq!(adv.advisory_id, "RUSTSEC-2024-0099");
        assert_eq!(adv.name, "test-crate");
        assert_eq!(adv.ecosystem, "cargo");
        // `>= 1.2.0` (patched) inverts to `< 1.2.0` (compromised).
        assert_eq!(adv.versions, vec!["< 1.2.0"]);
    }

    #[test]
    fn parse_advisory_file_no_patched_range() {
        let tmp = tempfile::NamedTempFile::with_suffix(".md").unwrap();
        std::fs::write(
            tmp.path(),
            "```toml\n[advisory]\nid = \"RUSTSEC-2024-0100\"\npackage = \"vuln-crate\"\n```\n",
        )
        .unwrap();
        let adv = parse_advisory_file(tmp.path()).unwrap().unwrap();
        // No patched range = entire package compromised.
        assert_eq!(adv.versions, vec!["*"]);
    }

    #[test]
    fn invert_single_geq() {
        assert_eq!(invert_single_range(">= 1.2.3"), Some("< 1.2.3".to_string()));
    }

    #[test]
    fn invert_single_gt() {
        assert_eq!(invert_single_range("> 1.2.3"), Some("<= 1.2.3".to_string()));
    }

    #[test]
    fn invert_multi_clause_uses_lower_bound() {
        // Multi-clause range like ">= 1.2.3, < 2.0.0" inverts to just the
        // lower bound. The upper bound implies a branch-specific patch which
        // RustSec issues as a separate advisory entry per branch.
        assert_eq!(
            invert_single_range(">= 1.2.3, < 2.0.0"),
            Some("< 1.2.3".to_string()),
        );
    }

    #[test]
    fn invert_no_lower_bound_returns_none() {
        // Pure upper-bound range can't be sensibly inverted in v1.
        assert_eq!(invert_single_range("< 1.2.3"), None);
    }

    #[test]
    fn invert_patched_ranges_dedupes_and_sorts() {
        let inverted = invert_patched_ranges(&[
            ">= 2.0.1".to_string(),
            ">= 1.2.3".to_string(),
            ">= 1.2.3".to_string(),  // duplicate
        ]);
        assert_eq!(inverted, vec!["< 1.2.3", "< 2.0.1"]);
    }

    #[test]
    fn invert_unparseable_falls_back_to_star() {
        // If every patched clause is unparseable, fall back to `*` so the
        // operator at least sees the advisory and can refine.
        let inverted = invert_patched_ranges(&["random garbage".to_string()]);
        assert_eq!(inverted, vec!["*"]);
    }

    #[test]
    fn parse_advisory_file_no_front_matter() {
        let tmp = tempfile::NamedTempFile::with_suffix(".md").unwrap();
        std::fs::write(tmp.path(), "# No front matter\n\nJust content.").unwrap();
        let result = parse_advisory_file(tmp.path()).unwrap();
        assert!(result.is_none());
    }
}
