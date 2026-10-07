//  Project:      git-scrub
//  File:         src/patterns/curate.rs
//  Purpose:      Schema for `ai curate` working-tree curation rules.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Working-tree curation rules.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top-level schema for `ai-curate.yaml`.
#[derive(Debug, Deserialize)]
pub struct CurateConfig {
    /// Named groups of paths to add to `.gitignore`.
    #[serde(default)]
    pub gitignore_additions: BTreeMap<String, Vec<String>>,
    /// Canonical policy files that should exist at the repo root.
    #[serde(default)]
    pub policy_files: Vec<String>,
    /// Text patterns to find-and-replace in tracked source files.
    #[serde(default)]
    pub reference_scrub: ReferenceScrub,
}

/// Container for the list of stray-reference scrub patterns.
#[derive(Debug, Default, Deserialize)]
pub struct ReferenceScrub {
    /// Ordered list of find/replace pairs.
    #[serde(default)]
    pub patterns: Vec<ScrubPattern>,
}

/// A single find/replace pair for stray-reference scrubbing.
#[derive(Debug, Deserialize)]
pub struct ScrubPattern {
    /// Literal substring to search for.
    pub find: String,
    /// Replacement string.
    pub replace: String,
}

impl CurateConfig {
    /// Flatten all gitignore groups into a single deduplicated, sorted list.
    #[must_use]
    pub fn all_gitignore_paths(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .gitignore_additions
            .values()
            .flat_map(|v| v.iter().map(String::as_str))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let yaml = r#"
gitignore_additions:
  ai_tool_dirs: [".claude/", ".cursor/"]
  state_files: ["STATE.md"]
policy_files: ["AI-TRAINING-POLICY.md"]
reference_scrub:
  patterns:
    - find: "Claude Code"
      replace: "local"
"#;
        let cfg: CurateConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(cfg.gitignore_additions.len(), 2);
        assert_eq!(cfg.policy_files, vec!["AI-TRAINING-POLICY.md"]);
        assert_eq!(cfg.reference_scrub.patterns.len(), 1);
        assert_eq!(cfg.reference_scrub.patterns[0].find, "Claude Code");
        assert_eq!(cfg.reference_scrub.patterns[0].replace, "local");
    }

    #[test]
    fn all_gitignore_paths_flattens_and_dedups() {
        let yaml = r#"
gitignore_additions:
  a: [".claude/", "TODO.md"]
  b: [".claude/", "STATE.md"]
"#;
        let cfg: CurateConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let paths = cfg.all_gitignore_paths();
        assert!(paths.contains(&".claude/"));
        assert!(paths.contains(&"TODO.md"));
        assert!(paths.contains(&"STATE.md"));
        // Dedup: .claude/ appears once despite being in two groups.
        assert_eq!(paths.iter().filter(|p| **p == ".claude/").count(), 1);
    }

    #[test]
    fn empty_config_is_valid() {
        let cfg: CurateConfig = serde_yaml_ng::from_str("{}").unwrap();
        assert!(cfg.gitignore_additions.is_empty());
        assert_eq!(cfg.policy_files, Vec::<String>::new());
        assert!(cfg.reference_scrub.patterns.is_empty());
        assert_eq!(cfg.all_gitignore_paths(), Vec::<&str>::new());
    }

    #[test]
    fn all_gitignore_paths_is_sorted() {
        let yaml = r#"
gitignore_additions:
  z_group: ["zzz.md", "aaa.md"]
  a_group: ["mmm.md"]
"#;
        let cfg: CurateConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let paths = cfg.all_gitignore_paths();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(
            paths, sorted,
            "all_gitignore_paths should return a sorted slice"
        );
    }
}
