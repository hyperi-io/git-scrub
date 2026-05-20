//  Project:      git-scrub
//  File:         tests/integration/files_pipeline.rs
//  Purpose:      End-to-end file-purge scrub against a real tmp git repo.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

use git_scrub::engine;
use git_scrub::patterns::discovery::EMBEDDED_FILES;
use git_scrub::patterns::files::parse_yaml as parse_files;
use git_scrub::patterns::{FileMatcher, FileMatcherOptions};
use git_scrub::scan;

use crate::common::TempRepo;

#[test]
fn scan_reports_claude_artefact_directory() {
    let repo = TempRepo::new();
    repo.write_file("src/main.rs", "fn main() {}\n");
    repo.write_file(".claude/instructions.md", "be helpful\n");
    repo.add_all_commit("Initial commit");

    let cfg = parse_files(EMBEDDED_FILES).expect("parse files");
    let matcher = FileMatcher::new(&cfg, &FileMatcherOptions::default()).expect("build matcher");

    let stats = scan::scan(repo.path(), None, Some(&matcher), None, None).expect("scan");
    assert_eq!(stats.commits_seen, 1);
    assert_eq!(
        stats.file_ops_dropped, 1,
        "should drop .claude/instructions.md"
    );
}

#[test]
fn execute_drops_claude_directory_from_history() {
    let repo = TempRepo::new();
    repo.write_file("src/main.rs", "fn main() {}\n");
    repo.write_file(".claude/instructions.md", "be helpful\n");
    repo.write_file(".claude/output.md", "result\n");
    repo.add_all_commit("Initial commit");

    let before = repo.log_raw();
    assert!(before.contains(".claude/instructions.md"));

    let cfg = parse_files(EMBEDDED_FILES).expect("parse files");
    let matcher = FileMatcher::new(&cfg, &FileMatcherOptions::default()).expect("build matcher");

    let stats = engine::run(repo.path(), None, Some(&matcher), None, None).expect("engine run");
    assert_eq!(stats.file_ops_dropped, 2);

    let after = repo.log_raw();
    assert!(
        !after.contains(".claude/"),
        "post-scrub raw log must not mention .claude:\n{after}"
    );
    assert!(
        after.contains("src/main.rs"),
        "unrelated files must survive:\n{after}"
    );
}

#[test]
fn include_extra_adds_to_purge_set() {
    let repo = TempRepo::new();
    repo.write_file("src/main.rs", "fn main() {}\n");
    repo.write_file(".env.local", "SECRET=oops\n");
    repo.add_all_commit("Initial commit");

    let cfg = parse_files(EMBEDDED_FILES).expect("parse files");
    let opts = FileMatcherOptions {
        include_extra: vec![".env.local".to_string()],
        ..Default::default()
    };
    let matcher = FileMatcher::new(&cfg, &opts).expect("build matcher");

    let stats = scan::scan(repo.path(), None, Some(&matcher), None, None).expect("scan");
    assert_eq!(stats.file_ops_dropped, 1);
}
