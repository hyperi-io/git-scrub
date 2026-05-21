//  Project:      git-scrub
//  File:         src/patterns/lockfile/mod.rs
//  Purpose:      LockfileRewriter trait — path-aware blob rewriting for supply chain scrub.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Path-aware blob transformer.
//!
//! Unlike `BlobRewriter` (which rewrites every blob's content), a
//! `LockfileRewriter` only fires on blobs whose committed path matches a
//! known lockfile basename. This requires the engine to maintain a
//! `mark -> content` cache for in-flight blobs.

pub mod bun;
pub use bun::BunLockRewriter;

pub mod cargo;
pub use cargo::CargoLockRewriter;

pub mod npm;
pub use npm::NpmLockRewriter;

pub mod pip;
pub use pip::PipLockRewriter;

pub mod pnpm;
pub use pnpm::PnpmLockRewriter;

pub mod yarn;
pub use yarn::YarnLockRewriter;

/// A transformer that conditionally rewrites lockfile blob contents.
pub trait LockfileRewriter: Send + Sync {
    /// Whether this rewriter applies to the given committed path.
    #[must_use]
    fn applies_to(&self, path: &str) -> bool;

    /// Rewrite `content`. Return `None` when no change is needed
    /// (engine emits the original blob unchanged).
    #[must_use]
    fn strip(&self, content: &[u8]) -> Option<Vec<u8>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StripAxiosFromCargo;

    impl LockfileRewriter for StripAxiosFromCargo {
        fn applies_to(&self, path: &str) -> bool {
            path.ends_with("/Cargo.lock") || path == "Cargo.lock"
        }
        fn strip(&self, _content: &[u8]) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn applies_to_matches_cargo_lock_at_root() {
        assert!(StripAxiosFromCargo.applies_to("Cargo.lock"));
        assert!(StripAxiosFromCargo.applies_to("crates/foo/Cargo.lock"));
        assert!(!StripAxiosFromCargo.applies_to("Cargo.toml"));
        assert!(!StripAxiosFromCargo.applies_to("vendor/axios/Cargo.lock.bak"));
    }

    #[test]
    fn strip_returns_none_for_unchanged_content() {
        let result = StripAxiosFromCargo.strip(b"some content");
        assert!(result.is_none());
    }
}
