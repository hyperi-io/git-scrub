//  Project:      git-scrub
//  File:         src/patterns/lockfile/pnpm.rs
//  Purpose:      pnpm-lock.yaml LockfileRewriter.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! pnpm lockfile rewriter.
//!
//! Handles pnpm-lock.yaml lockfileVersion 6.0+ shape (keys under
//! top-level `packages:` are `<name>@<version>` strings). Scoped names
//! (`@scope/name@version`) are split at the rightmost `@`.
//!
//! Older lockfileVersion < 6.0 (key form `/name/version`) is NOT
//! supported by v1 — operators on legacy pnpm should upgrade or use
//! the lockfile-only manual workflow.

use std::collections::HashSet;

use anyhow::Result;

use crate::patterns::LockfileRewriter;
use crate::patterns::supply::CompromisedPackage;
use crate::patterns::versions::Spec;

#[derive(Debug, Clone)]
struct Target {
    name: String,
    specs: Vec<Spec>,
}

/// Rewriter for pnpm lockfiles (`pnpm-lock.yaml`).
pub struct PnpmLockRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl PnpmLockRewriter {
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
            basenames: ["pnpm-lock.yaml"].into_iter().collect(),
            targets,
        })
    }

    fn matches(&self, name: &str, version: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.name == name && t.specs.iter().any(|s| s.matches(version)))
    }

    /// Parse a pnpm package key into `(name, version)`.
    ///
    /// Handles scoped names: `@scope/name@version` → `(@scope/name, version)`.
    /// Peer-suffixed keys like `axios@1.6.1(react@18.0.0)` have the parenthesised
    /// suffix stripped before splitting.
    ///
    /// Returns `None` for bare scope-only keys with no version (e.g. `@scope/name`).
    fn parse_key(key: &str) -> Option<(&str, &str)> {
        // Strip peer-suffix `(...)` if present.
        let core = match key.find('(') {
            Some(idx) => &key[..idx],
            None => key,
        };
        // Split at the rightmost `@` to separate name from version.
        let split = core.rfind('@')?;
        if split == 0 {
            // Key starts with `@` at position 0 — that's the scope marker, not the
            // version separator. There is no version component.
            return None;
        }
        Some((&core[..split], &core[split + 1..]))
    }
}

impl LockfileRewriter for PnpmLockRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let mut doc: serde_yaml_ng::Value = serde_yaml_ng::from_slice(content).ok()?;
        let mut changed = false;

        // Strip from `packages:` and `snapshots:` (both are `name@version` keyed maps).
        for top_key in ["packages", "snapshots"] {
            let Some(map) = doc.get_mut(top_key).and_then(|v| v.as_mapping_mut()) else {
                continue;
            };
            let keys_to_remove: Vec<serde_yaml_ng::Value> = map
                .keys()
                .filter_map(|k| {
                    let key_str = k.as_str()?;
                    let (name, version) = Self::parse_key(key_str)?;
                    if self.matches(name, version) {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect();
            for k in keys_to_remove {
                map.remove(&k);
                changed = true;
            }
        }

        // Strip from `importers.*.{dependencies,devDependencies,optionalDependencies}`.
        if let Some(importers) = doc.get_mut("importers").and_then(|v| v.as_mapping_mut()) {
            for (_importer_key, importer_value) in importers.iter_mut() {
                let Some(importer_map) = importer_value.as_mapping_mut() else {
                    continue;
                };
                for dep_section in ["dependencies", "devDependencies", "optionalDependencies"] {
                    let section_key = serde_yaml_ng::Value::String(dep_section.to_string());
                    let Some(deps) = importer_map
                        .get_mut(&section_key)
                        .and_then(|v| v.as_mapping_mut())
                    else {
                        continue;
                    };
                    let to_remove: Vec<serde_yaml_ng::Value> = deps
                        .iter()
                        .filter_map(|(k, v)| {
                            let name = k.as_str()?;
                            // Newer pnpm: value is a map `{specifier: ..., version: ...}`.
                            // Older inline form: value is a plain version string.
                            let version_raw = v
                                .get("version")
                                .and_then(|vv| vv.as_str())
                                .or_else(|| v.as_str())
                                .unwrap_or("");
                            // Strip peer-suffix from the version field too.
                            let version_core = match version_raw.find('(') {
                                Some(idx) => &version_raw[..idx],
                                None => version_raw,
                            };
                            if self.matches(name, version_core) {
                                Some(k.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                    for k in to_remove {
                        deps.remove(&k);
                        changed = true;
                    }
                }
            }
        }

        if !changed {
            return None;
        }
        serde_yaml_ng::to_string(&doc).ok().map(String::into_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::supply::{CompromisedPackage, PurgeTarget};

    fn pkg(name: &str, version: &str) -> CompromisedPackage {
        CompromisedPackage {
            name: name.to_string(),
            ecosystem: "pnpm".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    fn sample_v9() -> &'static str {
        // lockfileVersion 9.0 — has importers, packages, snapshots
        "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    dependencies:\n      axios:\n        specifier: ^1.6.0\n        version: 1.6.1\n      innocent-utils:\n        specifier: ^1.0.0\n        version: 1.0.0\n\npackages:\n  axios@1.6.1:\n    resolution:\n      integrity: sha512-dead\n\n  innocent-utils@1.0.0:\n    resolution:\n      integrity: sha512-alive\n\n  '@types/node@20.0.0':\n    resolution:\n      integrity: sha512-types\n\nsnapshots:\n  axios@1.6.1: {}\n  innocent-utils@1.0.0: {}\n  '@types/node@20.0.0': {}\n"
    }

    #[test]
    fn applies_to_pnpm_path() {
        let r = PnpmLockRewriter::new(&[]).unwrap();
        assert!(r.applies_to("pnpm-lock.yaml"));
        assert!(r.applies_to("packages/foo/pnpm-lock.yaml"));
        assert!(!r.applies_to("package-lock.json"));
        assert!(!r.applies_to("Cargo.lock"));
    }

    #[test]
    fn parse_key_simple() {
        assert_eq!(
            PnpmLockRewriter::parse_key("axios@1.6.1"),
            Some(("axios", "1.6.1"))
        );
    }

    #[test]
    fn parse_key_scoped() {
        assert_eq!(
            PnpmLockRewriter::parse_key("@types/node@20.0.0"),
            Some(("@types/node", "20.0.0"))
        );
    }

    #[test]
    fn parse_key_with_peer_suffix() {
        assert_eq!(
            PnpmLockRewriter::parse_key("axios@1.6.1(react@18.0.0)"),
            Some(("axios", "1.6.1"))
        );
    }

    #[test]
    fn strips_from_packages_snapshots_and_importers() {
        let bad = pkg("axios", "1.6.1");
        let r = PnpmLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_v9().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            !s.contains("axios@1.6.1"),
            "axios entry should be gone\n{s}"
        );
        assert!(
            !s.contains("axios:\n") && !s.contains("axios:"),
            "axios importer dep should be gone\n{s}"
        );
        assert!(
            s.contains("innocent-utils"),
            "innocent-utils should remain\n{s}"
        );
        assert!(s.contains("@types/node"), "@types/node should remain\n{s}");
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("nonexistent", "9.9.9");
        let r = PnpmLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(sample_v9().as_bytes()).is_none());
    }

    #[test]
    fn malformed_yaml_returns_none() {
        let bad = pkg("axios", "1.6.1");
        let r = PnpmLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(b"not yaml: : :").is_none());
    }

    #[test]
    fn no_packages_rewriters_compiles() {
        let r = PnpmLockRewriter::new(&[]).unwrap();
        // A lockfile with no matching packages should return None.
        assert!(r.strip(sample_v9().as_bytes()).is_none());
    }
}
