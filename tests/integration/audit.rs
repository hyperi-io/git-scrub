//  Project:      git-scrub
//  File:         tests/integration/audit.rs
//  Purpose:      Integration tests for the `audit` umbrella subcommand.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

use git_scrub::cli::curate::{CurateAuditSummary, audit_only};

use crate::common::TempRepo;

// ── audit_only library API ────────────────────────────────────────────────────

#[test]
fn audit_only_returns_summary_for_dirty_repo() {
    let repo = TempRepo::new();
    // Minimal .gitignore — missing many curate-config paths.
    repo.write_file(".gitignore", "target/\n");
    repo.add_all_commit("initial");

    let summary: CurateAuditSummary =
        audit_only(repo.path(), None).expect("audit_only should succeed");

    // A repo with just "target/" in .gitignore should have many missing entries.
    assert!(
        !summary.gitignore_missing.is_empty(),
        "expected missing gitignore entries: {:?}",
        summary.gitignore_missing
    );
    // Missing policy files (AI-TRAINING-POLICY.md, robots.txt)
    assert!(
        !summary.policy_files_missing.is_empty(),
        "expected missing policy files: {:?}",
        summary.policy_files_missing
    );
}

#[test]
fn audit_only_reports_clean_when_fully_configured() {
    let repo = TempRepo::new();
    repo.write_file(
        ".gitignore",
        concat!(
            ".claude/\n.cursor/\n.codex/\n.aider*\n.continue/\n.windsurf/\n",
            "hyperi-ai/\n.mcp.json\n.ai-version\n",
            "STATE.md\nCLAUDE.md\nGEMINI.md\nAGENTS.md\nCURSOR.md\nCODEX.md\nTODO.md\n",
            "docs/superpowers/\n",
        ),
    );
    repo.write_file("AI-TRAINING-POLICY.md", "policy content\n");
    repo.write_file("robots.txt", "robots content\n");
    repo.add_all_commit("fully configured");

    let summary = audit_only(repo.path(), None).expect("audit_only should succeed");

    assert!(
        summary.gitignore_missing.is_empty(),
        "expected no missing gitignore entries: {:?}",
        summary.gitignore_missing
    );
    assert!(
        summary.policy_files_missing.is_empty(),
        "expected no missing policy files: {:?}",
        summary.policy_files_missing
    );
    assert!(
        summary.tracked_conflicts.is_empty(),
        "expected no tracked conflicts: {:?}",
        summary.tracked_conflicts
    );
}

#[test]
fn audit_only_detects_tracked_conflicts() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    // Track STATE.md — should be gitignored per curate config.
    repo.write_file("STATE.md", "tracked state\n");
    repo.add_all_commit("with tracked conflict");

    let summary = audit_only(repo.path(), None).expect("audit_only should succeed");

    let conflict_paths: Vec<String> = summary
        .tracked_conflicts
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    assert!(
        conflict_paths.iter().any(|p| p.contains("STATE.md")),
        "expected STATE.md in tracked conflicts: {conflict_paths:?}"
    );
}

#[test]
fn audit_only_empty_summary_fields_are_vecs() {
    // Verify the CurateAuditSummary derives Default correctly (all empty vecs).
    let summary = CurateAuditSummary::default();
    assert!(summary.gitignore_missing.is_empty());
    assert!(summary.policy_files_missing.is_empty());
    assert!(summary.stray_reference_findings.is_empty());
    assert!(summary.tracked_conflicts.is_empty());
}
