//  Project:      git-scrub
//  File:         src/plan.rs
//  Purpose:      Markdown plan emission for --dry-run.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Markdown plan output.
//!
//! Emitted both to stdout and (later) to the runbook file. Single source of
//! truth for what the operator is about to do.

use std::fmt::Write;

use crate::engine::EngineStats;
use crate::patterns::discovery::Source;
use crate::preflight::PreflightReport;

/// Inputs to plan generation.
#[derive(Debug)]
pub struct PlanInputs<'a> {
    /// Which use-case (`ai`, `ai attribution`, `ai files`).
    pub use_case: &'a str,
    /// Where the attribution YAML came from.
    pub attribution_source: Option<Source>,
    /// Where the files YAML came from.
    pub files_source: Option<Source>,
    /// Preflight observations.
    pub preflight: &'a PreflightReport,
    /// Scan statistics (read-only count of what would change).
    pub stats: &'a EngineStats,
    /// Whether this is a dry-run or execute pass.
    pub dry_run: bool,
}

/// Render a markdown plan into a `String`.
#[must_use]
pub fn render(inputs: &PlanInputs<'_>) -> String {
    let mut out = String::with_capacity(2048);
    let _ = writeln!(out, "# git-scrub plan");
    let _ = writeln!(out);
    let _ = writeln!(out, "**Use case:** `{}`", inputs.use_case);
    let _ = writeln!(
        out,
        "**Mode:** {}",
        if inputs.dry_run {
            "dry-run (no changes will be made)"
        } else {
            "execute"
        }
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## What changes");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Statistic | Count |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| Commits scanned | {} |", inputs.stats.commits_seen);
    let _ = writeln!(
        out,
        "| Commits whose message would be rewritten | {} |",
        inputs.stats.commits_rewritten
    );
    let _ = writeln!(
        out,
        "| File operations that would be dropped | {} |",
        inputs.stats.file_ops_dropped
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## Pattern library sources");
    let _ = writeln!(out);
    if let Some(s) = inputs.attribution_source {
        let _ = writeln!(out, "- Attribution: {}", s.label());
    }
    if let Some(s) = inputs.files_source {
        let _ = writeln!(out, "- Files: {}", s.label());
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Pre-flight findings");
    let _ = writeln!(out);
    let _ = writeln!(out, "- Branches: {}", inputs.preflight.branch_count);
    if inputs.preflight.submodules.is_empty() {
        let _ = writeln!(out, "- Submodules: none");
    } else {
        let _ = writeln!(out, "- Submodules ({}):", inputs.preflight.submodules.len());
        for s in &inputs.preflight.submodules {
            let _ = writeln!(out, "  - `{s}`");
        }
        let _ = writeln!(
            out,
            "  - Note: rewriting the parent rewrites submodule POINTERS \
             only. Submodule histories are not touched."
        );
    }
    if inputs.preflight.has_signed_commits {
        let _ = writeln!(
            out,
            "- Signed commits detected — signatures WILL break on rewrite. \
             Re-signing requires the original maintainer's key (out of scope)."
        );
    } else {
        let _ = writeln!(out, "- Signed commits: none detected");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Hard truths");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Force-push does NOT delete on GitHub. Old SHAs remain URL-reachable \
         until upstream GC. Treat any leaked secret as already compromised."
    );
    let _ = writeln!(
        out,
        "- `refs/pull/N/head` is immutable. Commits on closed PR branches \
         stay reachable via PR diff URLs. GitHub Support is required."
    );
    let _ = writeln!(
        out,
        "- Forks are independent clones. Public-repo cleanup is inherently \
         lossy — the runbook will list forks and provide a notification \
         template."
    );
    out
}
