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
use git_scrub::patterns::{AttributionRewriter, FileMatcher, FileMatcherOptions, LockfileRewriter};
use git_scrub::verify;

use crate::common::{TempRepo, mint_repo_with_cargo_lock_containing_axios};

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

    let stats =
        engine::run(repo.path(), Some(&attr), Some(&matcher), None, None).expect("composite run");
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

    engine::run(repo.path(), Some(&attr), Some(&matcher), None, None).expect("run engine");
    let stats = verify::run(repo.path(), Some(&attr), Some(&matcher), None, None).expect("verify");

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

    let stats = engine::run(repo.path(), None, None, None, None).expect("noop run");
    assert_eq!(
        stats.commits_seen, 0,
        "engine short-circuits when both sets are empty: {stats:?}"
    );
}

/// A test-only `LockfileRewriter` that strips `[[package]]` blocks for the
/// package named "axios" from a Cargo.lock-format file.
struct StripAxiosFromCargo;

impl LockfileRewriter for StripAxiosFromCargo {
    fn applies_to(&self, path: &str) -> bool {
        path == "Cargo.lock" || path.ends_with("/Cargo.lock")
    }

    fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
        let s = std::str::from_utf8(content).ok()?;
        // Locate the [[package]] block for "axios" and remove it.
        // Blocks are separated by blank lines; we look for a block containing
        // `name = "axios"` and drop it entirely.
        let mut out = String::new();
        let mut changed = false;
        for block in s.split("\n\n") {
            if block.lines().any(|l| l.trim() == r#"name = "axios""#) {
                changed = true; // drop this block
            } else {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(block);
            }
        }
        if changed {
            Some(out.into_bytes())
        } else {
            None
        }
    }
}

#[test]
fn lockfile_rewriter_strips_axios_from_cargo_lock() {
    let repo = mint_repo_with_cargo_lock_containing_axios();

    let rewriters: Vec<Box<dyn LockfileRewriter>> = vec![Box::new(StripAxiosFromCargo)];
    let stats = engine::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("engine run with lockfile rewriter");

    assert!(
        stats.blobs_rewritten >= 1,
        "Cargo.lock blob should have been rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("Cargo.lock");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("axios"),
        "axios should have been stripped from Cargo.lock:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent"),
        "innocent package must survive:\n{after_str}"
    );

    // src/main.rs must be untouched.
    let main_after = repo.read_file_at_head("src/main.rs");
    assert_eq!(main_after, b"fn main() {}\n");
}
