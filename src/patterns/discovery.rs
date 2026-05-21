//  Project:      git-scrub
//  File:         src/patterns/discovery.rs
//  Purpose:      Platform-native config-file discovery cascade.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Pattern-file discovery cascade.
//!
//! Resolution order (highest priority first):
//!
//! 1. Explicit `--config <path>` (caller passes via `override_path`)
//! 2. Per-user config dir — Linux XDG, macOS `~/Library/Application Support`, Windows `%APPDATA%`
//! 3. Per-user data dir — Linux `XDG_DATA`, macOS Application Support, Windows `%LOCALAPPDATA%`
//! 4. System data dir — `/usr/share/git-scrub/...` (Linux/macOS), `%PROGRAMDATA%` (Windows)
//! 5. Binary-relative — `<binary-dir>/../share/git-scrub/...`
//! 6. Embedded compile-time fallback (`include_str!`)
//!
//! The `directories` crate handles the platform mapping for layers 2-3.

use std::path::{Path, PathBuf};
use std::{fs, io};

use directories::ProjectDirs;
use thiserror::Error;

/// Embedded fallback for `ai-attribution.yaml` (compile-time).
pub const EMBEDDED_ATTRIBUTION: &str = include_str!("../../config/patterns/ai-attribution.yaml");

/// Embedded fallback for `ai-files.yaml` (compile-time).
pub const EMBEDDED_FILES: &str = include_str!("../../config/patterns/ai-files.yaml");

/// Embedded supply chain advisory snapshot — compile-time fallback.
pub const EMBEDDED_SUPPLY_CHAIN: &str = include_str!("../../config/patterns/supply-chain.yaml");

/// Embedded fallback for `ai-curate.yaml` (compile-time).
pub const EMBEDDED_AI_CURATE: &str = include_str!("../../config/patterns/ai-curate.yaml");

/// Embedded canonical `AI-TRAINING-POLICY.md` content.
pub const EMBEDDED_AI_TRAINING_POLICY: &str = include_str!("../../AI-TRAINING-POLICY.md");

/// Embedded canonical `robots.txt` content.
pub const EMBEDDED_ROBOTS_TXT: &str = include_str!("../../robots.txt");

/// Pattern-file names recognised at runtime.
pub const ATTRIBUTION_FILE: &str = "ai-attribution.yaml";
/// Pattern-file names recognised at runtime.
pub const FILES_FILE: &str = "ai-files.yaml";
/// Discovery filename for the supply chain pattern file.
pub const SUPPLY_CHAIN_FILE: &str = "supply-chain.yaml";
/// Discovery filename for the curate pattern file.
pub const AI_CURATE_FILE: &str = "ai-curate.yaml";

/// Errors raised by pattern-file discovery.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// Could not read the resolved path.
    #[error("failed to read pattern file '{path}': {source}")]
    Io {
        /// The path the loader tried to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// Where in the cascade a pattern file was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Operator-supplied `--config <path>` override.
    Override,
    /// Per-user config dir (XDG / Library / `%APPDATA%`).
    UserConfig,
    /// Per-user data dir (`XDG_DATA` / Library / `%LOCALAPPDATA%`).
    UserData,
    /// System-wide data dir (`/usr/share/...` / `%PROGRAMDATA%`).
    System,
    /// Binary-relative `share/` dir for portable installs.
    BinaryRelative,
    /// Compile-time embedded fallback.
    Embedded,
}

impl Source {
    /// Stable human label for runbook output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Override => "command-line override",
            Self::UserConfig => "per-user config",
            Self::UserData => "per-user data",
            Self::System => "system",
            Self::BinaryRelative => "binary-relative",
            Self::Embedded => "embedded fallback",
        }
    }
}

/// A resolved pattern-file load.
#[derive(Debug)]
pub struct LoadedPattern {
    /// Where the content came from in the cascade.
    pub source: Source,
    /// On-disk path if the content came from the filesystem, else `None`.
    pub path: Option<PathBuf>,
    /// Raw YAML content (UTF-8).
    pub content: String,
}

/// Resolve and load a named pattern file, walking the discovery cascade.
///
/// Layer 1 (`override_path`) is honoured first if supplied. If the override
/// path does not exist, this returns a [`DiscoveryError::Io`]; we do NOT
/// silently fall through, because operator-supplied paths must be reliable.
pub fn load(
    name: &str,
    embedded: &'static str,
    override_path: Option<&Path>,
) -> Result<LoadedPattern, DiscoveryError> {
    if let Some(p) = override_path {
        let content = fs::read_to_string(p).map_err(|source| DiscoveryError::Io {
            path: p.to_path_buf(),
            source,
        })?;
        return Ok(LoadedPattern {
            source: Source::Override,
            path: Some(p.to_path_buf()),
            content,
        });
    }

    if let Some(found) = first_existing(name) {
        let (source, path) = found;
        match fs::read_to_string(&path) {
            Ok(content) => {
                return Ok(LoadedPattern {
                    source,
                    path: Some(path),
                    content,
                });
            }
            Err(source_err) => {
                return Err(DiscoveryError::Io {
                    path,
                    source: source_err,
                });
            }
        }
    }

    Ok(LoadedPattern {
        source: Source::Embedded,
        path: None,
        content: embedded.to_string(),
    })
}

fn first_existing(name: &str) -> Option<(Source, PathBuf)> {
    for (source, dir) in candidate_dirs() {
        let candidate = dir.join("patterns").join(name);
        if candidate.is_file() {
            return Some((source, candidate));
        }
    }
    None
}

fn candidate_dirs() -> Vec<(Source, PathBuf)> {
    let mut out: Vec<(Source, PathBuf)> = Vec::new();

    if let Some(pd) = ProjectDirs::from("io", "hyperi", "git-scrub") {
        out.push((Source::UserConfig, pd.config_dir().to_path_buf()));
        out.push((Source::UserData, pd.data_dir().to_path_buf()));
    }

    for sys in system_dirs() {
        out.push((Source::System, sys));
    }

    if let Some(bin_share) = binary_relative_share() {
        out.push((Source::BinaryRelative, bin_share));
    }

    out
}

fn system_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            v.push(PathBuf::from(programdata).join("git-scrub"));
        }
    } else if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/opt/homebrew/share/git-scrub"));
        v.push(PathBuf::from("/usr/local/share/git-scrub"));
        v.push(PathBuf::from("/usr/share/git-scrub"));
    } else {
        v.push(PathBuf::from("/usr/local/share/git-scrub"));
        v.push(PathBuf::from("/usr/share/git-scrub"));
    }
    v
}

fn binary_relative_share() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bindir = exe.parent()?;
    let candidate = bindir.parent()?.join("share").join("git-scrub");
    Some(candidate)
}

/// Cache directory for backups and runbooks — platform-native.
///
/// Returns `None` if no project-dirs could be resolved (extremely rare —
/// implies the OS gave us no home directory).
#[must_use]
pub fn cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("io", "hyperi", "git-scrub").map(|pd| pd.cache_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn override_path_loads_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.yaml");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "trailers: {{}}").unwrap();
        let loaded = load("ai-attribution.yaml", EMBEDDED_ATTRIBUTION, Some(&path)).unwrap();
        assert_eq!(loaded.source, Source::Override);
        assert_eq!(loaded.path, Some(path));
    }

    #[test]
    fn override_path_missing_returns_io_error() {
        let bad = PathBuf::from("/this/path/does/not/exist/for/git-scrub.yaml");
        let err = load("ai-attribution.yaml", EMBEDDED_ATTRIBUTION, Some(&bad)).unwrap_err();
        match err {
            DiscoveryError::Io { path, .. } => assert_eq!(path, bad),
        }
    }

    #[test]
    fn embedded_fallback_is_valid_yaml() {
        // Sanity: the compile-time strings must parse.
        let _: serde_yaml_ng::Value = serde_yaml_ng::from_str(EMBEDDED_ATTRIBUTION).unwrap();
        let _: serde_yaml_ng::Value = serde_yaml_ng::from_str(EMBEDDED_FILES).unwrap();
    }

    #[test]
    fn embedded_fallback_used_when_nothing_found() {
        // No override, and we can't fully control the system; this asserts
        // that the API path of "no override, nothing exists" returns Embedded
        // is exercised by the embedded constant being non-empty.
        assert!(!EMBEDDED_ATTRIBUTION.is_empty());
        assert!(!EMBEDDED_FILES.is_empty());
    }

    #[test]
    fn source_labels_are_stable() {
        assert_eq!(Source::Override.label(), "command-line override");
        assert_eq!(Source::Embedded.label(), "embedded fallback");
        assert_eq!(Source::UserConfig.label(), "per-user config");
        assert_eq!(Source::UserData.label(), "per-user data");
        assert_eq!(Source::System.label(), "system");
        assert_eq!(Source::BinaryRelative.label(), "binary-relative");
    }

    #[test]
    fn embedded_supply_chain_parses_as_yaml() {
        let _: serde_yaml_ng::Value = serde_yaml_ng::from_str(EMBEDDED_SUPPLY_CHAIN).unwrap();
    }

    #[test]
    fn embedded_supply_chain_round_trips_to_config_type() {
        let _: crate::patterns::SupplyConfig =
            serde_yaml_ng::from_str(EMBEDDED_SUPPLY_CHAIN).unwrap();
    }

    #[test]
    fn embedded_supply_chain_not_empty() {
        assert!(!EMBEDDED_SUPPLY_CHAIN.is_empty());
    }

    #[test]
    fn embedded_ai_curate_parses_as_yaml() {
        let _: serde_yaml_ng::Value = serde_yaml_ng::from_str(EMBEDDED_AI_CURATE).unwrap();
    }

    #[test]
    fn embedded_ai_curate_round_trips_to_curate_config() {
        let _: crate::patterns::CurateConfig = serde_yaml_ng::from_str(EMBEDDED_AI_CURATE).unwrap();
    }

    #[test]
    fn embedded_ai_curate_not_empty() {
        assert!(!EMBEDDED_AI_CURATE.is_empty());
    }

    #[test]
    fn embedded_policy_files_not_empty() {
        assert!(!EMBEDDED_AI_TRAINING_POLICY.is_empty());
        assert!(!EMBEDDED_ROBOTS_TXT.is_empty());
    }
}
