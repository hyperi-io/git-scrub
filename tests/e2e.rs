//  Project:      git-scrub
//  File:         tests/e2e.rs
//  Purpose:      End-to-end test binary — fixture-repo-driven tests.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! End-to-end test binary.
//!
//! Tests in this binary clone or use a local checkout of
//! `hyperi-io/git-scrub-test` (parameterised via the
//! `GIT_SCRUB_TEST_REPO` environment variable) and run `git-scrub`
//! against snapshotted copies of tagged fixtures.
//!
//! Tests skip honestly when the env var is unset — there is no
//! fallback fixture mode.
//!
//! # Running locally
//!
//! ```text
//! export GIT_SCRUB_TEST_REPO=/projects/git-scrub/.tmp/git-scrub-test
//! cargo nextest run --test e2e
//! ```
//!
//! # Running against the remote repo
//!
//! ```text
//! GIT_SCRUB_TEST_REPO=https://github.com/hyperi-io/git-scrub-test \
//!     cargo nextest run --test e2e
//! ```

mod common;

mod e2e {
    // Per-ecosystem test modules land here in Phases 4-16.
    pub mod supply_bun;
    pub mod supply_cargo;
    pub mod supply_composer;
    pub mod supply_go;
    pub mod supply_npm;
    pub mod supply_pip;
    pub mod supply_pnpm;
    pub mod supply_poetry;
    pub mod supply_uv;
    pub mod supply_yarn;
}
