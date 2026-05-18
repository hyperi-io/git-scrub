//  Project:      git-scrub
//  File:         src/patterns/blob.rs
//  Purpose:      Byte-level blob-content rewriting for `spill text` / `spill secrets`.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Blob-content rewriter for the v2 `spill` use case.
//!
//! Unlike attribution rewriting (which operates on commit messages),
//! this rewriter applies operator-supplied substitutions to the raw byte
//! content of every blob in the history. Replacement is byte-exact —
//! we never decode payloads as UTF-8, because git stores arbitrary bytes
//! and the operator may be redacting from a binary file.
//!
//! Two replacement kinds:
//!
//! - **Literal** — `memchr::memmem` byte-substring search. Fast,
//!   unambiguous, the right choice for known secrets / fixed tokens.
//! - **Regex** — `regex::bytes::Regex`. Use for patterns
//!   (e.g. `AKIA[A-Z0-9]{16}` for AWS access keys).

use std::borrow::Cow;

use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// YAML schema for blob-rewriter configs (`spill text`, `spill secrets`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlobConfig {
    /// One entry per substitution to apply.
    #[serde(default)]
    pub replacements: Vec<Replacement>,
}

/// A single find/replace rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replacement {
    /// The pattern to find. For `kind: literal`, the exact byte sequence.
    /// For `kind: regex`, a `regex` crate pattern (bytes regex syntax).
    pub find: String,
    /// What to replace each match with. Empty string deletes the match.
    pub replace_with: String,
    /// Replacement kind — defaults to literal substring.
    #[serde(default)]
    pub kind: ReplacementKind,
}

/// How the `find` field is interpreted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReplacementKind {
    /// `find` is a literal byte sequence — exact substring search.
    #[default]
    Literal,
    /// `find` is a `regex::bytes` pattern.
    Regex,
}

/// Errors raised by blob-rewriter loading and application.
#[derive(Debug, Error)]
pub enum BlobError {
    /// A regex pattern failed to compile.
    #[error("invalid regex `{pattern}`: {source}")]
    InvalidRegex {
        /// The offending pattern.
        pattern: String,
        /// Underlying regex error.
        #[source]
        source: regex::Error,
    },
}

/// Compiled blob rewriter — apply substitutions to a byte buffer.
#[derive(Debug)]
pub struct BlobRewriter {
    rules: Vec<CompiledRule>,
}

#[derive(Debug)]
enum CompiledRule {
    Literal { find: Vec<u8>, replace: Vec<u8> },
    Regex { re: Regex, replace: Vec<u8> },
}

impl BlobRewriter {
    /// Build a rewriter from a parsed [`BlobConfig`].
    pub fn new(config: &BlobConfig) -> Result<Self, BlobError> {
        let mut rules: Vec<CompiledRule> = Vec::with_capacity(config.replacements.len());
        for r in &config.replacements {
            let rule = match r.kind {
                ReplacementKind::Literal => CompiledRule::Literal {
                    find: r.find.as_bytes().to_vec(),
                    replace: r.replace_with.as_bytes().to_vec(),
                },
                ReplacementKind::Regex => {
                    let re = Regex::new(&r.find).map_err(|source| BlobError::InvalidRegex {
                        pattern: r.find.clone(),
                        source,
                    })?;
                    CompiledRule::Regex {
                        re,
                        replace: r.replace_with.as_bytes().to_vec(),
                    }
                }
            };
            rules.push(rule);
        }
        Ok(Self { rules })
    }

    /// Returns `true` if the rewriter has no rules — [`apply`] is a no-op.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Apply all rules to `payload`. Returns the rewritten bytes plus
    /// `true` iff anything changed.
    #[must_use]
    pub fn apply<'a>(&self, payload: &'a [u8]) -> (Cow<'a, [u8]>, bool) {
        if self.rules.is_empty() {
            return (Cow::Borrowed(payload), false);
        }
        let mut current: Cow<'_, [u8]> = Cow::Borrowed(payload);
        let mut changed = false;
        for rule in &self.rules {
            match rule {
                CompiledRule::Literal { find, replace } => {
                    if let Some(rewritten) = apply_literal(&current, find, replace) {
                        current = Cow::Owned(rewritten);
                        changed = true;
                    }
                }
                CompiledRule::Regex { re, replace } => {
                    let result = re.replace_all(&current, replace.as_slice());
                    match result {
                        Cow::Borrowed(_) => {}
                        Cow::Owned(v) => {
                            current = Cow::Owned(v);
                            changed = true;
                        }
                    }
                }
            }
        }
        (current, changed)
    }
}

fn apply_literal(haystack: &[u8], needle: &[u8], replace: &[u8]) -> Option<Vec<u8>> {
    if needle.is_empty() {
        return None;
    }
    let finder = memchr::memmem::Finder::new(needle);
    let mut iter = finder.find_iter(haystack);
    let first = iter.next()?;
    let mut out = Vec::with_capacity(haystack.len());
    out.extend_from_slice(&haystack[..first]);
    out.extend_from_slice(replace);
    let mut cursor = first + needle.len();
    for next_start in iter {
        out.extend_from_slice(&haystack[cursor..next_start]);
        out.extend_from_slice(replace);
        cursor = next_start + needle.len();
    }
    out.extend_from_slice(&haystack[cursor..]);
    Some(out)
}

/// Parse YAML into a [`BlobConfig`].
pub fn parse_yaml(yaml: &str) -> Result<BlobConfig, serde_yaml_ng::Error> {
    serde_yaml_ng::from_str(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal_cfg(find: &str, replace: &str) -> BlobConfig {
        BlobConfig {
            replacements: vec![Replacement {
                find: find.into(),
                replace_with: replace.into(),
                kind: ReplacementKind::Literal,
            }],
        }
    }

    fn regex_cfg(find: &str, replace: &str) -> BlobConfig {
        BlobConfig {
            replacements: vec![Replacement {
                find: find.into(),
                replace_with: replace.into(),
                kind: ReplacementKind::Regex,
            }],
        }
    }

    #[test]
    fn literal_replaces_all_occurrences() {
        let r = BlobRewriter::new(&literal_cfg("SECRET", "<REDACTED>")).unwrap();
        let (out, changed) = r.apply(b"a SECRET, another SECRET, end");
        assert!(changed);
        assert_eq!(&*out, b"a <REDACTED>, another <REDACTED>, end");
    }

    #[test]
    fn literal_no_match_borrows() {
        let r = BlobRewriter::new(&literal_cfg("SECRET", "<REDACTED>")).unwrap();
        let (out, changed) = r.apply(b"no match here");
        assert!(!changed);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn regex_replaces_pattern() {
        let r = BlobRewriter::new(&regex_cfg(r"AKIA[A-Z0-9]{16}", "<AWS_KEY>")).unwrap();
        let (out, changed) = r.apply(b"key=AKIAIOSFODNN7EXAMPLE end");
        assert!(changed);
        assert_eq!(&*out, b"key=<AWS_KEY> end");
    }

    #[test]
    fn invalid_regex_surfaces_error() {
        let cfg = regex_cfg("[unclosed", "x");
        let err = BlobRewriter::new(&cfg).unwrap_err();
        assert!(matches!(err, BlobError::InvalidRegex { .. }));
    }

    #[test]
    fn empty_rewriter_is_passthrough() {
        let r = BlobRewriter::new(&BlobConfig::default()).unwrap();
        assert!(r.is_empty());
        let (out, changed) = r.apply(b"anything");
        assert!(!changed);
        assert_eq!(&*out, b"anything");
    }

    #[test]
    fn handles_non_utf8_bytes() {
        let r = BlobRewriter::new(&literal_cfg("\x00secret", "<X>")).unwrap();
        let (out, changed) = r.apply(b"prefix\x00secret\xffsuffix");
        assert!(changed);
        assert_eq!(&*out, b"prefix<X>\xffsuffix");
    }

    #[test]
    fn empty_replacement_deletes_matches() {
        let r = BlobRewriter::new(&literal_cfg("token", "")).unwrap();
        let (out, _) = r.apply(b"a token b token c");
        assert_eq!(&*out, b"a  b  c");
    }

    #[test]
    fn multiple_rules_chain() {
        let cfg = BlobConfig {
            replacements: vec![
                Replacement {
                    find: "FOO".into(),
                    replace_with: "BAR".into(),
                    kind: ReplacementKind::Literal,
                },
                Replacement {
                    find: "BAR".into(),
                    replace_with: "BAZ".into(),
                    kind: ReplacementKind::Literal,
                },
            ],
        };
        let r = BlobRewriter::new(&cfg).unwrap();
        let (out, _) = r.apply(b"FOO");
        assert_eq!(&*out, b"BAZ", "rules apply in order");
    }

    #[test]
    fn yaml_parses_default_kind_as_literal() {
        let yaml = r#"
replacements:
  - find: "x"
    replace_with: "y"
"#;
        let cfg = parse_yaml(yaml).unwrap();
        assert_eq!(cfg.replacements[0].kind, ReplacementKind::Literal);
    }

    #[test]
    fn yaml_parses_regex_kind() {
        let yaml = r#"
replacements:
  - find: "\\d+"
    replace_with: "N"
    kind: regex
"#;
        let cfg = parse_yaml(yaml).unwrap();
        assert_eq!(cfg.replacements[0].kind, ReplacementKind::Regex);
    }
}
