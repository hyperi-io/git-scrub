//  Project:      git-scrub
//  File:         src/engine/blob_cache.rs
//  Purpose:      mark -> content cache for path-aware blob rewriting.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Blob mark-cache.
//!
//! `git fast-export` emits `blob` records before the commits that
//! reference them via `M <mode> <mark> <path>` lines. Because
//! `LockfileRewriter` is path-aware, the engine must hold blob
//! content in memory keyed by mark, so it can apply rewriters once
//! a commit references the blob with a path.
//!
//! Memory cost: bounded by total unique blob bytes in the export
//! stream. For typical repos this is fine; for pathological lockfile
//! histories we accept the pressure in v1 and revisit later.

use std::collections::HashMap;

/// Cache of in-flight blobs, keyed by their fast-export mark
/// (`:N` ↔ stored as the integer `N`).
#[derive(Debug, Default)]
pub struct BlobCache {
    entries: HashMap<u64, Vec<u8>>,
}

impl BlobCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a blob's content under `mark`.
    pub fn store(&mut self, mark: u64, content: Vec<u8>) {
        self.entries.insert(mark, content);
    }

    /// Retrieve a borrowed reference to a stored blob.
    #[must_use]
    pub fn get(&self, mark: u64) -> Option<&[u8]> {
        self.entries.get(&mark).map(Vec::as_slice)
    }

    /// Number of currently-cached blobs (for diagnostics / tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_blob_content() {
        let mut cache = BlobCache::new();
        cache.store(42, b"hello".to_vec());
        assert_eq!(cache.get(42), Some(b"hello".as_slice()));
    }

    #[test]
    fn returns_none_for_unknown_mark() {
        let cache = BlobCache::new();
        assert!(cache.get(99).is_none());
    }

    #[test]
    fn tracks_count_and_emptiness() {
        let mut cache = BlobCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        cache.store(1, vec![]);
        assert!(!cache.is_empty());
        assert_eq!(cache.len(), 1);
    }
}
