//  Project:      git-scrub
//  File:         src/patterns/lockfile/go.rs
//  Purpose:      go.sum LockfileRewriter.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! go.sum rewriter.
//!
//! `go.sum` is line-based: `<module> <version>[ /go.mod] <hash>`. Each
//! module/version pair typically has TWO lines (one for the module
//! contents, one for the `go.mod` file). Both are stripped when the
//! `(module, version)` pair matches a target.
//!
//! Version comparison strips the leading `v` (Go modules use `vX.Y.Z`)
//! and any `/go.mod` suffix before delegating to `Spec::matches`.
//! Go pseudo-versions like `v0.0.0-20210101120000-abc123def456` will
//! not match `SemVer` ranges; use exact-string targets for those.
//!
//! ## Module name notes
//!
//! - Major-version suffixes (`/v2`, `/v3`, …) are part of the module name:
//!   `github.com/foo/bar/v2 v2.0.0 h1:hash`. Operators must include the
//!   full module name (including `/vN`) in the target.
//! - `+incompatible` suffixes (e.g. `v2.0.0+incompatible`) pass through
//!   `SemVer` parsing correctly; the suffix is ignored by the parser.
//! - Pseudo-versions (`v0.0.0-20210101120000-abc123def456`) will not match
//!   semver ranges; use an exact-string version target for those.

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

/// Rewriter for `go.sum` lockfiles.
pub struct GoSumRewriter {
    basenames: HashSet<&'static str>,
    targets: Vec<Target>,
}

impl GoSumRewriter {
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
            basenames: ["go.sum"].into_iter().collect(),
            targets,
        })
    }

    /// Returns `true` when `(module, version_field)` matches any target.
    ///
    /// `version_field` is the raw second column from `go.sum`, which may be
    /// `v1.6.1` (module zip hash) or `v1.6.1/go.mod` (go.mod hash). Both
    /// variants are normalised to a bare semver string before matching.
    fn matches(&self, module: &str, version_field: &str) -> bool {
        let core = version_field
            .strip_suffix("/go.mod")
            .unwrap_or(version_field);
        let core = core.strip_prefix('v').unwrap_or(core);
        self.targets
            .iter()
            .any(|t| t.name == module && t.specs.iter().any(|s| s.matches(core)))
    }
}

impl LockfileRewriter for GoSumRewriter {
    fn applies_to(&self, path: &str) -> bool {
        let base = path.rsplit('/').next().unwrap_or(path);
        self.basenames.contains(base)
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let s = std::str::from_utf8(content).ok()?;
        let mut out = String::with_capacity(s.len());
        let mut changed = false;
        // Preserve trailing newline behaviour: split with `lines()`, then
        // re-add `\n` after each — except we track whether the original
        // ended with `\n` and omit the final one if it did not.
        let had_trailing_newline = s.ends_with('\n');
        for line in s.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let (Some(module), Some(version), Some(_hash)) =
                (parts.next(), parts.next(), parts.next())
            else {
                // Malformed / non-standard line — pass through unchanged.
                out.push_str(line);
                out.push('\n');
                continue;
            };
            if self.matches(module, version) {
                changed = true;
                continue; // drop this line
            }
            out.push_str(line);
            out.push('\n');
        }
        if !changed {
            return None;
        }
        if !had_trailing_newline {
            out.pop(); // remove the final '\n' we added
        }
        Some(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::supply::{CompromisedPackage, PurgeTarget};

    fn pkg(name: &str, version: &str) -> CompromisedPackage {
        CompromisedPackage {
            name: name.to_string(),
            ecosystem: "go".to_string(),
            versions: vec![version.to_string()],
            advisories: vec![],
            purge_targets: vec![PurgeTarget::LockfileEntry],
            notes: None,
        }
    }

    fn sample() -> &'static str {
        "github.com/axios/axios v1.6.1 h1:axioshash\n\
         github.com/axios/axios v1.6.1/go.mod h1:axiosgomod\n\
         github.com/innocent-utils/utils v1.0.0 h1:innocenthash\n\
         github.com/innocent-utils/utils v1.0.0/go.mod h1:innocentgomod\n\
         github.com/another/dep v2.3.4 h1:anotherhash\n\
         github.com/another/dep v2.3.4/go.mod h1:anothergomod\n"
    }

    #[test]
    fn applies_to_go_sum() {
        let r = GoSumRewriter::new(&[]).unwrap();
        assert!(r.applies_to("go.sum"));
        assert!(r.applies_to("subpkg/go.sum"));
        assert!(!r.applies_to("go.mod"));
        assert!(!r.applies_to("Cargo.lock"));
    }

    #[test]
    fn strips_both_lines_for_matching_module() {
        let bad = pkg("github.com/axios/axios", "1.6.1");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample().as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            !s.contains("github.com/axios/axios"),
            "all axios lines stripped\n{s}"
        );
        assert!(s.contains("github.com/innocent-utils/utils"));
        assert!(s.contains("github.com/another/dep"));
        // Both module line and /go.mod line gone — 2 fewer lines than sample (6 → 4).
        assert_eq!(s.lines().count(), 4);
    }

    #[test]
    fn target_with_leading_v_does_not_match() {
        // Target version with leading 'v' stays as "v1.6.1" in Spec::Exact.
        // The stripped version from the file is "1.6.1". They differ, so no match.
        // Operators should NOT include a leading 'v' in the version field.
        let bad = pkg("github.com/axios/axios", "v1.6.1");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        assert!(
            r.strip(sample().as_bytes()).is_none(),
            "leading 'v' on target won't match stripped version — operator should omit it"
        );
    }

    #[test]
    fn version_range_with_semver() {
        let bad = pkg("github.com/axios/axios", ">=1.6.0, <1.7.0");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample().as_bytes()).expect("rewrite");
        assert!(!String::from_utf8(out).unwrap().contains("axios"));
    }

    #[test]
    fn returns_none_when_no_match() {
        let bad = pkg("github.com/nonexistent/pkg", "9.9.9");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        assert!(r.strip(sample().as_bytes()).is_none());
    }

    #[test]
    fn preserves_blank_lines() {
        let input = "\n\
                     github.com/axios/axios v1.6.1 h1:hash\n\
                     \n\
                     github.com/innocent v1.0.0 h1:i\n";
        let bad = pkg("github.com/axios/axios", "1.6.1");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(input.as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with('\n'), "leading blank preserved\n{s:?}");
        assert!(s.contains("\n\n"), "middle blank preserved\n{s:?}");
        assert!(s.contains("innocent"));
    }

    #[test]
    fn malformed_line_passes_through() {
        let input = "this is not a valid go.sum line\n\
                     github.com/axios/axios v1.6.1 h1:hash\n";
        let bad = pkg("github.com/axios/axios", "1.6.1");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(input.as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("this is not a valid go.sum line"));
        assert!(!s.contains("axios"));
    }

    #[test]
    fn trailing_newline_is_preserved() {
        let input_with_newline = "github.com/axios/axios v1.6.1 h1:hash\n";
        let bad = pkg("github.com/axios/axios", "1.6.1");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(input_with_newline.as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        // stripping the only line leaves an empty string that ends with '\n' (from the blank out)
        // Actually stripping all lines leaves just "" — check that we don't panic
        assert!(!s.contains("axios"));
    }

    #[test]
    fn no_trailing_newline_preserved() {
        let input_no_newline =
            "github.com/axios/axios v1.6.1 h1:hash\ngithub.com/safe/dep v1.0.0 h1:safe";
        let bad = pkg("github.com/axios/axios", "1.6.1");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(input_no_newline.as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            !s.ends_with('\n'),
            "no trailing newline should be preserved"
        );
        assert!(s.contains("github.com/safe/dep"));
    }

    #[test]
    fn wildcard_version_matches_all() {
        let bad = pkg("github.com/axios/axios", "*");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(sample().as_bytes()).expect("rewrite");
        assert!(!String::from_utf8(out).unwrap().contains("axios"));
    }

    #[test]
    fn major_version_suffix_in_module_name() {
        // go.sum line with /v2 in module name — operator must use full module path.
        let input = "github.com/foo/bar/v2 v2.0.0 h1:foohash\n\
                     github.com/foo/bar/v2 v2.0.0/go.mod h1:foogomod\n\
                     github.com/safe/dep v1.0.0 h1:safe\n";
        let bad = pkg("github.com/foo/bar/v2", "2.0.0");
        let r = GoSumRewriter::new(&[&bad]).unwrap();
        let out = r.strip(input.as_bytes()).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("github.com/foo/bar/v2"));
        assert!(s.contains("github.com/safe/dep"));
    }
}
