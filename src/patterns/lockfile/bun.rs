//  Project:      git-scrub
//  File:         src/patterns/lockfile/bun.rs
//  Purpose:      bun.lock (text) LockfileRewriter.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! bun.lock (text format) rewriter.
//!
//! Bun's TEXT lockfile is JSONC-flavoured (allows line comments and
//! trailing commas). We strip line comments and trailing commas then parse as
//! standard JSON.
//!
//! The BINARY `bun.lockb` format is NOT supported by v1 (unstable
//! serialisation). Block comments `/* ... */` are also not handled (v1
//! limitation — they are uncommon in bun.lock in practice).

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

/// Rewriter for bun's text lockfile (`bun.lock`).
pub struct BunLockRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl BunLockRewriter {
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
            basenames: ["bun.lock"].into_iter().collect(),
            targets,
        })
    }

    fn matches(&self, name: &str, version: &str) -> bool {
        self.targets
            .iter()
            .any(|t| t.name == name && t.specs.iter().any(|s| s.matches(version)))
    }

    /// Strip JSONC line comments (`// ...`) and trailing commas so the
    /// result parses as standard JSON. Conservative: does NOT strip
    /// comments inside string literals.
    pub(crate) fn jsonc_to_json(input: &str) -> String {
        // First pass: strip line comments, respecting string boundaries.
        let mut out = String::with_capacity(input.len());
        let mut in_string = false;
        let mut escape = false;
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if in_string {
                out.push(c);
                if escape {
                    escape = false;
                    continue;
                }
                if c == '\\' {
                    escape = true;
                    continue;
                }
                if c == '"' {
                    in_string = false;
                }
                continue;
            }
            if c == '"' {
                in_string = true;
                out.push(c);
                continue;
            }
            if c == '/' && chars.peek() == Some(&'/') {
                // Line comment — consume to newline
                for cc in chars.by_ref() {
                    if cc == '\n' {
                        out.push('\n');
                        break;
                    }
                }
                continue;
            }
            out.push(c);
        }

        // Second pass: strip trailing commas before `}` or `]`.
        let chars: Vec<char> = out.chars().collect();
        let mut cleaned = String::with_capacity(out.len());
        let len = chars.len();
        let mut i = 0;
        while i < len {
            let c = chars[i];
            if c == ',' {
                // Look ahead past whitespace
                let mut j = i + 1;
                while j < len && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < len && (chars[j] == '}' || chars[j] == ']') {
                    // Trailing comma — skip it
                    i += 1;
                    continue;
                }
            }
            cleaned.push(c);
            i += 1;
        }
        cleaned
    }

    /// Extract the version from a bun packages array entry like
    /// `["axios@1.6.1", "registry+...", {...}, ""]` → `Some("1.6.1")`.
    /// For scoped names: `["@scope/name@1.0.0", ...]` → `Some("1.0.0")`.
    fn extract_version(arr: &[Value]) -> Option<String> {
        let first = arr.first()?.as_str()?;
        // Use rfind so scoped names like "@types/node@20.0.0" work correctly.
        let idx = first.rfind('@')?;
        if idx == 0 {
            return None; // bare "@" with no name before it
        }
        Some(first[idx + 1..].to_string())
    }
}

impl LockfileRewriter for BunLockRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let s = std::str::from_utf8(content).ok()?;
        let json_str = Self::jsonc_to_json(s);
        let mut doc: Value = serde_json::from_str(&json_str).ok()?;
        let mut changed = false;

        // Strip from `packages`: each value is an array whose first element is
        // "<name>@<version>".
        if let Some(packages) = doc.get_mut("packages").and_then(Value::as_object_mut) {
            packages.retain(|key, value| {
                let Some(arr) = value.as_array() else {
                    return true;
                };
                let Some(version) = Self::extract_version(arr) else {
                    return true;
                };
                if self.matches(key, &version) {
                    changed = true;
                    return false;
                }
                true
            });
        }

        // Strip from `workspaces.*.{dependencies,devDependencies,optionalDependencies}`.
        // The value there is a SemVer range string like "^1.6.0"; we trim leading
        // range operators to extract a base version for matching purposes.
        if let Some(workspaces) = doc.get_mut("workspaces").and_then(Value::as_object_mut) {
            for (_wname, wval) in workspaces.iter_mut() {
                let Some(wmap) = wval.as_object_mut() else {
                    continue;
                };
                for dep_key in ["dependencies", "devDependencies", "optionalDependencies"] {
                    let Some(deps) = wmap.get_mut(dep_key).and_then(Value::as_object_mut) else {
                        continue;
                    };
                    deps.retain(|name, value| {
                        let spec = value.as_str().unwrap_or("");
                        // Strip leading range operators to get a comparable version string.
                        let version = spec.trim_start_matches(|c: char| {
                            matches!(c, '^' | '~' | '=' | '>' | '<' | ' ')
                        });
                        if self.matches(name, version) {
                            changed = true;
                            return false;
                        }
                        true
                    });
                }
            }
        }

        if !changed {
            return None;
        }
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
            ecosystem: "bun".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    fn sample() -> &'static str {
        r#"{
  // bun lockfile v1
  "lockfileVersion": 1,
  "workspaces": {
    "": {
      "name": "my-app",
      "dependencies": {
        "axios": "1.6.1",
        "innocent-utils": "1.0.0",
      },
    },
  },
  "packages": {
    "axios": ["axios@1.6.1", "registry+https://registry.npmjs.org/", { "integrity": "sha512-dead" }, ""],
    "innocent-utils": ["innocent-utils@1.0.0", "registry+https://registry.npmjs.org/", { "integrity": "sha512-alive" }, ""],
    "@types/node": ["@types/node@20.0.0", "registry+https://registry.npmjs.org/", { "integrity": "sha512-types" }, ""],
  },
}
"#
    }

    #[test]
    fn applies_to_bun_path() {
        let r = BunLockRewriter::new(&[]).unwrap();
        assert!(r.applies_to("bun.lock"));
        assert!(r.applies_to("apps/foo/bun.lock"));
        assert!(!r.applies_to("bun.lockb"));
        assert!(!r.applies_to("package-lock.json"));
    }

    #[test]
    fn jsonc_strips_line_comments() {
        let cleaned = BunLockRewriter::jsonc_to_json("{\n  // a comment\n  \"x\": 1\n}");
        assert!(!cleaned.contains("comment"), "comment should be stripped");
        assert!(
            serde_json::from_str::<Value>(&cleaned).is_ok(),
            "result should be valid JSON after comment stripping"
        );
    }

    #[test]
    fn jsonc_strips_trailing_commas() {
        let cleaned = BunLockRewriter::jsonc_to_json("{ \"a\": 1, \"b\": 2, }");
        assert!(
            serde_json::from_str::<Value>(&cleaned).is_ok(),
            "result should be valid JSON after trailing comma stripping: {cleaned}"
        );
    }

    #[test]
    fn jsonc_preserves_string_contents() {
        let cleaned = BunLockRewriter::jsonc_to_json(r#"{ "url": "https://x.com/path" }"#);
        assert!(
            cleaned.contains("https://x.com/path"),
            "URL inside string should survive: {cleaned}"
        );
    }

    #[test]
    fn extract_version_simple() {
        let arr: Vec<Value> = serde_json::from_str(r#"["axios@1.6.1", ""]"#).unwrap();
        assert_eq!(
            BunLockRewriter::extract_version(&arr),
            Some("1.6.1".to_string())
        );
    }

    #[test]
    fn extract_version_scoped() {
        let arr: Vec<Value> = serde_json::from_str(r#"["@types/node@20.0.0", ""]"#).unwrap();
        assert_eq!(
            BunLockRewriter::extract_version(&arr),
            Some("20.0.0".to_string())
        );
    }

    #[test]
    fn strips_matching_packages_entry() {
        let bad = pkg("axios", "1.6.1");
        let r = BunLockRewriter::new(&[&bad]).unwrap();
        let out = r
            .strip(sample().as_bytes())
            .expect("rewrite should produce output");
        let s = String::from_utf8(out).unwrap();
        let parsed: Value = serde_json::from_str(&s).unwrap();
        let packages = parsed
            .get("packages")
            .and_then(Value::as_object)
            .expect("packages object");
        assert!(
            !packages.contains_key("axios"),
            "axios packages entry should be stripped"
        );
        assert!(
            packages.contains_key("innocent-utils"),
            "innocent-utils should remain"
        );
        assert!(
            packages.contains_key("@types/node"),
            "@types/node should remain"
        );
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("nonexistent", "9.9.9");
        let r = BunLockRewriter::new(&[&bad]).unwrap();
        assert!(
            r.strip(sample().as_bytes()).is_none(),
            "no-match should return None"
        );
    }

    #[test]
    fn malformed_returns_none() {
        let bad = pkg("axios", "1.6.1");
        let r = BunLockRewriter::new(&[&bad]).unwrap();
        assert!(
            r.strip(b"not json {{{").is_none(),
            "malformed input should return None"
        );
    }
}
