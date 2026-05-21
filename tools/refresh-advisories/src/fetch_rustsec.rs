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

    // `[versions].patched` lists the fixed version ranges. We record them as-is
    // in v1; accurate inversion to the compromised range is v1.x work.
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
        patched.clone()
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
    fn parse_advisory_file_from_tempfile() {
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
        assert_eq!(adv.versions, vec![">= 1.2.0"]);
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
        assert_eq!(adv.versions, vec!["*"]);
    }

    #[test]
    fn parse_advisory_file_no_front_matter() {
        let tmp = tempfile::NamedTempFile::with_suffix(".md").unwrap();
        std::fs::write(tmp.path(), "# No front matter\n\nJust content.").unwrap();
        let result = parse_advisory_file(tmp.path()).unwrap();
        assert!(result.is_none());
    }
}
