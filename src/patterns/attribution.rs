//  Project:      git-scrub
//  File:         src/patterns/attribution.rs
//  Purpose:      Commit-message attribution-trailer pattern matching and rewriting.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Attribution-pattern rewriter.
//!
//! Parses `config/patterns/ai-attribution.yaml`, compiles its regexes, and
//! applies them to commit-message bytes. Matching lines are removed; the
//! resulting message has its blank-line runs collapsed and trailing
//! whitespace stripped (controlled by the YAML `cleanup` block).

use std::collections::BTreeMap;

use regex::bytes::RegexSet;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// YAML schema for the attribution pattern library.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AttributionConfig {
    /// Map of tool name → list of trailer regexes (e.g. `Co-authored-by: …`).
    #[serde(default)]
    pub trailers: BTreeMap<String, Vec<String>>,

    /// Map of tool name → list of footer regexes (e.g. `Generated with Claude Code`).
    #[serde(default)]
    pub footers: BTreeMap<String, Vec<String>>,

    /// Post-removal cleanup directives.
    #[serde(default)]
    pub cleanup: CleanupConfig,
}

/// Post-removal cleanup applied after matching lines are dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// Collapse runs of 2+ blank lines into a single blank line.
    #[serde(default = "default_true")]
    pub collapse_blank_runs: bool,
    /// Trim trailing whitespace from every line.
    #[serde(default = "default_true")]
    pub strip_trailing_whitespace: bool,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            collapse_blank_runs: true,
            strip_trailing_whitespace: true,
        }
    }
}

const fn default_true() -> bool {
    true
}

/// Errors returned by attribution loading and matching.
#[derive(Debug, Error)]
pub enum AttributionError {
    /// A regex inside the YAML failed to compile.
    #[error("invalid regex in {section} for tool '{tool}': {pattern}: {source}")]
    InvalidRegex {
        /// Which YAML section the pattern belongs to (`trailers` or `footers`).
        section: &'static str,
        /// Tool name (`claude`, `cursor`, etc.).
        tool: String,
        /// The offending pattern string.
        pattern: String,
        /// Underlying regex error.
        #[source]
        source: regex::Error,
    },
}

/// Compiled attribution rewriter — match against and rewrite a commit message.
#[derive(Debug)]
pub struct AttributionRewriter {
    /// All compiled patterns (trailers + footers) as a single `RegexSet` for speed.
    set: RegexSet,
    cleanup: CleanupConfig,
}

impl AttributionRewriter {
    /// Build a rewriter from a parsed [`AttributionConfig`], filtering out
    /// tools listed in `exclude_tools`.
    pub fn new(
        config: &AttributionConfig,
        exclude_tools: &[String],
    ) -> Result<Self, AttributionError> {
        let mut patterns: Vec<String> = Vec::new();
        for (tool, pats) in &config.trailers {
            if exclude_tools.iter().any(|t| t == tool) {
                continue;
            }
            for p in pats {
                Self::validate_pattern("trailers", tool, p)?;
                patterns.push(p.clone());
            }
        }
        for (tool, pats) in &config.footers {
            if exclude_tools.iter().any(|t| t == tool) {
                continue;
            }
            for p in pats {
                Self::validate_pattern("footers", tool, p)?;
                patterns.push(p.clone());
            }
        }

        let set = RegexSet::new(&patterns).map_err(|e| AttributionError::InvalidRegex {
            section: "combined",
            tool: "set".into(),
            pattern: "<regex-set>".into(),
            source: e,
        })?;

        Ok(Self {
            set,
            cleanup: config.cleanup.clone(),
        })
    }

    fn validate_pattern(
        section: &'static str,
        tool: &str,
        pattern: &str,
    ) -> Result<(), AttributionError> {
        regex::bytes::Regex::new(pattern).map_err(|source| AttributionError::InvalidRegex {
            section,
            tool: tool.to_string(),
            pattern: pattern.to_string(),
            source,
        })?;
        Ok(())
    }

    /// Returns `true` if the rewriter has no patterns to match — `apply` is a no-op.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Returns `true` if the line matches any compiled pattern.
    #[must_use]
    pub fn line_matches(&self, line: &[u8]) -> bool {
        self.set.is_match(line)
    }

    /// Apply the rewriter to a full commit message (bytes in, bytes out).
    ///
    /// Operates on raw bytes — never decodes UTF-8. Commit messages may
    /// contain non-UTF-8 sequences and the fast-export stream is
    /// byte-exact.
    #[must_use]
    pub fn apply(&self, message: &[u8]) -> Vec<u8> {
        self.apply_with_drop_count(message).0
    }

    /// Like [`Self::apply`] but also returns the number of attribution lines
    /// matched and dropped (separate from any cosmetic cleanup).
    ///
    /// Useful for audit/scan: a drop count of zero means no AI residue was
    /// detected, even if cleanup transformations changed whitespace.
    #[must_use]
    pub fn apply_with_drop_count(&self, message: &[u8]) -> (Vec<u8>, usize) {
        let mut kept_lines: Vec<&[u8]> = Vec::new();
        let mut dropped: usize = 0;
        for line in split_lines(message) {
            let probe: &[u8] = if self.cleanup.strip_trailing_whitespace {
                trim_trailing_ws(line)
            } else {
                line
            };
            if self.set.is_match(probe) {
                dropped += 1;
            } else if self.cleanup.strip_trailing_whitespace {
                kept_lines.push(probe);
            } else {
                kept_lines.push(line);
            }
        }

        if self.cleanup.collapse_blank_runs {
            kept_lines = collapse_blank_runs(&kept_lines);
        }

        let mut out = Vec::with_capacity(message.len());
        for (i, line) in kept_lines.iter().enumerate() {
            if i > 0 {
                out.push(b'\n');
            }
            out.extend_from_slice(line);
        }
        (out, dropped)
    }
}

fn split_lines(message: &[u8]) -> Vec<&[u8]> {
    if message.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (i, &b) in message.iter().enumerate() {
        if b == b'\n' {
            lines.push(&message[start..i]);
            start = i + 1;
        }
    }
    if start <= message.len() {
        lines.push(&message[start..]);
    }
    lines
}

fn trim_trailing_ws(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 {
        let b = line[end - 1];
        if b == b' ' || b == b'\t' || b == b'\r' {
            end -= 1;
        } else {
            break;
        }
    }
    &line[..end]
}

fn collapse_blank_runs<'a>(lines: &[&'a [u8]]) -> Vec<&'a [u8]> {
    let mut out = Vec::with_capacity(lines.len());
    let mut prev_blank = false;
    for line in lines {
        let blank = line.iter().all(|&b| b == b' ' || b == b'\t');
        if blank && prev_blank {
            continue;
        }
        prev_blank = blank;
        out.push(*line);
    }
    while out
        .last()
        .is_some_and(|l| l.iter().all(|&b| b == b' ' || b == b'\t'))
    {
        out.pop();
    }
    out
}

/// Parse YAML bytes into an [`AttributionConfig`].
pub fn parse_yaml(yaml: &str) -> Result<AttributionConfig, serde_yaml_ng::Error> {
    serde_yaml_ng::from_str(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> AttributionConfig {
        let yaml = r"
trailers:
  claude:
    - '^Co-Authored-By:\s*Claude\s*<noreply@anthropic\.com>\s*$'
  cursor:
    - '^Co-authored-by:\s*Cursor\b.*$'
footers:
  claude:
    - '^Generated with Claude Code\s*$'
cleanup:
  collapse_blank_runs: true
  strip_trailing_whitespace: true
";
        parse_yaml(yaml).expect("fixture must parse")
    }

    #[test]
    fn matches_claude_trailer() {
        let r = AttributionRewriter::new(&fixture(), &[]).unwrap();
        assert!(r.line_matches(b"Co-Authored-By: Claude <noreply@anthropic.com>"));
        assert!(!r.line_matches(b"Co-Authored-By: Real Human <human@example.com>"));
    }

    #[test]
    fn drops_attribution_lines() {
        let r = AttributionRewriter::new(&fixture(), &[]).unwrap();
        let msg = b"\
Add feature

Implements the thing.

Co-Authored-By: Claude <noreply@anthropic.com>
Co-authored-by: Cursor <cursoragent@cursor.com>
Generated with Claude Code

";
        let out = r.apply(msg);
        let out_str = String::from_utf8_lossy(&out);
        assert!(!out_str.contains("Claude"));
        assert!(!out_str.contains("Cursor"));
        assert!(out_str.contains("Add feature"));
        assert!(out_str.contains("Implements the thing."));
    }

    #[test]
    fn preserves_human_coauthor_lines() {
        let r = AttributionRewriter::new(&fixture(), &[]).unwrap();
        let msg = b"\
Refactor

Co-Authored-By: Real Human <human@example.com>
";
        let out = r.apply(msg);
        assert!(String::from_utf8_lossy(&out).contains("Real Human"));
    }

    #[test]
    fn exclude_tools_skips_them() {
        let r = AttributionRewriter::new(&fixture(), &["claude".to_string()]).unwrap();
        assert!(!r.line_matches(b"Co-Authored-By: Claude <noreply@anthropic.com>"));
        assert!(r.line_matches(b"Co-authored-by: Cursor <cursoragent@cursor.com>"));
    }

    #[test]
    fn invalid_regex_surfaces_error() {
        let mut cfg = AttributionConfig::default();
        cfg.trailers
            .insert("bad".to_string(), vec!["[unclosed".to_string()]);
        let err = AttributionRewriter::new(&cfg, &[]).unwrap_err();
        assert!(matches!(err, AttributionError::InvalidRegex { .. }));
    }

    #[test]
    fn empty_message_returns_empty() {
        let r = AttributionRewriter::new(&fixture(), &[]).unwrap();
        assert_eq!(r.apply(b""), b"");
    }

    #[test]
    fn collapses_blank_runs() {
        let r = AttributionRewriter::new(&fixture(), &[]).unwrap();
        let msg = b"\
Add feature


Implements the thing.



Co-Authored-By: Claude <noreply@anthropic.com>
";
        let out = r.apply(msg);
        let s = String::from_utf8_lossy(&out);
        assert!(
            !s.contains("\n\n\n"),
            "should not contain triple newlines: {s:?}"
        );
    }

    // Tests against the bundled YAML to verify case-insensitive matching.

    #[test]
    fn bundled_matches_uppercase_co_authored_by() {
        let yaml = include_str!("../../config/patterns/ai-attribution.yaml");
        let cfg = parse_yaml(yaml).unwrap();
        let r = AttributionRewriter::new(&cfg, &[]).unwrap();
        let input = b"feat: thing\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n";
        let (out, dropped) = r.apply_with_drop_count(input);
        assert!(
            dropped >= 1,
            "Co-Authored-By (GitHub default form) must be dropped; got dropped={dropped}, out={}",
            String::from_utf8_lossy(&out)
        );
        let out_str = String::from_utf8_lossy(&out);
        assert!(
            !out_str.contains("Co-Authored-By: Claude"),
            "uppercase trailer must not survive in output"
        );
    }

    #[test]
    fn bundled_matches_mixed_case_co_authored_by() {
        let yaml = include_str!("../../config/patterns/ai-attribution.yaml");
        let cfg = parse_yaml(yaml).unwrap();
        let r = AttributionRewriter::new(&cfg, &[]).unwrap();
        // lowercase + SHOUTCASE both seeded as attribution
        let input =
            b"feat: thing\n\nco-authored-by: Cursor <cursoragent@cursor.com>\nCO-AUTHORED-BY: Claude <noreply@anthropic.com>\n";
        let (_, dropped) = r.apply_with_drop_count(input);
        assert_eq!(
            dropped, 2,
            "both lowercase co-authored-by and SHOUTCASE CO-AUTHORED-BY must match"
        );
    }

    #[test]
    fn bundled_matches_uppercase_codex_trailer() {
        // The fixture seeds: Co-Authored-By: Codex <noreply@openai.com>
        // The bundled patterns now use (?i) so this must match regardless of case.
        let yaml = include_str!("../../config/patterns/ai-attribution.yaml");
        let cfg = parse_yaml(yaml).unwrap();
        let r = AttributionRewriter::new(&cfg, &[]).unwrap();
        assert!(
            r.line_matches(b"Co-Authored-By: Codex <noreply@openai.com>"),
            "uppercase Co-Authored-By: Codex must be matched"
        );
        assert!(
            r.line_matches(b"Co-Authored-By: aider <noreply@aider.chat>"),
            "uppercase Co-Authored-By: aider must be matched"
        );
    }
}
