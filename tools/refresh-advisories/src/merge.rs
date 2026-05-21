//  Project:      git-scrub
//  File:         tools/refresh-advisories/src/merge.rs
//  Purpose:      Merge advisories from multiple sources into a deduplicated set.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Merge advisories from multiple sources into a deduplicated set.

use serde::{Deserialize, Serialize};

/// Normalised advisory shape used internally before YAML emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalisedAdvisory {
    pub name: String,
    pub ecosystem: String,
    pub versions: Vec<String>,
    pub advisory_id: String,
    pub url: Option<String>,
    pub notes: Option<String>,
}

/// Merge advisories from N sources, deduplicating by (ecosystem, name).
///
/// When the same package appears in multiple sources, version ranges are
/// merged (concatenated, preserving distinct strings). Advisory IDs are
/// preserved as a list in YAML emit (one per source).
#[must_use]
pub fn merge(sources: Vec<Vec<NormalisedAdvisory>>) -> Vec<NormalisedAdvisory> {
    use std::collections::BTreeMap;
    let mut by_key: BTreeMap<(String, String), Vec<NormalisedAdvisory>> = BTreeMap::new();
    for source in sources {
        for adv in source {
            let key = (adv.ecosystem.clone(), adv.name.clone());
            by_key.entry(key).or_default().push(adv);
        }
    }
    let mut out: Vec<NormalisedAdvisory> = by_key
        .into_iter()
        .map(|((eco, name), advs)| {
            let mut versions: Vec<String> = advs
                .iter()
                .flat_map(|a| a.versions.iter().cloned())
                .collect();
            versions.sort();
            versions.dedup();
            let advisory_id = advs
                .iter()
                .map(|a| a.advisory_id.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let url = advs.iter().find_map(|a| a.url.clone());
            let notes = advs.iter().find_map(|a| a.notes.clone());
            NormalisedAdvisory {
                name,
                ecosystem: eco,
                versions,
                advisory_id,
                url,
                notes,
            }
        })
        .collect();
    // Stable sort by ecosystem, name
    out.sort_by(|a, b| {
        (a.ecosystem.as_str(), a.name.as_str()).cmp(&(b.ecosystem.as_str(), b.name.as_str()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adv(name: &str, ecosystem: &str, version: &str, id: &str) -> NormalisedAdvisory {
        NormalisedAdvisory {
            name: name.to_string(),
            ecosystem: ecosystem.to_string(),
            versions: vec![version.to_string()],
            advisory_id: id.to_string(),
            url: None,
            notes: None,
        }
    }

    #[test]
    fn merge_dedups_by_eco_and_name() {
        let a = vec![adv("axios", "npm", "1.6.1", "GHSA-1")];
        let b = vec![adv("axios", "npm", "1.6.2", "OSV-2")];
        let merged = merge(vec![a, b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].versions, vec!["1.6.1", "1.6.2"]);
        assert!(merged[0].advisory_id.contains("GHSA-1"));
        assert!(merged[0].advisory_id.contains("OSV-2"));
    }

    #[test]
    fn merge_preserves_distinct_packages() {
        let a = vec![
            adv("axios", "npm", "1.6.1", "GHSA-1"),
            adv("xinference", "pip", "2.6.0", "PYSEC-1"),
        ];
        let merged = merge(vec![a]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_sorts_deterministically() {
        let unsorted = vec![
            adv("zzz", "npm", "1.0", "id1"),
            adv("aaa", "cargo", "1.0", "id2"),
            adv("aaa", "npm", "1.0", "id3"),
        ];
        let merged = merge(vec![unsorted]);
        assert_eq!(merged[0].ecosystem, "cargo");
        assert_eq!(merged[1].name, "aaa");
        assert_eq!(merged[1].ecosystem, "npm");
        assert_eq!(merged[2].name, "zzz");
    }

    #[test]
    fn merge_empty_sources_returns_empty() {
        let merged = merge(vec![]);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_deduplicates_identical_versions() {
        let a = vec![adv("pkg", "cargo", "1.0.0", "RUSTSEC-1")];
        let b = vec![adv("pkg", "cargo", "1.0.0", "RUSTSEC-1")];
        let merged = merge(vec![a, b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].versions.len(), 1);
    }

    #[test]
    fn merge_preserves_url_from_first_source() {
        let mut a = adv("pkg", "cargo", "1.0.0", "A-1");
        a.url = Some("https://example.com/1".to_string());
        let mut b = adv("pkg", "cargo", "1.0.1", "A-2");
        b.url = Some("https://example.com/2".to_string());
        let merged = merge(vec![vec![a], vec![b]]);
        // First source URL is preserved
        assert_eq!(merged[0].url.as_deref(), Some("https://example.com/1"));
    }
}
