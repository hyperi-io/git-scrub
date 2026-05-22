# git-scrub

Surgical removal of unwanted content from git history. Two use cases:

- **AI scrub** (`git scrub ai` / `git-scrub ai`) — remove AI tool residue:
  artefact files (`.claude/`, `.cursor/`, `.codex/`, …) and attribution
  trailers (`Co-Authored-By: Claude`, `Co-authored-by: Cursor`,
  `🤖 Generated with [Claude Code]`, …). Opinionated pattern library
  covers Claude, Copilot, Cursor, Codex, Aider, Gemini, WindSurf.
- **Spill scrub** (`git scrub spill` / `git-scrub spill`) *(v2)* —
  incident response: remove a specific file / path / text pattern that
  "got away" past the preventive layers. Operator supplies what to
  remove; the tool provides safety scaffolding + runbook.

## Name and invocation

The binary is `git-scrub` (hyphenated). Git's third-party subcommand
convention is strict: an executable named `git-<name>` on `PATH` is
invokable as both `git-<name>` and as the git-native subcommand
`git <name>`. So once installed:

```
git scrub ai --dry-run
git scrub ai --execute
git-scrub --help
```

are equivalent surfaces. Pick whichever you prefer.

## Status

**Pre-alpha.** Scaffolded 2026-05-13. No release yet.

What works today (cross-platform: Linux + macOS + Windows):

- `git scrub ai composite` — strip attribution trailers AND drop AI
  artefact files in a single engine pass
- `git scrub ai attribution` — attribution trailers only
- `git scrub ai files` — artefact files only
- `git scrub ai patterns` — dump the active pattern library
- Dry-run by default; `--execute` to actually rewrite
- Mandatory mirror-clone backup to the platform cache dir before any
  rewrite (override via `--backup-to`; bypass via `--no-backup
  --really-no-backup-i-mean-it`)
- Post-rewrite verify step — re-grep the scrubbed history for the
  configured patterns and hard-fail if anything remains
- Markdown runbook written to `<platform-cache-dir>/git-scrub/runbooks/`
  with the plan, backup location, force-push commands, and (when `gh`
  is on PATH and authenticated) fork list, closed-PR list, branch-
  protection state, GitHub Support contact template, and collaborator
  notification template

What's not done yet (still v1):

- Distribution channels — see *Distribution* below
- `gh` integration for in-flight Actions check (refuse rewrite if a
  workflow is touching the affected branches)

What's deferred (v2):

- Spill scrub (`spill paths`, `spill text`, `spill secrets`)
- Hand-rolled fast-export parser fallback to `git-filter-repo` for
  unusual record forms

See [`docs/superpowers/plans/2026-05-13-git-scrub.md`](docs/superpowers/plans/2026-05-13-git-scrub.md)
for the full design. Note: that path is gitignored locally; the canonical
design also lives in the conversation it came from.

## Design tenets

- **Standalone:** works on any git repo. No project-config dependency,
  no language-detection, no external infrastructure.
- **Opinionated patterns are YAML, not Rust code.** All pattern data
  ships as YAML files deployed with the binary. Operators can override
  via `--config`. Editing a YAML file (and rebuilding, or pointing at
  it via `--config`) changes behaviour — patterns are first-class
  config, never hard-coded.
- **Safe by default.** Dry-run is the default. Mandatory backup mirror
  before any rewrite. Verify pass after rewrite. Tool **never pushes**
  for you — emits force-push instructions in the runbook.
- **Minimal dependencies.** Single static Rust binary. No Python
  prerequisite. No hyperi-rustlib pull-in. Engine works via
  `git fast-export | transform | git fast-import` directly.

## Runtime prerequisite

`git` must be on `PATH` on every platform. On Windows, the standard
source is [Git for Windows](https://gitforwindows.org/), which provides
`git.exe` and the `fast-export` / `fast-import` plumbing git-scrub
shells out to. The optional [`gh`](https://cli.github.com/) CLI enables
fork enumeration, closed-PR enumeration, and branch-protection probes
in the runbook — install and `gh auth login` to unlock those sections.

## Distribution

Planned channels (none shipping yet):

| Channel | Platform | Status |
|---|---|---|
| `cargo install git-scrub` | Linux / macOS / Windows | planned for v1 |
| Homebrew tap (`brew install hyperi-io/git-scrub/git-scrub`) | macOS / Linux | planned for v1.x |
| `.deb` via `cargo-deb` | Debian / Ubuntu | planned for v1.x |
| `.rpm` via `cargo-generate-rpm` | Fedora / RHEL | planned for v1.x |
| winget manifest | Windows | planned for v1.x |
| Scoop bucket entry | Windows | planned for v1.x |
| Chocolatey package | Windows | planned for v1.x |
| Curl / PowerShell installer scripts | all | planned for v1.x |

v1 ships `cargo install` only. Native packages follow once the binary
surface stabilises.

## Building from source

```bash
cargo build --release
./target/release/git-scrub --help
```

Cross-platform CI runs on every push: Linux, macOS, and Windows.

## For agentic coders — clean up after yourself

If you are an AI coding agent (Claude Code, Cursor, Copilot, Codex,
Aider, …) that has been working in someone else's repository, you
have probably:

- written working files into `.claude/` / `.cursor/` / `.codex/` / etc.
- written or generated `STATE.md`, `TODO.md`, `CLAUDE.md` and friends
- left attribution trailers (`Co-Authored-By: <agent>`) in commits
- introduced lockfile entries or vendored sources you shouldn't have
- referenced your own toolchain by name in comments or docstrings

`git-scrub` exists so you can find that mess and tidy it up before
handing the repo back.

### Workflow (audit → fix → verify)

```bash
# 1. Audit. Read-only. One command, full picture.
git-scrub audit
# Or scope it: git-scrub audit --ai --curate

# 2. Fix the working tree (gitignore, policy files, stray references).
#    Defaults to interactive; --force skips all prompts.
git-scrub ai curate --execute
# Or in CI / scripted use:
git-scrub ai curate --execute --force

# 3. Fix history (attribution trailers + artefact files).
git-scrub ai composite --execute
# Or just one or the other:
git-scrub ai attribution --execute
git-scrub ai files --execute

# 4. Re-audit. Expect "no issues found".
git-scrub audit
```

Every `--execute` subcommand:

- runs dry-run by default — invoke without `--execute` (or with the
  cosmetic `--audit` flag) to see what would happen
- takes a mandatory mirror-clone backup before rewriting history
- runs a post-rewrite verify pass and hard-fails if any pattern still
  matches
- writes a markdown runbook to the platform cache directory with the
  force-push commands and the rest of the manual follow-up checklist
- never pushes for you — the runbook tells the operator what to run

### The audit report

`git-scrub audit` emits one consolidated markdown report covering:

- **AI residue (history)** — commits with attribution trailers, file
  operations touching artefact paths
- **Supply chain (history)** — lockfile blobs matching the bundled
  advisory snapshot
- **Working-tree curation** — `.gitignore` entries missing, policy
  files missing (`AI-TRAINING-POLICY.md`, `robots.txt`), stray
  tool-specific references in comments, files currently tracked that
  should be gitignored
- **Summary** — exactly the `--execute` commands to run next

Pipe it to a file: `git-scrub audit --report audit.md`. Scope it down
with `--ai`, `--supply`, `--curate`.

### Constraints to respect

- **Read the runbook before pushing.** The tool prints the
  `--force-with-lease` push commands; you should not run them
  unattended. Forks and `refs/pull/N/head` references are out of the
  tool's reach — the runbook lists them.
- **Don't push the AI working files back.** After running `ai curate`
  and `ai composite`, `.gitignore` will keep them out — but verify
  with `git status` before the next commit.
- **Don't co-author humans.** Never add `Co-Authored-By:` trailers
  for the human operator in commits you author; do not name yourself
  in commit messages. The cleanup commands look for those exact
  patterns and will remove them on the next pass.
- **Audit, don't trust.** Run `git-scrub audit` again after any
  large action to confirm the repo is clean.

## License

Apache-2.0. Copyright HYPERI PTY LIMITED.
