//  Project:      git-scrub
//  File:         tests/e2e/ai_curate_execute.rs
//  Purpose:      E2e: ai curate --execute --force against a minimal fresh repo.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! `ai curate --execute --force` end-to-end test.
//!
//! Curate operates on the working tree, not on history, so the fixture corpus
//! is not needed. This test creates a minimal repo with an incomplete
//! `.gitignore` and verifies that `ai curate --execute --force` adds the
//! expected entries and creates the canonical policy files.

#[test]
fn ai_curate_execute_force_applies_all_stages() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path();

    // Init a minimal repo with a .gitignore that is MISSING the curate entries.
    std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir)
        .status()?;
    std::fs::write(dir.join(".gitignore"), "target/\n")?;
    std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "add",
            ".gitignore",
        ])
        .current_dir(dir)
        .status()?;
    std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ])
        .current_dir(dir)
        .status()?;

    // Run curate --execute --force. The --force flag skips TTY prompts and
    // applies all stages automatically. stdin will not be a TTY in the test
    // runner, so without --force the stages would all be skipped.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_git-scrub"))
        .args([
            "-C",
            &dir.display().to_string(),
            "ai",
            "curate",
            "--execute",
            "--force",
        ])
        .output()?;

    anyhow::ensure!(
        out.status.success(),
        "ai curate --execute --force failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // Stage 1: .gitignore should now contain the curate entries.
    let gitignore = std::fs::read_to_string(dir.join(".gitignore"))?;
    assert!(
        gitignore.contains(".claude/"),
        ".gitignore should contain '.claude/' after curate; content:\n{gitignore}",
    );
    assert!(
        gitignore.contains("STATE.md"),
        ".gitignore should contain 'STATE.md' after curate; content:\n{gitignore}",
    );
    assert!(
        gitignore.contains("CLAUDE.md"),
        ".gitignore should contain 'CLAUDE.md' after curate; content:\n{gitignore}",
    );

    // Stage 2: canonical policy files should exist at the repo root.
    assert!(
        dir.join("AI-TRAINING-POLICY.md").exists(),
        "AI-TRAINING-POLICY.md should have been created by curate",
    );
    assert!(
        dir.join("robots.txt").exists(),
        "robots.txt should have been created by curate",
    );

    Ok(())
}
