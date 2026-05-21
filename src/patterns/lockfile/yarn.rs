//  Project:      git-scrub
//  File:         src/patterns/lockfile/yarn.rs
//  Purpose:      yarn.lock (classic v1) LockfileRewriter.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! yarn.lock (classic v1) rewriter.
//!
//! Yarn Berry (v2/v3/v4) uses a YAML-like format with `__metadata:` —
//! NOT supported in git-scrub v1. Use the lockfile-only manual workflow
//! for Berry repos, or wait for a future git-scrub release.
//!
//! Yarn-classic format is block-based:
//! - Blocks are separated by blank lines
//! - Each block opens with comma-separated selectors at column 0
//! - Selector form: `<name>@<range>` (quoted or unquoted)
//! - Block body is 2-space indented; `version "x.y.z"` line gives the resolved version
//!
//! Stripping: drop the entire block if any selector's name + the resolved
//! version match a `Target`.

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

/// Rewriter for yarn.lock (classic v1) lockfiles.
pub struct YarnLockRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl YarnLockRewriter {
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
            basenames: ["yarn.lock"].into_iter().collect(),
            targets,
        })
    }

    fn matches(&self, name: &str, version: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.name == name && t.specs.iter().any(|s| s.matches(version)))
    }

    /// Extract unique package names from a yarn-classic selector line.
    ///
    /// Examples:
    /// - `"axios@^1.6.0":` → `["axios"]`
    /// - `axios@^1.6.0:` → `["axios"]`
    /// - `"@types/node@^20.0.0":` → `["@types/node"]`
    /// - `"axios@^1.6.0", "axios@^1.5.0":` → `["axios"]`
    ///
    /// Scoped names (`@scope/name`) are handled by splitting at the **rightmost**
    /// `@`, which is always the version separator.
    fn extract_names(selector_line: &str) -> Vec<String> {
        let line = selector_line.trim().trim_end_matches(':');
        line.split(',')
            .filter_map(|part| {
                let part = part.trim().trim_matches('"');
                // Split at the rightmost `@` — for scoped names like `@types/node@^20`
                // the rightmost `@` is the version separator (idx > 0).
                let idx = part.rfind('@')?;
                if idx == 0 {
                    return None;
                }
                Some(part[..idx].to_string())
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Extract the `version` value from a yarn-classic block body.
    ///
    /// Looks for a line starting with optional whitespace followed by
    /// `version "x.y.z"`.
    fn extract_version(body_lines: &[&str]) -> Option<String> {
        for line in body_lines {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("version ") {
                let v = rest.trim().trim_matches('"');
                return Some(v.to_string());
            }
        }
        None
    }

    /// Decide whether to keep a block.
    ///
    /// Returns `Some(block)` to keep, `None` to drop.
    fn process_block<'a, F>(block: &'a [&'a str], matches: &F) -> Option<Vec<&'a str>>
    where
        F: Fn(&str, &str) -> bool,
    {
        let first = block.first()?;
        let body: Vec<&str> = block.iter().skip(1).copied().collect();
        let names = Self::extract_names(first);
        let version = Self::extract_version(&body).unwrap_or_default();
        for name in &names {
            if matches(name, &version) {
                return None; // drop the block
            }
        }
        Some(block.to_vec())
    }
}

impl LockfileRewriter for YarnLockRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let s = std::str::from_utf8(content).ok()?;

        // Bail on Berry — always has `__metadata:` near the top.
        if s.contains("__metadata:") {
            return None;
        }

        let mut output = String::with_capacity(s.len());
        let mut changed = false;

        // Collect leading comment/blank lines as preamble.
        let mut lines = s.lines().peekable();
        while let Some(line) = lines.peek() {
            if line.starts_with('#') || line.is_empty() {
                output.push_str(line);
                output.push('\n');
                lines.next();
            } else {
                break;
            }
        }

        // Process the remaining lines as blank-line-delimited blocks.
        let mut buffer: Vec<&str> = Vec::new();
        for line in lines {
            if line.is_empty() {
                if buffer.is_empty() {
                    output.push('\n');
                } else {
                    match Self::process_block(&buffer, &|n, v| self.matches(n, v)) {
                        Some(keep) => {
                            for bl in keep {
                                output.push_str(bl);
                                output.push('\n');
                            }
                            output.push('\n');
                        }
                        None => {
                            changed = true;
                        }
                    }
                    buffer.clear();
                }
            } else {
                buffer.push(line);
            }
        }

        // Trailing block (no final blank line).
        if !buffer.is_empty() {
            match Self::process_block(&buffer, &|n, v| self.matches(n, v)) {
                Some(keep) => {
                    for bl in keep {
                        output.push_str(bl);
                        output.push('\n');
                    }
                }
                None => {
                    changed = true;
                }
            }
        }

        if !changed {
            return None;
        }
        Some(output.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::supply::{CompromisedPackage, PurgeTarget};

    fn pkg(name: &str, version: &str) -> CompromisedPackage {
        CompromisedPackage {
            name: name.to_string(),
            ecosystem: "yarn".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    fn sample_classic() -> &'static str {
        "# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n\
         # yarn lockfile v1\n\
         \n\
         \n\
         \"@types/node@^20.0.0\":\n\
         \x20\x20version \"20.0.0\"\n\
         \x20\x20resolved \"https://registry.yarnpkg.com/@types/node/-/node-20.0.0.tgz#abc\"\n\
         \x20\x20integrity sha512-types\n\
         \n\
         axios@^1.6.0:\n\
         \x20\x20version \"1.6.1\"\n\
         \x20\x20resolved \"https://registry.yarnpkg.com/axios/-/axios-1.6.1.tgz#def\"\n\
         \x20\x20integrity sha512-dead\n\
         \n\
         innocent-utils@^1.0.0:\n\
         \x20\x20version \"1.0.0\"\n\
         \x20\x20resolved \"https://registry.yarnpkg.com/innocent-utils/-/innocent-utils-1.0.0.tgz#ghi\"\n\
         \x20\x20integrity sha512-alive\n"
    }

    #[test]
    fn applies_to_yarn_path() {
        let r = YarnLockRewriter::new(&[]).unwrap();
        assert!(r.applies_to("yarn.lock"));
        assert!(r.applies_to("packages/foo/yarn.lock"));
        assert!(!r.applies_to("package-lock.json"));
        assert!(!r.applies_to("pnpm-lock.yaml"));
    }

    #[test]
    fn extract_names_simple_quoted() {
        let names = YarnLockRewriter::extract_names("\"axios@^1.6.0\":");
        assert_eq!(names, vec!["axios"]);
    }

    #[test]
    fn extract_names_unquoted() {
        let names = YarnLockRewriter::extract_names("axios@^1.6.0:");
        assert_eq!(names, vec!["axios"]);
    }

    #[test]
    fn extract_names_scoped() {
        let names = YarnLockRewriter::extract_names("\"@types/node@^20.0.0\":");
        assert_eq!(names, vec!["@types/node"]);
    }

    #[test]
    fn extract_names_multiple_selectors() {
        let names: HashSet<String> =
            YarnLockRewriter::extract_names("\"axios@^1.6.0\", \"axios@^1.5.0\":")
                .into_iter()
                .collect();
        let expected: HashSet<String> = ["axios".to_string()].into_iter().collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn extract_version_from_body() {
        let body = vec!["  version \"1.6.1\"", "  resolved \"...\""];
        assert_eq!(
            YarnLockRewriter::extract_version(&body),
            Some("1.6.1".to_string())
        );
    }

    #[test]
    fn strips_matching_block_preserving_others() {
        let bad = pkg("axios", "1.6.1");
        let r = YarnLockRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample_classic().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        // axios block gone
        assert!(
            !s.contains("axios@^1.6.0"),
            "axios block should be stripped\n{s}"
        );
        assert!(
            !s.contains("integrity sha512-dead"),
            "axios body should be gone\n{s}"
        );
        // innocent + scoped preserved
        assert!(
            s.contains("innocent-utils"),
            "innocent-utils should remain\n{s}"
        );
        assert!(s.contains("@types/node"), "@types/node should remain\n{s}");
        // Comments preserved
        assert!(
            s.contains("# THIS IS AN AUTOGENERATED FILE"),
            "preamble should remain\n{s}"
        );
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("nonexistent", "9.9.9");
        let r = YarnLockRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(sample_classic().as_bytes()).is_none());
    }

    #[test]
    fn returns_none_for_yarn_berry() {
        let bad = pkg("axios", "1.6.1");
        let r = YarnLockRewriter::new(&[&bad]).unwrap();
        let berry = b"__metadata:\n  version: 8\n\n\"axios@npm:^1.6.0\":\n  version: 1.6.1\n";
        assert!(
            r.strip(berry).is_none(),
            "Berry format must be left alone in v1"
        );
    }

    #[test]
    fn no_packages_rewriter_compiles() {
        let r = YarnLockRewriter::new(&[]).unwrap();
        assert!(r.strip(sample_classic().as_bytes()).is_none());
    }
}
