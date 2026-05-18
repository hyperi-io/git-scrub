//  Project:      git-scrub
//  File:         src/main.rs
//  Purpose:      Binary entry point — clap parse and dispatch to lib::cli::run.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

#![forbid(unsafe_code)]

use anyhow::Result;
use clap::Parser;

use git_scrub::cli::{self, Cli};

fn main() -> Result<()> {
    let cli = Cli::parse();
    cli::run(&cli)
}
