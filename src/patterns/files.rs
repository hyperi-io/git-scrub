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
                add_with_nested_expansion(&mut purge_builder, "purge", tool, p)?;
            }
        }

        for extra in &opts.include_extra {
            add_with_nested_expansion(&mut purge_builder, "include_extra", "<cli>", extra)?;
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

/// Add a glob pattern to `builder`, also adding a `**/<pattern>` sibling so
/// the pattern matches anywhere in the path hierarchy, not just at the root.
///
/// Expansion rules (mirrors gitignore semantics):
/// - If `pattern` already starts with `**/`, it already matches anywhere --
///   only the original pattern is added.
/// - If `pattern` starts with `/`, the leading `/` is stripped and only the
///   root-anchored form is added (explicit root-only request).
/// - Otherwise both `pattern` and `**/<pattern>` are added so the pattern
///   fires whether the directory appears at the repo root or nested under a
///   sub-project (e.g. `.claude/**` matches both `.claude/foo` and
///   `my-app/.claude/foo`).
fn add_with_nested_expansion(
    builder: &mut GlobSetBuilder,
    section: &'static str,
    tool: &str,
    pattern: &str,
) -> Result<(), FileError> {
    if pattern.starts_with("**/") {
        // Already anchored to match anywhere -- just add as-is.
        builder.add(compile_glob(section, tool, pattern)?);
    } else if let Some(root_only) = pattern.strip_prefix('/') {
        // Explicit root-only anchor -- strip the slash and add once.
        builder.add(compile_glob(section, tool, root_only)?);
    } else {
        // Add the original pattern (matches at repo root) plus the
        // `**/` prefix form (matches anywhere in the tree).
        builder.add(compile_glob(section, tool, pattern)?);
        let nested = format!("**/{pattern}");
        builder.add(compile_glob(section, tool, &nested)?);
    }
    Ok(())
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

    // Tests for nested-path matching using the bundled YAML.

    #[test]
    fn bundled_matches_nested_claude_directory_under_subapp() {
        let yaml = include_str!("../../config/patterns/ai-files.yaml");
        let cfg = parse_yaml(yaml).unwrap();
        let m = FileMatcher::new(&cfg, &FileMatcherOptions::default()).unwrap();
        // Root-level match must still work.
        assert!(
            m.should_purge(".claude/notes.md"),
            ".claude/ at root must match"
        );
        // Nested match: the fixture seeds paths like ai-residue-app/.claude/notes.md.
        assert!(
            m.should_purge("ai-residue-app/.claude/notes.md"),
            ".claude/ nested under a sub-app must also match"
        );
        assert!(
            m.should_purge("crates/foo/.cursor/state.json"),
            ".cursor/ nested under a sub-crate must match"
        );
    }

    #[test]
    fn bundled_does_not_match_files_with_tool_name_in_filename_only() {
        let yaml = include_str!("../../config/patterns/ai-files.yaml");
        let cfg = parse_yaml(yaml).unwrap();
        let m = FileMatcher::new(&cfg, &FileMatcherOptions::default()).unwrap();
        // A file whose name contains "claude" but is NOT under a .claude/ directory
        // must NOT be purged by the directory-glob patterns.
        assert!(
            !m.should_purge("docs/about-claude.md"),
            "docs/about-claude.md is not under .claude/ -- must not match"
        );
        assert!(
            !m.should_purge("src/cursor_helper.rs"),
            "src/cursor_helper.rs is not under .cursor/ -- must not match"
        );
    }

    #[test]
    fn root_only_pattern_does_not_match_nested() {
        // Patterns starting with '/' should be root-only (no expansion).
        let yaml = r"
purge:
  test:
    - '/.only-at-root/**'
exclude_by_default: []
";
        let cfg = parse_yaml(yaml).unwrap();
        let m = FileMatcher::new(&cfg, &FileMatcherOptions::default()).unwrap();
        assert!(
            m.should_purge(".only-at-root/file.txt"),
            "root-only pattern with leading / must match at root (slash stripped)"
        );
        assert!(
            !m.should_purge("nested/.only-at-root/file.txt"),
            "root-only pattern must NOT match when nested"
        );
    }
}
