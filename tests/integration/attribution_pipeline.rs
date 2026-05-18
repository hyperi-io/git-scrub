//  Project:      git-scrub
//  File:         tests/integration/attribution_pipeline.rs
//  Purpose:      End-to-end attribution scrub against a real tmp git repo.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

use git_scrub::engine;
use git_scrub::patterns::AttributionRewriter;
use git_scrub::patterns::attribution::parse_yaml as parse_attr;
use git_scrub::patterns::discovery::EMBEDDED_ATTRIBUTION;
use git_scrub::scan;

use crate::common::TempRepo;

#[test]
fn scan_reports_claude_attribution() {
    let repo = TempRepo::new();
    repo.write_file("README.md", "hello\n");
    repo.add_all_commit(
        "Add README\n\nImplements feature.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
    );

    let cfg = parse_attr(EMBEDDED_ATTRIBUTION).expect("parse attribution");
    let rewriter = AttributionRewriter::new(&cfg, &[]).expect("build rewriter");

    let stats = scan::scan(repo.path(), Some(&rewriter), None, None).expect("scan");
    assert_eq!(stats.commits_seen, 1);
    assert_eq!(stats.commits_rewritten, 1);
    assert_eq!(stats.file_ops_dropped, 0);
}

#[test]
fn execute_strips_claude_trailer_from_history() {
    let repo = TempRepo::new();
    repo.write_file("README.md", "hello\n");
    repo.add_all_commit(
        "Add README\n\nImplements feature.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
    );

    let before = repo.log_messages();
    assert!(
        before.contains("Claude"),
        "fixture must contain Claude trailer pre-scrub"
    );

    let cfg = parse_attr(EMBEDDED_ATTRIBUTION).expect("parse attribution");
    let rewriter = AttributionRewriter::new(&cfg, &[]).expect("build rewriter");

    let stats = engine::run(repo.path(), Some(&rewriter), None, None).expect("engine run");
    assert_eq!(stats.commits_rewritten, 1);

    let after = repo.log_messages();
    assert!(
        !after.contains("Claude"),
        "post-scrub log must not mention Claude:\n{after}"
    );
    assert!(
        after.contains("Implements feature."),
        "non-attribution body lines must survive:\n{after}"
    );
}

#[test]
fn preserves_human_coauthor_lines() {
    let repo = TempRepo::new();
    repo.write_file("README.md", "hello\n");
    repo.add_all_commit(
        "Refactor\n\nCo-authored-by: Real Human <human@example.com>\n\
         Co-Authored-By: Claude <noreply@anthropic.com>\n",
    );

    let cfg = parse_attr(EMBEDDED_ATTRIBUTION).expect("parse attribution");
    let rewriter = AttributionRewriter::new(&cfg, &[]).expect("build rewriter");
    engine::run(repo.path(), Some(&rewriter), None, None).expect("engine run");

    let after = repo.log_messages();
    assert!(
        after.contains("Real Human"),
        "human co-author must survive:\n{after}"
    );
    assert!(!after.contains("Claude"));
}
