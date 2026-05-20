//  Project:      git-scrub
//  File:         src/patterns/lockfile/npm.rs
//  Purpose:      package-lock.json / npm-shrinkwrap.json LockfileRewriter.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! npm lockfile rewriter.
//!
//! Handles both lockfile-version 1 (top-level `dependencies` keyed by
//! name, with nested `dependencies` for transitives) and v2/v3
//! (top-level `packages` keyed by `node_modules/<name>` path).
//! Scoped packages (`@scope/name`) are preserved as-is.

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

/// Rewriter for npm lockfiles (`package-lock.json`, `npm-shrinkwrap.json`).
pub struct NpmLockRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl NpmLockRewriter {
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
        let basenames: HashSet<&'static str> = ["package-lock.json", "npm-shrinkwrap.json"]
            .into_iter()
            .collect();
        Ok(Self { basenames, targets })
    }

    fn matches(&self, name: &str, version: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.name == name && t.specs.iter().any(|s| s.matches(version)))
    }

    /// Extract a package name from a v2/v3 `packages` key like
    /// `node_modules/foo` → `foo`, `node_modules/@scope/foo` → `@scope/foo`,
    /// `node_modules/foo/node_modules/bar` → `bar`.
    /// The root key `""` yields an empty string.
    fn name_from_v2_key(key: &str) -> &str {
        if let Some(idx) = key.rfind("node_modules/") {
            &key[idx + "node_modules/".len()..]
        } else {
            key
        }
    }

    /// Recursively scrub a v1 `dependencies` object.
    ///
    /// Returns `true` when anything was removed at any depth.
    fn scrub_v1_deps(&self, deps: &mut serde_json::Map<String, Value>) -> bool {
        let mut changed = false;
        deps.retain(|name, value| {
            let version = value.get("version").and_then(Value::as_str).unwrap_or("");
            if self.matches(name, version) {
                changed = true;
                return false;
            }
            true
        });
        // Recurse into surviving entries' nested `dependencies`.
        for (_name, value) in deps.iter_mut() {
            if let Some(inner) = value.get_mut("dependencies").and_then(Value::as_object_mut)
                && self.scrub_v1_deps(inner)
            {
                changed = true;
            }
        }
        changed
    }
}

impl LockfileRewriter for NpmLockRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let mut doc: Value = serde_json::from_slice(content).ok()?;
        let mut changed = false;

        // v2/v3: top-level `"packages"` object keyed by node_modules path.
        if let Some(packages) = doc.get_mut("packages").and_then(Value::as_object_mut) {
            packages.retain(|key, value| {
                if key.is_empty() {
                    return true; // root entry has no package name; preserve
                }
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| Self::name_from_v2_key(key));
                let version = value.get("version").and_then(Value::as_str).unwrap_or("");
                if self.matches(name, version) {
                    changed = true;
                    return false;
                }
                true
            });
        }

        // v1 (and v2 mixed): top-level `"dependencies"` object, recursively.
        if let Some(deps) = doc.get_mut("dependencies").and_then(Value::as_object_mut)
            && self.scrub_v1_deps(deps)
        {
            changed = true;
        }

        if !changed {
            return None;
        }
        // Pretty-print for diffability — npm itself emits 2-space indent.
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
            ecosystem: "npm".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    fn sample_v3() -> &'static str {
        // lockfile v2/v3 — top-level `packages` keyed by node_modules path
        r#"{
  "name": "myproj",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "myproj",
      "version": "1.0.0",
      "dependencies": {
        "axios": "1.6.1",
        "innocent-utils": "1.0.0"
      }
    },
    "node_modules/axios": {
      "version": "1.6.1",
      "resolved": "https://registry.npmjs.org/axios/-/axios-1.6.1.tgz",
      "integrity": "sha512-dead..."
    },
    "node_modules/innocent-utils": {
      "version": "1.0.0",
      "resolved": "https://registry.npmjs.org/innocent-utils/-/innocent-utils-1.0.0.tgz",
      "integrity": "sha512-alive..."
    },
    "node_modules/@types/node": {
      "version": "20.0.0",
      "resolved": "https://registry.npmjs.org/@types/node/-/node-20.0.0.tgz",
      "integrity": "sha512-types..."
    }
  }
}"#
    }

    fn sample_v1() -> &'static str {
        // lockfile v1 — top-level `dependencies` keyed by package name,
        // recursive for transitives
        r#"{
  "name": "myproj",
  "version": "1.0.0",
  "lockfileVersion": 1,
  "dependencies": {
    "axios": {
      "version": "1.6.1",
      "integrity": "sha1-dead..."
    },
    "innocent-utils": {
      "version": "1.0.0",
      "integrity": "sha1-alive...",
      "dependencies": {
        "axios": {
          "version": "1.6.1",
          "integrity": "sha1-transitive-dead..."
        },
        "deep-innocent": {
          "version": "0.1.0"
        }
      }
    }
  }
}"#
    }

    #[test]
    fn applies_to_npm_paths() {
        let r = NpmLockRewriter::new(&[]).unwrap();
        assert!(r.applies_to("package-lock.json"));
        assert!(r.applies_to("npm-shrinkwrap.json"));
        assert!(r.applies_to("packages/foo/package-lock.json"));
        assert!(!r.applies_to("package.json"));
        assert!(!r.applies_to("Cargo.lock"));
    }

    #[test]
    fn strips_v3_node_modules_entry() {
        let bad = pkg("axios", "1.6.1");
        let r = NpmLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_v3().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        // The "node_modules/axios" key should be gone.
        assert!(
            !s.contains("\"node_modules/axios\""),
            "v3 entry should be stripped\n{s}"
        );
        // Innocent and scoped packages remain.
        assert!(s.contains("innocent-utils"));
        assert!(s.contains("@types/node"));
    }

    #[test]
    fn strips_v1_top_level_dependency() {
        let bad = pkg("axios", "1.6.1");
        let r = NpmLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_v1().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        // innocent-utils is preserved (even though axios nested inside it is
        // also stripped — see next test for the recursive case).
        assert!(s.contains("innocent-utils"));
    }

    #[test]
    fn strips_v1_nested_transitive_dependency() {
        let bad = pkg("axios", "1.6.1");
        let r = NpmLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_v1().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        // ALL axios entries should be gone (top-level AND nested transitive).
        let count = s.matches("\"axios\"").count();
        assert_eq!(
            count, 0,
            "all axios entries (top + nested) should be stripped\n{s}"
        );
        // The innocent transitive should remain.
        assert!(s.contains("deep-innocent"));
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("nonexistent", "9.9.9");
        let r = NpmLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(sample_v3().as_bytes()).is_none());
        assert!(r.strip(sample_v1().as_bytes()).is_none());
    }

    #[test]
    fn name_from_v2_key_handles_scopes_and_nesting() {
        assert_eq!(NpmLockRewriter::name_from_v2_key("node_modules/foo"), "foo");
        assert_eq!(
            NpmLockRewriter::name_from_v2_key("node_modules/@scope/foo"),
            "@scope/foo"
        );
        assert_eq!(
            NpmLockRewriter::name_from_v2_key("node_modules/foo/node_modules/bar"),
            "bar"
        );
        assert_eq!(NpmLockRewriter::name_from_v2_key(""), "");
    }

    #[test]
    fn malformed_json_returns_none() {
        let bad = pkg("axios", "1.6.1");
        let r = NpmLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(b"not json {").is_none());
    }
}
