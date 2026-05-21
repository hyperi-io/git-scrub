//  Project:      git-scrub
//  File:         tests/integration.rs
//  Purpose:      Integration-test root — consolidates submodules into one binary.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Integration tests for `git-scrub`.
//!
//! Single-binary layout: every subtest is a submodule under `integration/`
//! so the test crate links once (3× compile-time saving vs N test binaries).

mod common;

mod integration {
    pub mod ai_curate;
    pub mod attribution_pipeline;
    pub mod engine_pipeline;
    pub mod files_pipeline;
    pub mod state_files;
}
