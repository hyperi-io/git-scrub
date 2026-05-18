//  Project:      git-scrub
//  File:         src/lib.rs
//  Purpose:      Public library surface — modules re-exported for the bin and tests.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

//! git-scrub library — pattern loading, fast-export engine, plan & runbook output.
//!
//! The binary in [`bin/git-scrub`](../git-scrub/index.html) is a thin clap
//! adapter on top of this library; the library surface is what integration
//! tests drive.

pub mod cli;
pub mod engine;
pub mod gh;
pub mod patterns;
pub mod plan;
pub mod preflight;
pub mod runbook;
pub mod scan;
pub mod verify;

/// Crate version baked at compile time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
