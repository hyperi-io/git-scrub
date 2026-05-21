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
use git_scrub::patterns::lockfile::{
    BunLockRewriter, CargoLockRewriter, NpmLockRewriter, PnpmLockRewriter, YarnLockRewriter,
};
use git_scrub::patterns::supply::{CompromisedPackage, PurgeTarget};
use git_scrub::patterns::{AttributionRewriter, FileMatcher, FileMatcherOptions, LockfileRewriter};
use git_scrub::verify::{self, VerifyError};

use crate::common::{
    TempRepo, mint_repo_with_bun_lockfile_containing_axios,
    mint_repo_with_cargo_lock_containing_axios, mint_repo_with_npm_lockfile_containing_axios,
    mint_repo_with_pnpm_lockfile_containing_axios, mint_repo_with_poetry_lock_containing_axios,
    mint_repo_with_uv_lock_containing_axios, mint_repo_with_yarn_lockfile_containing_axios,
};

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

    // Post-rewrite verify: a re-scan must find zero remaining lockfile matches.
    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after lockfile rewrite must pass — axios should be gone");
}

#[test]
fn real_cargo_lock_rewriter_strips_fake_malware_from_synthetic_repo() {
    // mint_repo_with_cargo_lock_containing_axios seeds axios at version 1.0.0.
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "cargo".to_string(),
        versions: vec!["1.0.0".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(CargoLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_cargo_lock_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("Cargo.lock");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("name = \"axios\""),
        "axios should be gone from Cargo.lock:\n{after_str}"
    );
    assert!(
        after_str.contains("name = \"innocent\""),
        "innocent should remain in Cargo.lock:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after real CargoLockRewriter must pass");
}

#[test]
fn verify_fails_when_lockfile_pattern_survives_rewrite() {
    // Regression test: verify must FAIL when content wasn't actually rewritten.
    // This test proves verify is doing its job as a safety net.
    //
    // Setup: mint a repo with axios at version 1.0.0 (from the fixture).
    // We do NOT run engine::run to rewrite it.
    // Then call verify::run with a CargoLockRewriter that WOULD match axios.
    // Expected: verify fails with VerifyError::StillMatching, because
    // the un-rewritten repo still contains the axios package.

    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "cargo".to_string(),
        versions: vec!["1.0.0".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };

    let repo = mint_repo_with_cargo_lock_containing_axios();
    let rewriters: Vec<Box<dyn LockfileRewriter>> =
        vec![Box::new(CargoLockRewriter::new(&[&bad]).unwrap())];

    // Call verify WITHOUT calling engine::run first.
    // The axios entry is still in the repo, so verify should fail.
    let result = verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()));

    match result {
        Err(VerifyError::StillMatching { blobs, .. }) => {
            assert!(
                blobs >= 1,
                "expected at least one blob hit for surviving axios, got {blobs}"
            );
        }
        Err(other) => panic!("expected StillMatching, got {other:?}"),
        Ok(stats) => panic!(
            "verify passed unexpectedly — should have caught the surviving axios pattern: {stats:?}"
        ),
    }
}

#[test]
fn npm_lock_rewriter_strips_axios_from_synthetic_repo() {
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "npm".to_string(),
        versions: vec!["1.6.1".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(NpmLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_npm_lockfile_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one npm blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("package-lock.json");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("\"node_modules/axios\""),
        "axios should be gone from package-lock.json:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent-utils"),
        "innocent-utils should remain in package-lock.json:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after NpmLockRewriter must pass — axios should be gone");
}

#[test]
fn cargo_lock_rewriter_also_strips_from_uv_lock() {
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "uv".to_string(),
        versions: vec!["1.6.1".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(CargoLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_uv_lock_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one uv.lock blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("uv.lock");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("name = \"axios\""),
        "axios should be gone from uv.lock:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent-utils"),
        "innocent-utils should remain in uv.lock:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after CargoLockRewriter on uv.lock must pass");
}

#[test]
fn cargo_lock_rewriter_also_strips_from_poetry_lock() {
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "poetry".to_string(),
        versions: vec!["1.6.1".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(CargoLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_poetry_lock_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one poetry.lock blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("poetry.lock");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("name = \"axios\""),
        "axios should be gone from poetry.lock:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent-utils"),
        "innocent-utils should remain in poetry.lock:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after CargoLockRewriter on poetry.lock must pass");
}

#[test]
fn pnpm_lock_rewriter_strips_axios_from_synthetic_repo() {
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "pnpm".to_string(),
        versions: vec!["1.6.1".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(PnpmLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_pnpm_lockfile_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one pnpm blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("pnpm-lock.yaml");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("axios@1.6.1"),
        "axios should be gone from pnpm-lock.yaml:\n{after_str}"
    );
    assert!(
        !after_str.contains("axios:"),
        "axios importer dep should be gone from pnpm-lock.yaml:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent-utils"),
        "innocent-utils should remain in pnpm-lock.yaml:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after PnpmLockRewriter must pass — axios should be gone");
}

#[test]
fn yarn_lock_rewriter_strips_axios_from_synthetic_repo() {
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "yarn".to_string(),
        versions: vec!["1.6.1".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(YarnLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_yarn_lockfile_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one yarn.lock blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("yarn.lock");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("axios@^1.6.0"),
        "axios block should be gone from yarn.lock:\n{after_str}"
    );
    assert!(
        !after_str.contains("integrity sha512-dead"),
        "axios body should be gone from yarn.lock:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent-utils"),
        "innocent-utils should remain in yarn.lock:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after YarnLockRewriter must pass — axios should be gone");
}

#[test]
fn bun_lock_rewriter_strips_axios_from_synthetic_repo() {
    let bad = CompromisedPackage {
        name: "axios".to_string(),
        ecosystem: "bun".to_string(),
        versions: vec!["1.6.1".to_string()],
        advisories: vec![],
        purge_targets: vec![PurgeTarget::LockfileEntry],
        notes: None,
    };
    let rewriter: Box<dyn LockfileRewriter> = Box::new(BunLockRewriter::new(&[&bad]).unwrap());

    let repo = mint_repo_with_bun_lockfile_containing_axios();
    let rewriters = vec![rewriter];
    let stats =
        engine::run(repo.path(), None, None, None, Some(rewriters.as_slice())).expect("engine run");
    assert!(
        stats.blobs_rewritten >= 1,
        "expected at least one bun.lock blob rewritten: {stats:?}"
    );

    let after = repo.read_file_at_head("bun.lock");
    let after_str = String::from_utf8_lossy(&after);
    assert!(
        !after_str.contains("\"axios\""),
        "axios should be gone from bun.lock:\n{after_str}"
    );
    assert!(
        after_str.contains("innocent-utils"),
        "innocent-utils should remain in bun.lock:\n{after_str}"
    );

    verify::run(repo.path(), None, None, None, Some(rewriters.as_slice()))
        .expect("verify after BunLockRewriter must pass — axios should be gone");
}
