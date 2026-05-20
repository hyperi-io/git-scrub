//  Project:      git-scrub
//  File:         src/patterns/lockfile/cargo.rs
//  Purpose:      Cargo.lock LockfileRewriter (also: uv.lock, poetry.lock).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Cargo.lock / uv.lock / poetry.lock rewriter.
//!
//! All three formats are TOML with a top-level `[[package]]` array of
//! tables. Each entry has `name` and `version` at minimum. Stripping
//! a known-bad entry means removing the table from the array and
//! re-serialising; checksums and dependencies referencing the entry
//! are deliberately left intact (orphan references break the build —
//! the safety property we want).

use std::collections::HashSet;

use anyhow::Result;
use toml::Value;

use crate::patterns::LockfileRewriter;
use crate::patterns::supply::CompromisedPackage;
use crate::patterns::versions::Spec;

#[derive(Debug, Clone)]
struct Target {
    name: String,
    specs: Vec<Spec>,
}

/// Rewriter for TOML-array lockfiles (Cargo.lock, uv.lock, poetry.lock).
pub struct CargoLockRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl CargoLockRewriter {
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
        let basenames: HashSet<&'static str> = ["Cargo.lock", "uv.lock", "poetry.lock"]
            .into_iter()
            .collect();
        Ok(Self { basenames, targets })
    }

    fn entry_matches(&self, name: &str, version: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.name == name && t.specs.iter().any(|s| s.matches(version)))
    }
}

impl LockfileRewriter for CargoLockRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let s = std::str::from_utf8(content).ok()?;
        let mut doc: Value = toml::from_str(s).ok()?;
        let packages = doc.as_table_mut()?.get_mut("package")?.as_array_mut()?;
        let before = packages.len();
        packages.retain(|entry| {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                return true;
            };
            let Some(version) = entry.get("version").and_then(Value::as_str) else {
                return true;
            };
            !self.entry_matches(name, version)
        });
        if packages.len() == before {
            return None;
        }
        toml::to_string(&doc).ok().map(String::into_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::supply::{CompromisedPackage, PurgeTarget};

    fn sample_lockfile() -> &'static str {
        r#"
version = 4

[[package]]
name = "axios"
version = "1.6.1"

[[package]]
name = "innocent"
version = "0.1.0"

[[package]]
name = "another-innocent"
version = "2.3.4"
"#
    }

    fn pkg(name: &str, version: &str) -> CompromisedPackage {
        CompromisedPackage {
            name: name.to_string(),
            ecosystem: "cargo".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    #[test]
    fn applies_to_cargo_paths() {
        let r = CargoLockRewriter::new(&[]).unwrap();
        assert!(r.applies_to("Cargo.lock"));
        assert!(r.applies_to("crates/foo/Cargo.lock"));
        assert!(r.applies_to("uv.lock"));
        assert!(r.applies_to("poetry.lock"));
        assert!(!r.applies_to("Cargo.toml"));
    }

    #[test]
    fn strips_matching_entry() {
        let bad = pkg("axios", "1.6.1");
        let r = CargoLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_lockfile().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("name = \"axios\""));
        assert!(s.contains("name = \"innocent\""));
        assert!(s.contains("name = \"another-innocent\""));
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("nonexistent", "9.9.9");
        let r = CargoLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(sample_lockfile().as_bytes()).is_none());
    }

    #[test]
    fn strips_wildcard_version() {
        let bad = pkg("axios", "*");
        let r = CargoLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_lockfile().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("axios"));
    }

    #[test]
    fn preserves_version_when_range_doesnt_match() {
        let bad = pkg("axios", ">=2.0.0");
        let r = CargoLockRewriter::new(&[&bad]).unwrap();
        // sample axios is 1.6.1; range matches >=2.0.0 only.
        assert!(r.strip(sample_lockfile().as_bytes()).is_none());
    }

    #[test]
    fn malformed_toml_returns_none() {
        let bad = pkg("axios", "1.6.1");
        let r = CargoLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(b"not toml [[[").is_none());
    }
}
