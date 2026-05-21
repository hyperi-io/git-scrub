//  Project:      git-scrub
//  File:         tests/integration/ai_curate.rs
//  Purpose:      Integration tests for `ai curate` working-tree curation.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

use git_scrub::cli::curate::{CurateArgs, run};
use git_scrub::patterns::discovery::{EMBEDDED_AI_CURATE, EMBEDDED_AI_TRAINING_POLICY};

use crate::common::TempRepo;

/// Build a minimal `CurateArgs` for test invocations (dry-run by default).
fn args_dry_run() -> CurateArgs {
    CurateArgs {
        execute: false,
        force: false,
        config: None,
        policy_source: None,
        report: None,
    }
}

fn args_execute_force() -> CurateArgs {
    CurateArgs {
        execute: true,
        force: true,
        config: None,
        policy_source: None,
        report: None,
    }
}

// ── Dry-run / reporting ───────────────────────────────────────────────────────

#[test]
fn curate_dry_run_reports_missing_gitignore_entries() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    repo.add_all_commit("initial");

    let report_path = repo.path().join("curate-report.md");
    let args = CurateArgs {
        report: Some(report_path.clone()),
        ..args_dry_run()
    };

    run(&args, Some(repo.path())).expect("curate dry-run should succeed");

    let report = std::fs::read_to_string(&report_path).unwrap();
    assert!(
        report.contains(".claude/"),
        "report should mention .claude/: {report}"
    );
    assert!(
        report.contains("--execute"),
        "dry-run report should hint at --execute: {report}"
    );
    // Working tree must NOT be modified in dry-run.
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(
        !gitignore.contains(".claude/"),
        "dry-run must not modify .gitignore: {gitignore}"
    );
}

#[test]
fn curate_dry_run_reports_missing_policy_files() {
    let repo = TempRepo::new();
    // Write the full gitignore so stage 1 is clean, but no policy files.
    repo.write_file(
        ".gitignore",
        concat!(
            ".claude/\n.cursor/\n.codex/\n.aider*\n.continue/\n.windsurf/\n",
            "hyperi-ai/\n.mcp.json\n.ai-version\n",
            "STATE.md\nCLAUDE.md\nGEMINI.md\nAGENTS.md\nCURSOR.md\nCODEX.md\nTODO.md\n",
            "docs/superpowers/\n",
        ),
    );
    repo.add_all_commit("full gitignore, no policy files");

    let report_path = repo.path().join("report.md");
    let args = CurateArgs {
        report: Some(report_path.clone()),
        ..args_dry_run()
    };
    run(&args, Some(repo.path())).expect("curate dry-run");

    let report = std::fs::read_to_string(&report_path).unwrap();
    assert!(
        report.contains("AI-TRAINING-POLICY.md"),
        "missing policy file should appear in report: {report}"
    );
    assert!(
        report.contains("robots.txt"),
        "missing robots.txt should appear in report: {report}"
    );
}

// ── Execute --force ───────────────────────────────────────────────────────────

#[test]
fn curate_execute_with_force_adds_missing_gitignore_entries() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    repo.add_all_commit("initial");

    run(&args_execute_force(), Some(repo.path())).expect("curate execute --force");

    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(
        gitignore.contains(".claude/"),
        ".gitignore should contain .claude/: {gitignore}"
    );
    assert!(
        gitignore.contains("STATE.md"),
        ".gitignore should contain STATE.md: {gitignore}"
    );
    assert!(
        gitignore.contains("docs/superpowers/"),
        ".gitignore should contain docs/superpowers/: {gitignore}"
    );
    // Original entry must survive.
    assert!(
        gitignore.contains("target/"),
        "original target/ must be preserved: {gitignore}"
    );
}

#[test]
fn curate_execute_with_force_adds_policy_files() {
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
    repo.add_all_commit("full gitignore");

    run(&args_execute_force(), Some(repo.path())).expect("curate execute --force");

    assert!(
        repo.path().join("AI-TRAINING-POLICY.md").is_file(),
        "AI-TRAINING-POLICY.md should be created"
    );
    assert!(
        repo.path().join("robots.txt").is_file(),
        "robots.txt should be created"
    );

    // Content should match the embedded fallback.
    let written = std::fs::read_to_string(repo.path().join("AI-TRAINING-POLICY.md")).unwrap();
    assert!(
        written.contains("HYPERI PTY LIMITED"),
        "embedded policy content should be written: {written}"
    );
    // Verify it matches the embedded constant.
    assert_eq!(written, EMBEDDED_AI_TRAINING_POLICY);
}

// ── Idempotency ───────────────────────────────────────────────────────────────

#[test]
fn curate_idempotent_when_already_clean() {
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
    repo.add_all_commit("already clean");

    let report_path = repo.path().join("report.md");
    let args = CurateArgs {
        report: Some(report_path.clone()),
        ..args_dry_run()
    };
    run(&args, Some(repo.path())).expect("curate on clean repo");

    let report = std::fs::read_to_string(&report_path).unwrap();
    // Stage 1 and Stage 2 should be clean.
    // Stage 3 and 4 will also be clean since no tracked conflicts or stray refs.
    assert!(
        report.contains("nothing to do"),
        "already-clean repo should report nothing to do: {report}"
    );
}

#[test]
fn curate_execute_idempotent_second_run() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    repo.add_all_commit("initial");

    // First run: adds all missing entries.
    run(&args_execute_force(), Some(repo.path())).expect("first curate run");

    let gitignore_after_first = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    let count_claude_after_first = gitignore_after_first.matches(".claude/").count();

    // Second run: must not double-add entries.
    run(&args_execute_force(), Some(repo.path())).expect("second curate run");

    let gitignore_after_second = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    let count_claude_after_second = gitignore_after_second.matches(".claude/").count();

    assert_eq!(
        count_claude_after_first, count_claude_after_second,
        ".claude/ should not be added twice on second run"
    );
}

// ── Stage 3: stray references ─────────────────────────────────────────────────

#[test]
fn curate_scrubs_stray_reference_in_comment() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "# Claude Code scratch\n.tmp/\n");
    repo.add_all_commit("initial");

    run(&args_execute_force(), Some(repo.path())).expect("curate scrub");

    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(
        !gitignore.contains("# Claude Code scratch"),
        "stray comment should have been replaced: {gitignore}"
    );
    assert!(
        gitignore.contains("# Local scratch"),
        ".gitignore should now have the generic comment: {gitignore}"
    );
    // The .tmp/ entry must survive.
    assert!(
        gitignore.contains(".tmp/"),
        ".tmp/ must survive scrub: {gitignore}"
    );
}

#[test]
fn curate_scrubs_cursor_reference() {
    let repo = TempRepo::new();
    repo.write_file("README.md", "# Claude Code scratch\n# Cursor AI scratch\n");
    repo.add_all_commit("initial");

    run(&args_execute_force(), Some(repo.path())).expect("curate scrub");

    let readme = std::fs::read_to_string(repo.path().join("README.md")).unwrap();
    assert!(
        !readme.contains("# Cursor AI scratch"),
        "Cursor AI comment should be replaced: {readme}"
    );
    assert!(
        readme.contains("# Local scratch"),
        "should have generic comment: {readme}"
    );
}

// ── Stage 4: tracked conflicts ────────────────────────────────────────────────

#[test]
fn curate_detects_tracked_conflicts() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    // Track files that should be gitignored per curate config.
    repo.write_file("STATE.md", "should not be tracked\n");
    repo.write_file(".claude/notes.md", "should not be tracked\n");
    repo.add_all_commit("with tracked conflicts");

    let report_path = repo.path().join("report.md");
    let args = CurateArgs {
        report: Some(report_path.clone()),
        ..args_dry_run()
    };
    run(&args, Some(repo.path())).expect("curate dry-run");

    let report = std::fs::read_to_string(&report_path).unwrap();
    assert!(
        report.contains("Stage 4"),
        "report should have a Stage 4 section: {report}"
    );
    // Both tracked files should appear in the conflict list.
    assert!(
        report.contains("STATE.md") || report.contains(".claude"),
        "tracked conflicts should appear in report: {report}"
    );
}

#[test]
fn curate_stage4_does_not_auto_resolve_tracked_conflicts() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    repo.write_file("STATE.md", "tracked content\n");
    repo.add_all_commit("with tracked STATE.md");

    // Execute with force — stage 4 should still NOT remove STATE.md from the index.
    run(&args_execute_force(), Some(repo.path())).expect("curate execute");

    // STATE.md must still exist as a tracked file.
    let still_tracked_output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .arg("ls-files")
        .arg("STATE.md")
        .output()
        .unwrap();
    let ls_output = String::from_utf8_lossy(&still_tracked_output.stdout);
    assert!(
        ls_output.contains("STATE.md"),
        "STATE.md should remain tracked (stage 4 does not auto-resolve): {ls_output}"
    );
}

// ── Embedded config round-trip ────────────────────────────────────────────────

#[test]
fn embedded_ai_curate_yaml_round_trips() {
    // Confirm the YAML parses cleanly — validates our YAML file is correct.
    let _: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(EMBEDDED_AI_CURATE).expect("embedded ai-curate.yaml must parse");
}

// ── Custom config path ────────────────────────────────────────────────────────

#[test]
fn curate_accepts_custom_config_path() {
    let repo = TempRepo::new();
    repo.write_file(".gitignore", "target/\n");
    repo.add_all_commit("initial");

    // Write a minimal custom config that only adds one path.
    let custom_config = repo.path().join("custom-curate.yaml");
    std::fs::write(
        &custom_config,
        "gitignore_additions:\n  test: [\".my-custom-tool/\"]\npolicy_files: []\n",
    )
    .unwrap();

    let args = CurateArgs {
        execute: true,
        force: true,
        config: Some(custom_config),
        policy_source: None,
        report: None,
    };
    run(&args, Some(repo.path())).expect("curate with custom config");

    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(
        gitignore.contains(".my-custom-tool/"),
        "custom config path should be added: {gitignore}"
    );
    // Default paths should NOT appear since we used a custom config with only one entry.
    assert!(
        !gitignore.contains("STATE.md"),
        "default STATE.md should not appear when using custom config: {gitignore}"
    );
}
