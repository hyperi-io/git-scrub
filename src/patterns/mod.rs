//  Project:      git-scrub
//  File:         src/patterns/mod.rs
//  Purpose:      Pattern library — YAML schemas, discovery, and matching logic.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Config-driven pattern library.
//!
//! All AI tool signatures (attribution trailers, footers, artefact file
//! paths) live in YAML data files under `config/patterns/`. The Rust code
//! NEVER hard-codes pattern strings. Operators override defaults via:
//!
//! 1. `--config <path>` on the command line
//! 2. Platform-native discovery cascade (XDG / `~/Library/...` / `%APPDATA%`)
//! 3. Embedded compile-time fallback (`include_str!`)
//!
//! See the design plan at `docs/superpowers/plans/2026-05-13-git-scrub.md`.

pub mod attribution;
pub mod blob;
pub mod discovery;
pub mod files;
pub mod lockfile;
pub mod supply;
pub mod versions;

pub use attribution::{AttributionConfig, AttributionRewriter};
pub use blob::{BlobConfig, BlobRewriter};
pub use discovery::{LoadedPattern, Source};
pub use files::{FileConfig, FileMatcher, FileMatcherOptions};
pub use lockfile::LockfileRewriter;
pub use supply::{CompromisedPackage, PurgeTarget, SupplyConfig};
pub use versions::Spec;
