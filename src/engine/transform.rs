//  Project:      git-scrub
//  File:         src/engine/transform.rs
//  Purpose:      Stream-parse a git fast-export stream and apply pattern transforms.
//  Language:     Rust
//
//  License:      Apache-2.0
//  Copyright:    (c) 2026 HYPERI PTY LIMITED

//! Streaming fast-export transformer.
//!
//! Reads byte-by-byte from a `git fast-export` stdout pipe, applies
//! attribution-pattern rewriting to commit / tag messages, drops file
//! operations matching purge patterns, and writes the result to a
//! `git fast-import` stdin pipe.
//!
//! ## Stream format (minimal subset we handle)
//!
//! - `blob` records: content is cached by mark for path-aware rewriting.
//!   Each blob is emitted deferred — just before the commit that first
//!   references it (all blobs before a commit are emitted before the commit
//!   header). This preserves fast-import's requirement that `M :<mark>` lines
//!   appear after the blob with that mark.
//! - `commit` records: rewrite the `data N` message block; filter `M D R C`
//!   file ops by path; apply `LockfileRewriter` to mark-referenced blobs.
//! - `tag` records: rewrite the `data N` annotation message.
//! - `reset`, `feature`, `option`, `done`, `progress`, `checkpoint`,
//!   `ls`, `cat-blob`, `get-mark`: passthrough verbatim.
//!
//! Anything we don't recognise passes through verbatim. We never decode
//! UTF-8; the stream is bytes end-to-end. Commit messages MAY contain
//! non-UTF-8 sequences.
//!
//! ## Blob emission order
//!
//! git fast-import requires that a `blob mark :N` record appears BEFORE
//! any `M :<N>` reference. Because `LockfileRewriter` is path-aware and we
//! don't know which path a blob will be committed at until we see the commit,
//! we buffer the entire commit record and emit all needed blob records (both
//! original and rewritten) immediately before the commit header.
//!
//! ## Inline blobs
//!
//! `M <mode> inline <path>` followed by a `data N` block are direct inline
//! blobs with no mark. They pass through unchanged in v1 — `LockfileRewriter`
//! does not apply to them.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};

use super::{BlobCache, EngineError, EngineStats};
use crate::patterns::{AttributionRewriter, BlobRewriter, FileMatcher, LockfileRewriter};

/// Drive the transform stage from the given reader to the given writer.
///
/// Returns the per-pass statistics for plan / runbook output.
pub fn run_stream<R: Read, W: Write>(
    reader: R,
    mut writer: W,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
    lockfiles: Option<&[Box<dyn LockfileRewriter>]>,
) -> Result<EngineStats, EngineError> {
    let mut br = BufReader::new(reader);
    let mut stats = EngineStats::default();
    let mut blob_cache = BlobCache::new();
    // Tracks which blob marks have already been written to the output stream.
    // Once a blob is emitted (with its original mark), fast-import remembers
    // it — subsequent M-lines can re-reference the same mark without re-emitting.
    let mut emitted_marks: HashSet<u64> = HashSet::new();
    // Next mark number to use when allocating synthetic marks for rewritten blobs.
    // Updated whenever we see a `mark :N` in the stream so new marks never collide.
    let mut next_mark: u64 = 1;

    loop {
        let mut line: Vec<u8> = Vec::with_capacity(128);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            break; // EOF
        }

        match record_kind(&line) {
            RecordKind::Commit => {
                stats.commits_seen += 1;
                process_commit(
                    &line,
                    &mut br,
                    &mut writer,
                    attribution,
                    files,
                    blob,
                    lockfiles,
                    &mut blob_cache,
                    &mut emitted_marks,
                    &mut next_mark,
                    &mut stats,
                )?;
            }
            RecordKind::Tag => {
                process_tag(&line, &mut br, &mut writer, attribution)?;
            }
            RecordKind::Blob => {
                stats.blobs_seen += 1;
                cache_blob(
                    &line,
                    &mut br,
                    &mut blob_cache,
                    &mut next_mark,
                    blob,
                    &mut stats,
                )?;
            }
            RecordKind::Other => {
                writer.write_all(&line).map_err(EngineError::Io)?;
            }
        }
    }

    writer.flush().map_err(EngineError::Io)?;
    Ok(stats)
}

#[derive(Debug, Clone, Copy)]
enum RecordKind {
    Commit,
    Tag,
    Blob,
    Other,
}

fn record_kind(line: &[u8]) -> RecordKind {
    if line.starts_with(b"commit ") {
        RecordKind::Commit
    } else if line.starts_with(b"tag ") {
        RecordKind::Tag
    } else if line == b"blob\n" || line.starts_with(b"blob ") {
        RecordKind::Blob
    } else {
        RecordKind::Other
    }
}

/// Parse a mark line of the form `mark :N\n` and return `N`.
/// Returns `None` if the line does not match.
fn parse_mark_line(line: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(line).ok()?;
    let s = s.strip_prefix("mark :")?;
    let s = s.trim_end_matches('\n');
    s.parse::<u64>().ok()
}

/// Cache a blob record without emitting it.
///
/// Consumes the `blob\n` header and all sub-lines up to and including the
/// `data N` block. The blob content (optionally rewritten by `blob_rewriter`)
/// is stored in `blob_cache` keyed by its mark. `next_mark` is updated so
/// new marks never collide. When `blob_rewriter` produces new content,
/// `stats.blobs_rewritten` is incremented.
///
/// # Errors
///
/// Returns `EngineError` on I/O failure or a malformed data line.
fn cache_blob<R: BufRead>(
    _header: &[u8],
    br: &mut R,
    blob_cache: &mut BlobCache,
    next_mark: &mut u64,
    blob: Option<&BlobRewriter>,
    stats: &mut EngineStats,
) -> Result<(), EngineError> {
    let mut mark: Option<u64> = None;

    loop {
        let mut line: Vec<u8> = Vec::with_capacity(64);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            // EOF before data block — nothing to cache.
            return Ok(());
        }
        if line.starts_with(b"data ") {
            let len = parse_data_len(&line)?;
            let mut raw = vec![0u8; len];
            br.read_exact(&mut raw).map_err(EngineError::Io)?;
            consume_optional_lf(br)?;
            // Apply BlobRewriter (content-based, not path-aware) at cache time.
            let (content, changed) = match blob {
                Some(r) if !r.is_empty() => {
                    let (cow, changed) = r.apply(&raw);
                    (cow.into_owned(), changed)
                }
                _ => (raw, false),
            };
            if changed {
                stats.blobs_rewritten += 1;
            }
            if let Some(m) = mark {
                blob_cache.store(m, content);
            }
            return Ok(());
        }
        if let Some(m) = parse_mark_line(&line) {
            mark = Some(m);
            // Keep next_mark ahead of all seen marks so new marks don't collide.
            if m >= *next_mark {
                *next_mark = m + 1;
            }
        }
        // Other sub-lines (original-oid, etc.) are discarded; they will be
        // reconstructed or omitted when the blob is re-emitted from the cache.
    }
}

/// Emit a cached blob record to `writer`.
///
/// Writes:
/// ```text
/// blob
/// mark :<mark>
/// data <len>
/// <content>
/// ```
fn emit_cached_blob<W: Write>(
    writer: &mut W,
    mark: u64,
    content: &[u8],
) -> Result<(), EngineError> {
    writer.write_all(b"blob\n").map_err(EngineError::Io)?;
    writeln!(writer, "mark :{mark}").map_err(EngineError::Io)?;
    writeln!(writer, "data {}", content.len()).map_err(EngineError::Io)?;
    writer.write_all(content).map_err(EngineError::Io)?;
    writer.write_all(b"\n").map_err(EngineError::Io)?;
    Ok(())
}

/// An item in a commit's post-data section.
///
/// Most lines are plain `Line` items. Inline blob data blocks
/// (`M <mode> inline <path>` followed by `data N\n<bytes>`) are stored as a
/// `InlineData` pair so we can skip or emit them atomically.
enum PostItem {
    Line(Vec<u8>),
    /// Inline data: the `data N\n` header and the N raw content bytes.
    InlineData(Vec<u8>, Vec<u8>),
}

/// The result of planning a commit's M-line rewrites.
struct MLinePlan {
    /// The M-line bytes to emit (possibly with a new mark).
    m_line: Vec<u8>,
    /// If this M-line needs a new rewritten blob emitted before the commit,
    /// this holds `(new_mark, new_content)`.
    new_blob: Option<(u64, Vec<u8>)>,
    /// The original mark referenced (if any), used to track emission.
    original_mark: Option<u64>,
}

// Reads pre-data header lines (mark, original-oid, author, committer, etc.)
// and the mandatory `data N` message block. Returns the pre-data lines,
// the (possibly rewritten) message bytes, and whether attribution changed it.
#[allow(clippy::type_complexity)]
fn read_commit_pre_data<R: BufRead>(
    br: &mut R,
    attribution: Option<&AttributionRewriter>,
) -> Result<(Vec<Vec<u8>>, Vec<u8>, bool), EngineError> {
    let mut pre_data: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut line: Vec<u8> = Vec::with_capacity(128);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            return Err(EngineError::StreamParse {
                offset: 0,
                message: "unexpected EOF inside commit header".to_string(),
            });
        }
        if line.starts_with(b"data ") {
            let len = parse_data_len(&line)?;
            let mut buf = vec![0u8; len];
            br.read_exact(&mut buf).map_err(EngineError::Io)?;
            consume_optional_lf(br)?;
            let rewritten = if let Some(r) = attribution {
                r.apply(&buf)
            } else {
                buf.clone()
            };
            let changed = rewritten != buf;
            return Ok((pre_data, rewritten, changed));
        }
        pre_data.push(line);
    }
}

/// Process a `commit` record.
///
/// Buffers the full commit, plans blob emissions, then flushes:
/// `(pre-commit blobs) + (commit record)`.
///
/// git fast-import requires blob records with mark `:N` to appear BEFORE any
/// `M :<N>` reference. Because `LockfileRewriter` is path-aware and we don't
/// know the path until we see the commit, we buffer the whole record first.
#[allow(clippy::too_many_arguments)]
fn process_commit<R: BufRead, W: Write>(
    header: &[u8],
    br: &mut R,
    writer: &mut W,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
    lockfiles: Option<&[Box<dyn LockfileRewriter>]>,
    blob_cache: &mut BlobCache,
    emitted_marks: &mut HashSet<u64>,
    next_mark: &mut u64,
    stats: &mut EngineStats,
) -> Result<(), EngineError> {
    // --- Phase 1: buffer commit lines until the record ends. ---

    // Pre-data header lines (mark, original-oid, author, committer, encoding).
    // Message bytes and whether attribution changed it.
    let (pre_data, msg_bytes, msg_rewritten) = read_commit_pre_data(br, attribution)?;
    // Post-data items (from, merge, file ops, blank terminator, or EOF sentinel).
    let mut post_lines: Vec<PostItem> = Vec::new();
    // Whether the commit was terminated by a blank line (vs EOF or next record).
    let mut trailing_blank = false;
    // The next top-level record we consumed without a blank separator (if any).
    let mut lookahead: Option<Vec<u8>> = None;

    // Read post-data items (file ops, from, merge, blank, or next record).
    // When we encounter an inline M-line, the next item is a `data N` block
    // whose bytes must be read atomically to avoid mis-parsing.
    let mut next_is_inline_data = false;
    loop {
        let mut line: Vec<u8> = Vec::with_capacity(128);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            break; // EOF
        }
        if line == b"\n" {
            trailing_blank = true;
            break;
        }
        if matches!(
            record_kind(&line),
            RecordKind::Commit | RecordKind::Tag | RecordKind::Blob
        ) {
            // Next top-level record without a blank separator — keep it for re-dispatch.
            lookahead = Some(line);
            break;
        }
        if next_is_inline_data && line.starts_with(b"data ") {
            // Read the inline data block.
            let len = parse_data_len(&line)?;
            let mut data_bytes = vec![0u8; len];
            br.read_exact(&mut data_bytes).map_err(EngineError::Io)?;
            consume_optional_lf(br)?;
            post_lines.push(PostItem::InlineData(line, data_bytes));
            next_is_inline_data = false;
            continue;
        }
        // Check if this line is an inline M-line.
        next_is_inline_data =
            line.starts_with(b"M ") && line.windows(8).any(|w| w == b" inline ");
        post_lines.push(PostItem::Line(line));
    }

    if msg_rewritten {
        stats.commits_rewritten += 1;
    }

    // --- Phase 2: plan blob emissions for M-lines. ---
    // For each M-line that references a cached blob mark, decide whether to
    // rewrite it (LockfileRewriter) or pass through with deferred original.

    let mut m_plans: Vec<(usize, MLinePlan)> = Vec::new(); // (index in post_lines, plan)
    let mut purged_indices: HashSet<usize> = HashSet::new();

    for (i, item) in post_lines.iter().enumerate() {
        let line = match item {
            PostItem::Line(l) => l.as_slice(),
            PostItem::InlineData(..) => continue, // never a file-op directive
        };
        if !is_file_op_line(line) {
            continue;
        }
        let op = line.first().copied().unwrap_or(0);
        if line.starts_with(b"deleteall\n") || line.starts_with(b"filedeleteall\n") {
            continue;
        }
        let has_inline = op == b'M' && line.windows(8).any(|w| w == b" inline ");
        let path = extract_path(line, op);

        // Check for purge.
        let purge = files.is_some_and(|m| path.as_ref().is_some_and(|p| m.should_purge(p)));
        if purge {
            purged_indices.insert(i);
            // If inline, the InlineData item is at i+1 — also skip it.
            if has_inline && i + 1 < post_lines.len() {
                purged_indices.insert(i + 1);
            }
            stats.file_ops_dropped += 1;
            continue;
        }

        // For M-lines with mark references, plan blob emission.
        if op == b'M' && !has_inline && let Some(blob_mark) = parse_m_line_mark(line) {
            let path_str = path.as_deref().unwrap_or("");
            // Check for lockfile rewrite.
            let rewritten_content = lockfiles.and_then(|lfs| {
                lfs.iter().find_map(|lf| {
                    if lf.applies_to(path_str) {
                        let cached = blob_cache.get(blob_mark)?;
                        lf.strip(cached)
                    } else {
                        None
                    }
                })
            });
            if let Some(new_content) = rewritten_content {
                let new_mark = *next_mark;
                *next_mark += 1;
                let new_m_line = rewrite_m_line_mark(line, new_mark);
                m_plans.push((
                    i,
                    MLinePlan {
                        m_line: new_m_line,
                        new_blob: Some((new_mark, new_content)),
                        original_mark: Some(blob_mark),
                    },
                ));
                stats.blobs_rewritten += 1;
            } else {
                m_plans.push((
                    i,
                    MLinePlan {
                        m_line: line.to_vec(),
                        new_blob: None,
                        original_mark: Some(blob_mark),
                    },
                ));
            }
        }
    }

    // --- Phase 3: emit pre-commit blobs (originals not yet emitted). ---
    // Emit original blobs for all M-lines that don't have a replacement
    // (or even for replaced ones, their original mark might be needed by other
    // M-lines; but since we allocated a new mark for replacements, the original
    // mark is no longer referenced by THIS commit's M-lines for the replaced case).
    // We emit originals for non-replaced M-lines only.
    for (_, plan) in &m_plans {
        if let Some((new_mark, new_content)) = &plan.new_blob {
            // Replaced — emit the new rewritten blob before the commit.
            emit_cached_blob(writer, *new_mark, new_content)?;
        } else if let Some(orig_mark) = plan.original_mark
            && !emitted_marks.contains(&orig_mark)
            && let Some(content) = blob_cache.get(orig_mark)
        {
            // Not replaced — emit original (deferred from cache time).
            let content = content.to_vec();
            emit_cached_blob(writer, orig_mark, &content)?;
            emitted_marks.insert(orig_mark);
        }
    }

    // --- Phase 4: emit the commit. ---
    writer.write_all(header).map_err(EngineError::Io)?;
    for line in &pre_data {
        writer.write_all(line).map_err(EngineError::Io)?;
    }
    writeln!(writer, "data {}", msg_bytes.len()).map_err(EngineError::Io)?;
    writer.write_all(&msg_bytes).map_err(EngineError::Io)?;
    writer.write_all(b"\n").map_err(EngineError::Io)?;

    // Build a plan index for O(1) lookup.
    let plan_map: std::collections::HashMap<usize, &MLinePlan> =
        m_plans.iter().map(|(i, p)| (*i, p)).collect();

    // Emit post-data items (file ops etc.), applying purge and M-line rewrites.
    for (i, item) in post_lines.iter().enumerate() {
        if purged_indices.contains(&i) {
            continue; // skip purged M-lines and their associated InlineData blocks
        }
        match item {
            PostItem::Line(line) => {
                if let Some(plan) = plan_map.get(&i) {
                    writer.write_all(&plan.m_line).map_err(EngineError::Io)?;
                } else {
                    writer.write_all(line).map_err(EngineError::Io)?;
                }
            }
            PostItem::InlineData(header, data_bytes) => {
                // Pass through inline data (not purged — purged ones are skipped above).
                writer.write_all(header).map_err(EngineError::Io)?;
                writer.write_all(data_bytes).map_err(EngineError::Io)?;
                writer.write_all(b"\n").map_err(EngineError::Io)?;
            }
        }
    }

    if trailing_blank {
        writer.write_all(b"\n").map_err(EngineError::Io)?;
    }

    // --- Phase 5: handle lookahead (next top-level record). ---
    if let Some(la) = lookahead {
        match record_kind(&la) {
            RecordKind::Commit => {
                stats.commits_seen += 1;
                process_commit(
                    &la,
                    br,
                    writer,
                    attribution,
                    files,
                    blob,
                    lockfiles,
                    blob_cache,
                    emitted_marks,
                    next_mark,
                    stats,
                )?;
            }
            RecordKind::Tag => process_tag(&la, br, writer, attribution)?,
            RecordKind::Blob => {
                stats.blobs_seen += 1;
                cache_blob(&la, br, blob_cache, next_mark, blob, stats)?;
            }
            RecordKind::Other => writer.write_all(&la).map_err(EngineError::Io)?,
        }
    }

    Ok(())
}

fn process_tag<R: BufRead, W: Write>(
    header: &[u8],
    br: &mut R,
    writer: &mut W,
    attribution: Option<&AttributionRewriter>,
) -> Result<(), EngineError> {
    writer.write_all(header).map_err(EngineError::Io)?;
    let mut stats = EngineStats::default();
    loop {
        let mut line: Vec<u8> = Vec::with_capacity(128);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            return Ok(());
        }
        if line.starts_with(b"data ") {
            rewrite_data_block(
                &line,
                br,
                writer,
                attribution,
                &mut stats,
                /*is_commit=*/ false,
            )?;
            return Ok(());
        }
        writer.write_all(&line).map_err(EngineError::Io)?;
    }
}

/// Read a `data N\n<N bytes>` block (with optional trailing LF), write it
/// back to `writer` — optionally with the commit message rewritten.
///
/// The fast-export format allows an optional LF after the data payload.
/// We consume it on input if present and always emit one on output to
/// keep the stream regular for fast-import.
fn rewrite_data_block<R: BufRead, W: Write>(
    data_line: &[u8],
    br: &mut R,
    writer: &mut W,
    attribution: Option<&AttributionRewriter>,
    stats: &mut EngineStats,
    is_commit: bool,
) -> Result<(), EngineError> {
    let len = parse_data_len(data_line)?;
    let mut buf = vec![0u8; len];
    br.read_exact(&mut buf).map_err(EngineError::Io)?;
    consume_optional_lf(br)?;

    let rewritten = if let Some(r) = attribution {
        r.apply(&buf)
    } else {
        buf.clone()
    };
    if is_commit && rewritten != buf {
        stats.commits_rewritten += 1;
    }

    writeln!(writer, "data {}", rewritten.len()).map_err(EngineError::Io)?;
    writer.write_all(&rewritten).map_err(EngineError::Io)?;
    writer.write_all(b"\n").map_err(EngineError::Io)?;
    Ok(())
}

/// Peek at the next byte and consume it if it is an LF. The fast-export
/// format permits but does not require an LF after a `data` payload.
fn consume_optional_lf<R: BufRead>(br: &mut R) -> Result<(), EngineError> {
    let buf = br.fill_buf().map_err(EngineError::Io)?;
    if buf.first() == Some(&b'\n') {
        br.consume(1);
    }
    Ok(())
}

fn parse_data_len(data_line: &[u8]) -> Result<usize, EngineError> {
    // Expect "data N\n" — N is decimal. We do not support the rare
    // `data << EOT\n...EOT\n` form; falling back to filter-repo is the
    // documented escape hatch.
    let s = std::str::from_utf8(data_line).map_err(|_| EngineError::StreamParse {
        offset: 0,
        message: "data line not UTF-8".into(),
    })?;
    let s = s
        .strip_prefix("data ")
        .ok_or_else(|| EngineError::StreamParse {
            offset: 0,
            message: format!("malformed data line: {s:?}"),
        })?;
    let s = s.trim_end_matches('\n');
    if let Some(rest) = s.strip_prefix("<< ") {
        return Err(EngineError::StreamParse {
            offset: 0,
            message: format!(
                "heredoc-style `data << {rest}` blocks are not supported in v1 \
                 (filter-repo fallback required)"
            ),
        });
    }
    s.parse::<usize>().map_err(|e| EngineError::StreamParse {
        offset: 0,
        message: format!("bad data length {s:?}: {e}"),
    })
}

fn is_file_op_line(line: &[u8]) -> bool {
    matches!(line.first(), Some(b'M' | b'D' | b'R' | b'C' | b'd'))
        && matches!(line.get(1), Some(b' ' | b'e'))
        || line.starts_with(b"deleteall\n")
        || line.starts_with(b"filedeleteall\n")
}

/// Try to parse the mark from an `M <mode> :<mark> <path>` line.
///
/// Returns `Some(mark)` only for the `:<mark>` dataref form. Returns `None`
/// for `inline`, `deleteall`, or anything else that carries no mark.
fn parse_m_line_mark(line: &[u8]) -> Option<u64> {
    // Expect exactly: M <mode> :<mark> <path>\n
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let s = std::str::from_utf8(body).ok()?;
    if !s.starts_with("M ") {
        return None;
    }
    let mut parts = s.splitn(4, ' ');
    let _m = parts.next()?; // "M"
    let _mode = parts.next()?; // mode
    let dataref = parts.next()?; // ":<mark>" or "inline" or sha
    let dataref = dataref.strip_prefix(':')?;
    dataref.parse::<u64>().ok()
}

/// Rewrite the mark reference in an `M <mode> :<old_mark> <path>\n` line to
/// use `new_mark`.
fn rewrite_m_line_mark(line: &[u8], new_mark: u64) -> Vec<u8> {
    // Parse as UTF-8 — M lines are always ASCII.
    let Ok(s) = std::str::from_utf8(line) else {
        return line.to_vec();
    };
    let s = s.strip_suffix('\n').unwrap_or(s);
    // Format: M <mode> :<mark> <path>
    let mut parts = s.splitn(4, ' ');
    let Some(m) = parts.next() else {
        return line.to_vec();
    };
    let Some(mode) = parts.next() else {
        return line.to_vec();
    };
    let Some(_old_dataref) = parts.next() else {
        return line.to_vec();
    };
    let Some(path) = parts.next() else {
        return line.to_vec();
    };
    format!("{m} {mode} :{new_mark} {path}\n").into_bytes()
}

/// Extract the path from a file-op line for matching. Returns `None` if we
/// can't confidently parse (in which case we pass the op through).
///
/// Formats:
/// - `M <mode> <dataref> <path>\n`
/// - `D <path>\n`
/// - `R <from> <to>\n`  → match on the destination path
/// - `C <from> <to>\n`  → match on the destination path
fn extract_path(line: &[u8], op: u8) -> Option<String> {
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let s = std::str::from_utf8(body).ok()?;

    match op {
        b'D' => {
            let rest = s.strip_prefix("D ")?;
            Some(unquote_path(rest))
        }
        b'M' => {
            // M <mode> <dataref> <path>
            let mut parts = s.splitn(4, ' ');
            let _m = parts.next()?;
            let _mode = parts.next()?;
            let _dataref = parts.next()?;
            let path = parts.next()?;
            Some(unquote_path(path))
        }
        b'R' | b'C' => {
            // R <from> <to>  — paths may be quoted; quoting handles spaces.
            let rest = s.strip_prefix(&format!("{} ", op as char))?;
            let (_from, to) = split_two_paths(rest)?;
            Some(unquote_path(&to))
        }
        _ => None,
    }
}

fn split_two_paths(rest: &str) -> Option<(String, String)> {
    // If the first path is quoted, parse it as a quoted string and the rest
    // is the second path.
    if let Some(stripped) = rest.strip_prefix('"') {
        let mut buf = String::new();
        let iter = stripped.chars();
        let mut escape = false;
        let mut consumed = 1usize; // for opening quote
        let mut closed = false;
        for c in iter {
            consumed += c.len_utf8();
            if escape {
                buf.push(c);
                escape = false;
            } else if c == '\\' {
                buf.push(c);
                escape = true;
            } else if c == '"' {
                closed = true;
                break;
            } else {
                buf.push(c);
            }
        }
        if !closed {
            return None;
        }
        let after = &rest[consumed..];
        let second = after.strip_prefix(' ').unwrap_or(after);
        Some((format!("\"{buf}\""), second.to_string()))
    } else {
        let (a, b) = rest.split_once(' ')?;
        Some((a.to_string(), b.to_string()))
    }
}

/// Decode a fast-export path field. Quoted form: `"a/b c.txt"` with
/// C-style `\\` and `\"` escapes. Unquoted form is passed verbatim.
fn unquote_path(s: &str) -> String {
    if !s.starts_with('"') || !s.ends_with('"') || s.len() < 2 {
        return s.to_string();
    }
    let inner = &s[1..s.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') | None => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::attribution::{AttributionRewriter, parse_yaml as parse_attr};
    use crate::patterns::files::{FileMatcher, FileMatcherOptions, parse_yaml as parse_files};

    fn attribution() -> AttributionRewriter {
        let yaml = r"
trailers:
  claude:
    - '^Co-Authored-By:\s*Claude\s*<noreply@anthropic\.com>\s*$'
footers:
  claude:
    - '^Generated with Claude Code\s*$'
cleanup:
  collapse_blank_runs: true
  strip_trailing_whitespace: true
";
        AttributionRewriter::new(&parse_attr(yaml).unwrap(), &[]).unwrap()
    }

    fn files_matcher() -> FileMatcher {
        let yaml = r"
purge:
  claude:
    - '.claude/**'
exclude_by_default: []
";
        FileMatcher::new(&parse_files(yaml).unwrap(), &FileMatcherOptions::default()).unwrap()
    }

    #[test]
    fn rewrites_commit_message() {
        // Hand-build the stream with the correct data length to avoid
        // miscount bugs in fixture authoring.
        let payload = b"Add feature\n\nImplements the thing.\n\n\
                        Co-Authored-By: Claude <noreply@anthropic.com>\n";
        let mut stream: Vec<u8> = Vec::new();
        stream.extend_from_slice(b"commit refs/heads/main\n");
        stream.extend_from_slice(b"mark :1\n");
        stream.extend_from_slice(b"author Test <t@example.com> 1700000000 +0000\n");
        stream.extend_from_slice(b"committer Test <t@example.com> 1700000000 +0000\n");
        stream.extend_from_slice(format!("data {}\n", payload.len()).as_bytes());
        stream.extend_from_slice(payload);
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"done\n");

        let mut out: Vec<u8> = Vec::new();
        let attr = attribution();
        let stats =
            run_stream(&stream[..], &mut out, Some(&attr), None, None, None).unwrap();
        assert_eq!(stats.commits_seen, 1);
        assert_eq!(stats.commits_rewritten, 1);
        let out_s = String::from_utf8_lossy(&out);
        assert!(
            !out_s.contains("Claude"),
            "message should be scrubbed: {out_s}"
        );
        assert!(out_s.contains("Add feature"));
    }

    #[test]
    fn drops_file_op_for_purge_path() {
        let stream = b"\
commit refs/heads/main
mark :1
author T <t@example.com> 1700000000 +0000
committer T <t@example.com> 1700000000 +0000
data 5
Init

M 100644 :2 .claude/notes.md
M 100644 :3 src/main.rs

done
";
        let mut out: Vec<u8> = Vec::new();
        let m = files_matcher();
        let stats = run_stream(&stream[..], &mut out, None, Some(&m), None, None).unwrap();
        assert_eq!(stats.file_ops_dropped, 1);
        let out_s = String::from_utf8_lossy(&out);
        assert!(!out_s.contains(".claude/notes.md"));
        assert!(out_s.contains("src/main.rs"));
    }

    #[test]
    fn passes_through_blob_records() {
        // Blobs referenced by a commit M-line are buffered and re-emitted just
        // before the commit. Build a minimal stream with a blob + commit.
        let content = b"hello world\n";
        let mut stream: Vec<u8> = Vec::new();
        stream.extend_from_slice(b"blob\n");
        stream.extend_from_slice(b"mark :1\n");
        writeln!(&mut stream, "data {}", content.len()).unwrap();
        stream.extend_from_slice(content);
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"commit refs/heads/main\n");
        stream.extend_from_slice(b"mark :2\n");
        stream.extend_from_slice(b"author T <t@example.com> 1700000000 +0000\n");
        stream.extend_from_slice(b"committer T <t@example.com> 1700000000 +0000\n");
        stream.extend_from_slice(b"data 4\n");
        stream.extend_from_slice(b"init");
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"M 100644 :1 README.md\n");
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"done\n");

        let mut out: Vec<u8> = Vec::new();
        run_stream(&stream[..], &mut out, None, None, None, None).unwrap();
        assert!(
            out.windows(b"hello world".len())
                .any(|w| w == b"hello world"),
            "blob content must appear in output: {}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn drops_inline_blob_modify_consumes_data() {
        // Inline blob M-lines: the data block follows inline. Purged inline blobs
        // must consume the data block so it doesn't corrupt the stream.
        let stream = b"\
commit refs/heads/main
mark :1
author T <t@example.com> 1700000000 +0000
committer T <t@example.com> 1700000000 +0000
data 5
Init

M 100644 inline .claude/inline.md
data 5
HELLO
M 100644 :2 src/main.rs

done
";
        let mut out: Vec<u8> = Vec::new();
        let m = files_matcher();
        let stats = run_stream(&stream[..], &mut out, None, Some(&m), None, None).unwrap();
        assert_eq!(stats.file_ops_dropped, 1);
        let out_s = String::from_utf8_lossy(&out);
        assert!(!out_s.contains(".claude/inline.md"));
        assert!(!out_s.contains("HELLO"));
        assert!(out_s.contains("src/main.rs"));
    }

    #[test]
    fn unquote_path_handles_escapes() {
        assert_eq!(unquote_path("plain.txt"), "plain.txt");
        assert_eq!(unquote_path(r#""with space.txt""#), "with space.txt");
        assert_eq!(unquote_path(r#""a\"b""#), "a\"b");
        assert_eq!(unquote_path(r#""a\\b""#), "a\\b");
    }

    #[test]
    fn lockfile_rewriter_rewrites_blob_content() {
        use crate::patterns::LockfileRewriter;

        struct StripAxios;
        impl LockfileRewriter for StripAxios {
            fn applies_to(&self, path: &str) -> bool {
                path == "Cargo.lock" || path.ends_with("/Cargo.lock")
            }
            fn strip(&self, content: &[u8]) -> Option<Vec<u8>> {
                let s = std::str::from_utf8(content).ok()?;
                if s.contains("axios") {
                    Some(s.replace("axios", "REMOVED").into_bytes())
                } else {
                    None
                }
            }
        }

        let cargo_lock = b"axios=1.0\ninnocent=2.0\n";
        let mut stream: Vec<u8> = Vec::new();
        stream.extend_from_slice(b"blob\n");
        stream.extend_from_slice(b"mark :1\n");
        writeln!(&mut stream, "data {}", cargo_lock.len()).unwrap();
        stream.extend_from_slice(cargo_lock);
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"commit refs/heads/main\n");
        stream.extend_from_slice(b"mark :2\n");
        stream.extend_from_slice(b"author T <t@example.com> 1700000000 +0000\n");
        stream.extend_from_slice(b"committer T <t@example.com> 1700000000 +0000\n");
        stream.extend_from_slice(b"data 4\n");
        stream.extend_from_slice(b"init");
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"M 100644 :1 Cargo.lock\n");
        stream.extend_from_slice(b"\n");
        stream.extend_from_slice(b"done\n");

        let lfs: Vec<Box<dyn LockfileRewriter>> = vec![Box::new(StripAxios)];
        let mut out: Vec<u8> = Vec::new();
        let stats =
            run_stream(&stream[..], &mut out, None, None, None, Some(&lfs)).unwrap();

        assert_eq!(stats.blobs_rewritten, 1, "one blob must be rewritten");
        let out_s = String::from_utf8_lossy(&out);
        assert!(out_s.contains("REMOVED"), "rewritten content must appear: {out_s}");
        assert!(out_s.contains("innocent"), "innocent content must survive: {out_s}");
        assert!(
            !out_s.contains("axios"),
            "original content must be gone: {out_s}"
        );
    }
}
