//  Project:      git-scrub
//  File:         src/patterns/lockfile/pip.rs
//  Purpose:      Pipfile.lock LockfileRewriter.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Pipfile.lock rewriter.
//!
//! Strips entries from `default` and `develop` objects keyed by package
//! name. Version comparison strips the leading `==` (pipenv pins versions
//! with `"version": "==1.6.1"`).
//!
//! For v1, name matching is exact (case-sensitive). PEP-503 normalisation
//! (case-folding, `-`/`_`/`.` collapsing) is not applied — operators with
//! normalisation needs author multiple advisory entries.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use crate::patterns::LockfileRewriter;
use crate::patterns::supply::CompromisedPackage;
use crate::patterns::versions::Spec;

#[derive(Debug, Clone)]
struct Target {
    name: String,
    specs: Vec<Spec>,
}

/// Rewriter for Pipfile.lock files.
pub struct PipLockRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl PipLockRewriter {
    /// Build from a list of compromised packages.
    ///
    /// # Errors
    ///
    /// Returns an error if any version expression in the package list fails to parse.
    pub fn new(packages: &[&CompromisedPackage]) -> Result<Self> {
        let mut targets = Vec::with_capacity(packages.len());
        for p in packages {
            targets.push(Target {
                name: p.name.clone(),
                specs: p.version_specs()?,
            });
        }
        Ok(Self {
            basenames: ["Pipfile.lock"].into_iter().collect(),
            targets,
        })
    }

    fn matches(&self, name: &str, version: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.name == name && t.specs.iter().any(|s| s.matches(version)))
    }

    /// Strip pipenv's `==` operator from a version string.
    ///
    /// `"==1.6.1"` → `"1.6.1"`. Returns the input unchanged if no `==` prefix.
    fn normalise_version(raw: &str) -> &str {
        raw.strip_prefix("==").unwrap_or(raw)
    }
}

impl LockfileRewriter for PipLockRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let mut doc: Value = serde_json::from_slice(content).ok()?;
        let mut changed = false;

        for top_key in ["default", "develop"] {
            let Some(packages) = doc.get_mut(top_key).and_then(Value::as_object_mut) else {
                continue;
            };
            packages.retain(|name, value| {
                let raw_version = value.get("version").and_then(Value::as_str).unwrap_or("");
                let version = Self::normalise_version(raw_version);
                if self.matches(name, version) {
                    changed = true;
                    return false;
                }
                true
            });
        }

        if !changed {
            return None;
        }
        // Pretty-print for diffability.
        serde_json::to_vec_pretty(&doc).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::supply::{CompromisedPackage, PurgeTarget};

    fn pkg(name: &str, version: &str) -> CompromisedPackage {
        CompromisedPackage {
            name: name.to_string(),
            ecosystem: "pip".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    fn sample() -> &'static str {
        r#"{
    "_meta": {
        "hash": { "sha256": "deadbeef" },
        "pipfile-spec": 6,
        "requires": { "python_version": "3.11" }
    },
    "default": {
        "axios": {
            "hashes": ["sha256:dead"],
            "version": "==1.6.1"
        },
        "innocent-utils": {
            "hashes": ["sha256:alive"],
            "version": "==1.0.0"
        }
    },
    "develop": {
        "pytest": {
            "hashes": ["sha256:test"],
            "version": "==7.0.0"
        },
        "axios": {
            "hashes": ["sha256:dead2"],
            "version": "==1.6.1"
        }
    }
}"#
    }

    #[test]
    fn applies_to_pipfile_lock() {
        let r = PipLockRewriter::new(&[]).unwrap();
        assert!(r.applies_to("Pipfile.lock"));
        assert!(r.applies_to("subdir/Pipfile.lock"));
        assert!(!r.applies_to("Pipfile"));
        assert!(!r.applies_to("pyproject.toml"));
    }

    #[test]
    fn normalise_version_strips_equals() {
        assert_eq!(PipLockRewriter::normalise_version("==1.6.1"), "1.6.1");
        assert_eq!(PipLockRewriter::normalise_version("1.6.1"), "1.6.1");
    }

    #[test]
    fn strips_from_default() {
        let bad = pkg("axios", "1.6.1");
        let r = PipLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample().as_bytes()).expect("rewrite");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let default = parsed.get("default").unwrap().as_object().unwrap();
        assert!(!default.contains_key("axios"));
        assert!(default.contains_key("innocent-utils"));
    }

    #[test]
    fn strips_from_develop() {
        let bad = pkg("axios", "1.6.1");
        let r = PipLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample().as_bytes()).expect("rewrite");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let develop = parsed.get("develop").unwrap().as_object().unwrap();
        assert!(!develop.contains_key("axios"));
        assert!(develop.contains_key("pytest"));
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("nonexistent", "9.9.9");
        let r = PipLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(sample().as_bytes()).is_none());
    }

    #[test]
    fn malformed_returns_none() {
        let bad = pkg("axios", "1.6.1");
        let r = PipLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(b"not json").is_none());
    }

    #[test]
    fn version_range_match() {
        let bad = pkg("axios", ">=1.6.0, <1.7.0");
        let r = PipLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample().as_bytes()).expect("rewrite");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let default = parsed.get("default").unwrap().as_object().unwrap();
        assert!(!default.contains_key("axios"));
    }

    #[test]
    fn handles_missing_develop_section() {
        let content = r#"{
    "_meta": { "hash": { "sha256": "deadbeef" }, "pipfile-spec": 6, "requires": {} },
    "default": {
        "axios": { "hashes": ["sha256:dead"], "version": "==1.6.1" }
    }
}"#;
        let bad = pkg("axios", "1.6.1");
        let r = PipLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(content.as_bytes()).expect("rewrite");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let default = parsed.get("default").unwrap().as_object().unwrap();
        assert!(!default.contains_key("axios"));
    }
}
