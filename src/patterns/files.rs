//  Project:      git-scrub
//  File:         src/patterns/files.rs
//  Purpose:      File-path pattern matching (purge / exclude-by-default).
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! File-path pattern matcher.
//!
//! Patterns are globs matched against the **forward-slash form** of paths
//! as they appear in the `git fast-export` stream. Git's internal path
//! representation is always `/`-separated on every platform, so a pattern
//! like `.claude/**` matches identically on Linux, macOS, and Windows.

use std::collections::BTreeMap;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// YAML schema for the file pattern library.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileConfig {
    /// Map of tool name → list of glob patterns to purge.
    #[serde(default)]
    pub purge: BTreeMap<String, Vec<String>>,

    /// Path globs that are preserved by default (state-file family, etc).
    /// Operator must explicitly `--include <name>` to purge these.
    #[serde(default)]
    pub exclude_by_default: Vec<String>,
}

/// Errors returned by file-pattern loading and matching.
#[derive(Debug, Error)]
pub enum FileError {
    /// A glob pattern in the YAML failed to compile.
    #[error("invalid glob in {section} for tool '{tool}': {pattern}: {source}")]
    InvalidGlob {
        /// Which YAML section the pattern belongs to.
        section: &'static str,
        /// Tool name (or `exclude_by_default`).
        tool: String,
        /// The offending glob pattern.
        pattern: String,
        /// Underlying glob error.
        #[source]
        source: globset::Error,
    },
}

/// Compiled glob matcher — given a path, says whether it should be purged.
#[derive(Debug)]
pub struct FileMatcher {
    purge: GlobSet,
    excluded: GlobSet,
}

/// Inputs to [`FileMatcher::new`] for fine-grained set adjustment.
#[derive(Debug, Clone, Default)]
pub struct FileMatcherOptions {
    /// Tool names to skip entirely (do not contribute to purge set).
    pub exclude_tools: Vec<String>,
    /// Extra paths/globs to add to the purge set on top of the YAML defaults.
    pub include_extra: Vec<String>,
    /// Extra paths/globs to subtract from the purge set.
    pub exclude_extra: Vec<String>,
    /// If `true`, override the exclude-by-default protection for these names.
    /// Names listed here are purged even if matched by `exclude_by_default`.
    pub force_include: Vec<String>,
}

impl FileMatcher {
    /// Build a matcher from a parsed [`FileConfig`] and runtime options.
    pub fn new(config: &FileConfig, opts: &FileMatcherOptions) -> Result<Self, FileError> {
        let mut purge_builder = GlobSetBuilder::new();

        for (tool, pats) in &config.purge {
            if opts.exclude_tools.iter().any(|t| t == tool) {
                continue;
            }
            for p in pats {
                let glob = compile_glob("purge", tool, p)?;
                purge_builder.add(glob);
            }
        }

        for extra in &opts.include_extra {
            let glob = compile_glob("include_extra", "<cli>", extra)?;
            purge_builder.add(glob);
        }

        let purge = purge_builder
            .build()
            .map_err(|source| FileError::InvalidGlob {
                section: "purge",
                tool: "<set>".into(),
                pattern: "<glob-set>".into(),
                source,
            })?;

        let mut excluded_builder = GlobSetBuilder::new();
        for entry in &config.exclude_by_default {
            if opts.force_include.iter().any(|t| t == entry) {
                continue;
            }
            let glob = compile_glob("exclude_by_default", "<config>", entry)?;
            excluded_builder.add(glob);
        }
        for extra in &opts.exclude_extra {
            let glob = compile_glob("exclude_extra", "<cli>", extra)?;
            excluded_builder.add(glob);
        }
        let excluded = excluded_builder
            .build()
            .map_err(|source| FileError::InvalidGlob {
                section: "exclude_by_default",
                tool: "<set>".into(),
                pattern: "<glob-set>".into(),
                source,
            })?;

        Ok(Self { purge, excluded })
    }

    /// Returns `true` if the matcher has no purge patterns at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.purge.is_empty()
    }

    /// Decide whether the given forward-slash path should be purged.
    ///
    /// `path` MUST use `/` separators (git's canonical form). Callers
    /// reading OS-native paths must convert first.
    #[must_use]
    pub fn should_purge(&self, path: &str) -> bool {
        if self.excluded.is_match(path) {
            return false;
        }
        self.purge.is_match(path)
    }
}

fn compile_glob(section: &'static str, tool: &str, pattern: &str) -> Result<Glob, FileError> {
    Glob::new(pattern).map_err(|source| FileError::InvalidGlob {
        section,
        tool: tool.to_string(),
        pattern: pattern.to_string(),
        source,
    })
}

/// Parse YAML bytes into a [`FileConfig`].
pub fn parse_yaml(yaml: &str) -> Result<FileConfig, serde_yaml_ng::Error> {
    serde_yaml_ng::from_str(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> FileConfig {
        let yaml = r"
purge:
  claude:
    - '.claude/**'
    - '.claude-output/**'
  cursor:
    - '.cursor/**'
    - '.cursorrules'
exclude_by_default:
  - 'CLAUDE.md'
  - 'STATE.md'
";
        parse_yaml(yaml).expect("fixture parses")
    }

    #[test]
    fn purges_claude_dir() {
        let m = FileMatcher::new(&fixture(), &FileMatcherOptions::default()).unwrap();
        assert!(m.should_purge(".claude/instructions.md"));
        assert!(m.should_purge(".claude/sub/dir/file.txt"));
    }

    #[test]
    fn preserves_state_file_family_by_default() {
        let m = FileMatcher::new(&fixture(), &FileMatcherOptions::default()).unwrap();
        assert!(!m.should_purge("CLAUDE.md"));
        assert!(!m.should_purge("STATE.md"));
    }

    #[test]
    fn force_include_overrides_exclude() {
        let opts = FileMatcherOptions {
            force_include: vec!["CLAUDE.md".into()],
            ..Default::default()
        };
        // CLAUDE.md isn't in purge — force_include only removes the protection,
        // doesn't add it to purge. So adding it via include_extra:
        let opts = FileMatcherOptions {
            include_extra: vec!["CLAUDE.md".into()],
            ..opts
        };
        let m = FileMatcher::new(&fixture(), &opts).unwrap();
        assert!(m.should_purge("CLAUDE.md"));
    }

    #[test]
    fn exclude_tools_skips_them() {
        let opts = FileMatcherOptions {
            exclude_tools: vec!["claude".into()],
            ..Default::default()
        };
        let m = FileMatcher::new(&fixture(), &opts).unwrap();
        assert!(!m.should_purge(".claude/instructions.md"));
        assert!(m.should_purge(".cursor/rules.json"));
    }

    #[test]
    fn unrelated_paths_pass_through() {
        let m = FileMatcher::new(&fixture(), &FileMatcherOptions::default()).unwrap();
        assert!(!m.should_purge("src/main.rs"));
        assert!(!m.should_purge("Cargo.toml"));
        assert!(!m.should_purge("docs/README.md"));
    }

    #[test]
    fn cursor_rules_file_at_root() {
        let m = FileMatcher::new(&fixture(), &FileMatcherOptions::default()).unwrap();
        assert!(m.should_purge(".cursorrules"));
    }

    #[test]
    fn invalid_glob_surfaces_error() {
        let mut cfg = FileConfig::default();
        cfg.purge
            .insert("bad".to_string(), vec!["[unclosed".to_string()]);
        let err = FileMatcher::new(&cfg, &FileMatcherOptions::default()).unwrap_err();
        assert!(matches!(err, FileError::InvalidGlob { .. }));
    }
}
