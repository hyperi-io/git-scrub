//  Project:      git-scrub
//  File:         tests/integration/state_files.rs
//  Purpose:      Verify the state-file family (CLAUDE.md, STATE.md, ...) is preserved.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

use git_scrub::patterns::discovery::EMBEDDED_FILES;
use git_scrub::patterns::files::parse_yaml as parse_files;
use git_scrub::patterns::{FileMatcher, FileMatcherOptions};
use git_scrub::scan;

use crate::common::TempRepo;

#[test]
fn state_file_family_survives_by_default() {
    let repo = TempRepo::new();
    repo.write_file("CLAUDE.md", "# Project state\n");
    repo.write_file("STATE.md", "# Project state\n");
    repo.write_file(".claude/output.md", "AI output\n");
    repo.add_all_commit("Add state files and AI output");

    let cfg = parse_files(EMBEDDED_FILES).expect("parse files");
    let matcher = FileMatcher::new(&cfg, &FileMatcherOptions::default()).expect("build matcher");
    let stats = scan::scan(repo.path(), None, Some(&matcher), None).expect("scan");

    // Only the .claude/output.md inside .claude/ should be dropped — CLAUDE.md
    // (state file at the root) should survive.
    assert_eq!(stats.file_ops_dropped, 1);
}

#[test]
fn force_include_overrides_state_file_protection() {
    let repo = TempRepo::new();
    repo.write_file("CLAUDE.md", "# state\n");
    repo.add_all_commit("Add CLAUDE.md");

    let cfg = parse_files(EMBEDDED_FILES).expect("parse files");
    let opts = FileMatcherOptions {
        include_extra: vec!["CLAUDE.md".to_string()],
        force_include: vec!["CLAUDE.md".to_string()],
        ..Default::default()
    };
    let matcher = FileMatcher::new(&cfg, &opts).expect("build matcher");

    let stats = scan::scan(repo.path(), None, Some(&matcher), None).expect("scan");
    assert_eq!(stats.file_ops_dropped, 1);
}
