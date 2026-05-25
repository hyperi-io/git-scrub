//  Project:      git-scrub
//  File:         src/patterns/supply.rs
//  Purpose:      Supply chain advisory schema + matcher expansion.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Supply chain advisory schema.
//!
//! YAML at `config/patterns/supply-chain.yaml` (or operator-supplied
//! `--config`) deserialises into `SupplyConfig`. The config drives
//! `LockfileRewriter` dispatch (which lockfile parsers fire on which
//! paths) and records vendored path templates for future use.
//!
//! **v1 ecosystem coverage:** Cargo, npm, pnpm, yarn (classic v1),
//! bun (text lockfile), uv, pip (Pipfile.lock), poetry, go (go.sum),
//! composer — ten ecosystems total.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::patterns::versions::Spec;

/// Top-level config.
#[derive(Debug, Deserialize)]
pub struct SupplyConfig {
    pub generated_at: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub compromised: Vec<CompromisedPackage>,
    #[serde(default)]
    pub lockfiles: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub vendored_paths: BTreeMap<String, Vec<String>>,
}

/// A single compromised package entry.
#[derive(Debug, Deserialize)]
pub struct CompromisedPackage {
    pub name: String,
    pub ecosystem: String,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
    #[serde(default)]
    pub purge_targets: Vec<PurgeTarget>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Advisory reference (informational; surfaces in runbook).
#[derive(Debug, Deserialize)]
pub struct Advisory {
    pub id: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// What to purge for a compromised package.
#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PurgeTarget {
    LockfileEntry,
    VendoredSource,
}

impl CompromisedPackage {
    /// Parse the `versions` strings into `Spec` values, returning the
    /// first parse failure encountered.
    pub fn version_specs(&self) -> anyhow::Result<Vec<Spec>> {
        self.versions.iter().map(|v| Spec::parse(v)).collect()
    }

    /// Whether the package's `purge_targets` requests lockfile rewrite.
    #[must_use]
    pub fn purge_lockfile(&self) -> bool {
        self.purge_targets.contains(&PurgeTarget::LockfileEntry)
    }

    /// Whether the package's `purge_targets` requests vendored purge.
    #[must_use]
    pub fn purge_vendored(&self) -> bool {
        self.purge_targets.contains(&PurgeTarget::VendoredSource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let yaml = r#"
compromised:
  - name: axios
    ecosystem: npm
    versions: ["1.6.1"]
    purge_targets: [lockfile_entry]
"#;
        let cfg: SupplyConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(cfg.compromised.len(), 1);
        assert_eq!(cfg.compromised[0].name, "axios");
        assert!(cfg.compromised[0].purge_lockfile());
        assert!(!cfg.compromised[0].purge_vendored());
    }

    #[test]
    fn parses_full_config_with_lockfile_map() {
        let yaml = r#"
generated_at: "2026-05-19T00:00:00Z"
sources:
  - "https://github.com/rustsec/advisory-db"
compromised:
  - name: sui-execution-cut
    ecosystem: cargo
    versions: ["*"]
    advisories:
      - id: RUSTSEC-2024-0123
        url: https://rustsec.org/advisories/RUSTSEC-2024-0123
    purge_targets: [lockfile_entry, vendored_source]
    notes: Typosquat.
lockfiles:
  cargo: ["Cargo.lock"]
  npm: ["package-lock.json"]
vendored_paths:
  cargo: ["vendor/{name}/"]
"#;
        let cfg: SupplyConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(cfg.generated_at.as_deref(), Some("2026-05-19T00:00:00Z"));
        assert_eq!(
            cfg.lockfiles.get("cargo").unwrap(),
            &vec!["Cargo.lock".to_string()]
        );
        assert_eq!(cfg.compromised[0].advisories[0].id, "RUSTSEC-2024-0123");
        assert!(cfg.compromised[0].purge_vendored());
    }
}
