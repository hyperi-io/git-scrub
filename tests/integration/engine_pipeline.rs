//  Project:      git-scrub
//  File:         tests/integration/engine_pipeline.rs
//  Purpose:      Composite engine pass (attribution + files) end-to-end.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

use git_scrub::engine;
use git_scrub::patterns::attribution::parse_yaml as parse_attr;
use git_scrub::patterns::discovery::{EMBEDDED_ATTRIBUTION, EMBEDDED_FILES};
use git_scrub::patterns::files::parse_yaml as parse_files;
use git_scrub::patterns::{AttributionRewriter, FileMatcher, FileMatcherOptions};
use git_scrub::verify;

use crate::common::TempRepo;

#[test]
fn composite_rewrites_messages_and_drops_files() {
    let repo = TempRepo::new();
    repo.write_file("src/main.rs", "fn main() {}\n");
    repo.write_file(".claude/output.md", "AI artefact\n");
    repo.add_all_commit(
        "Add code and AI artefact\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
    );

    let attr_cfg = parse_attr(EMBEDDED_ATTRIBUTION).expect("attr yaml");
    let attr = AttributionRewriter::new(&attr_cfg, &[]).expect("attr rewriter");

    let file_cfg = parse_files(EMBEDDED_FILES).expect("files yaml");
    let matcher =
        FileMatcher::new(&file_cfg, &FileMatcherOptions::default()).expect("file matcher");

    let stats = engine::run(repo.path(), Some(&attr), Some(&matcher), None).expect("composite run");
    assert!(
        stats.commits_rewritten >= 1,
        "attribution rewrite should have fired: {stats:?}"
    );
    assert!(
        stats.file_ops_dropped >= 1,
        ".claude/output.md should have been dropped: {stats:?}"
    );

    let messages = repo.log_messages();
    assert!(
        !messages.contains("Claude"),
        "messages should be clean:\n{messages}"
    );

    let raw = repo.log_raw();
    assert!(!raw.contains(".claude/"));
    assert!(raw.contains("src/main.rs"));
}

#[test]
fn verify_passes_after_clean_rewrite() {
    let repo = TempRepo::new();
    repo.write_file("src/main.rs", "fn main() {}\n");
    repo.write_file(".claude/output.md", "AI artefact\n");
    repo.add_all_commit("Add stuff\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n");

    let attr_cfg = parse_attr(EMBEDDED_ATTRIBUTION).expect("attr yaml");
    let attr = AttributionRewriter::new(&attr_cfg, &[]).expect("attr rewriter");
    let file_cfg = parse_files(EMBEDDED_FILES).expect("files yaml");
    let matcher =
        FileMatcher::new(&file_cfg, &FileMatcherOptions::default()).expect("file matcher");

    engine::run(repo.path(), Some(&attr), Some(&matcher), None).expect("run engine");
    let stats = verify::run(repo.path(), Some(&attr), Some(&matcher), None).expect("verify");

    assert_eq!(
        stats.commits_rewritten, 0,
        "verify must find zero remaining commits"
    );
    assert_eq!(
        stats.file_ops_dropped, 0,
        "verify must find zero remaining file ops"
    );
}

#[test]
fn empty_pattern_sets_are_a_noop() {
    let repo = TempRepo::new();
    repo.write_file("README.md", "ordinary content\n");
    repo.add_all_commit("Initial");

    let stats = engine::run(repo.path(), None, None, None).expect("noop run");
    assert_eq!(
        stats.commits_seen, 0,
        "engine short-circuits when both sets are empty: {stats:?}"
    );
}
