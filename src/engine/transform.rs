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
//! - `blob` records: passthrough, including the `data N` block.
//! - `commit` records: rewrite the `data N` message block; filter `M D R C`
//!   file ops by path.
//! - `tag` records: rewrite the `data N` annotation message.
//! - `reset`, `feature`, `option`, `done`, `progress`, `checkpoint`,
//!   `ls`, `cat-blob`, `get-mark`: passthrough verbatim.
//!
//! Anything we don't recognise passes through verbatim. We never decode
//! UTF-8; the stream is bytes end-to-end. Commit messages MAY contain
//! non-UTF-8 sequences.

use std::io::{BufRead, BufReader, Read, Write};

use super::{EngineError, EngineStats};
use crate::patterns::{AttributionRewriter, BlobRewriter, FileMatcher};

/// Drive the transform stage from the given reader to the given writer.
///
/// Returns the per-pass statistics for plan / runbook output.
pub fn run_stream<R: Read, W: Write>(
    reader: R,
    mut writer: W,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
) -> Result<EngineStats, EngineError> {
    let mut br = BufReader::new(reader);
    let mut stats = EngineStats::default();

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
                    &mut stats,
                )?;
            }
            RecordKind::Tag => {
                process_tag(&line, &mut br, &mut writer, attribution)?;
            }
            RecordKind::Blob => {
                stats.blobs_seen += 1;
                process_blob(&line, &mut br, &mut writer, blob, &mut stats)?;
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

/// Process a `commit` record:
///   commit <ref>
///   mark :N           (optional)
///   original-oid ...  (optional)
///   author ...        (optional, repeats)
///   committer ...
///   encoding ...      (optional)
///   data N
///   <N bytes>
///   from :N           (optional)
///   merge :N          (optional, repeats)
///   <file ops>
///   <blank line>
fn process_commit<R: BufRead, W: Write>(
    header: &[u8],
    br: &mut R,
    writer: &mut W,
    attribution: Option<&AttributionRewriter>,
    files: Option<&FileMatcher>,
    blob: Option<&BlobRewriter>,
    stats: &mut EngineStats,
) -> Result<(), EngineError> {
    writer.write_all(header).map_err(EngineError::Io)?;

    // Pass through pre-data header lines until we hit `data N` (or EOF).
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
            rewrite_data_block(
                &line,
                br,
                writer,
                attribution,
                stats,
                /*is_commit=*/ true,
            )?;
            break;
        }
        writer.write_all(&line).map_err(EngineError::Io)?;
    }

    // After the data block, file ops and post-message lines until a blank
    // line terminates the record OR another top-level record begins.
    loop {
        let mut line: Vec<u8> = Vec::with_capacity(128);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            return Ok(());
        }
        if line == b"\n" {
            writer.write_all(&line).map_err(EngineError::Io)?;
            return Ok(());
        }

        if is_file_op_line(&line) {
            handle_file_op(&line, br, writer, files, stats)?;
            continue;
        }

        if matches!(
            record_kind(&line),
            RecordKind::Commit | RecordKind::Tag | RecordKind::Blob
        ) {
            // Next top-level record without a blank line in between: re-dispatch.
            match record_kind(&line) {
                RecordKind::Commit => {
                    stats.commits_seen += 1;
                    process_commit(&line, br, writer, attribution, files, blob, stats)?;
                }
                RecordKind::Tag => process_tag(&line, br, writer, attribution)?,
                RecordKind::Blob => {
                    stats.blobs_seen += 1;
                    process_blob(&line, br, writer, blob, stats)?;
                }
                RecordKind::Other => writer.write_all(&line).map_err(EngineError::Io)?,
            }
            return Ok(());
        }

        writer.write_all(&line).map_err(EngineError::Io)?;
    }
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

fn process_blob<R: BufRead, W: Write>(
    header: &[u8],
    br: &mut R,
    writer: &mut W,
    blob: Option<&BlobRewriter>,
    stats: &mut EngineStats,
) -> Result<(), EngineError> {
    writer.write_all(header).map_err(EngineError::Io)?;
    loop {
        let mut line: Vec<u8> = Vec::with_capacity(64);
        let n = br.read_until(b'\n', &mut line).map_err(EngineError::Io)?;
        if n == 0 {
            return Ok(());
        }
        if line.starts_with(b"data ") {
            rewrite_blob_data_block(&line, br, writer, blob, stats)?;
            return Ok(());
        }
        writer.write_all(&line).map_err(EngineError::Io)?;
    }
}

/// Read a blob `data N\n<N bytes>` block, optionally apply blob
/// substitutions, and write the (possibly-resized) block back.
fn rewrite_blob_data_block<R: BufRead, W: Write>(
    data_line: &[u8],
    br: &mut R,
    writer: &mut W,
    blob: Option<&BlobRewriter>,
    stats: &mut EngineStats,
) -> Result<(), EngineError> {
    let len = parse_data_len(data_line)?;
    let mut buf = vec![0u8; len];
    br.read_exact(&mut buf).map_err(EngineError::Io)?;
    consume_optional_lf(br)?;

    let (out_bytes, changed): (Vec<u8>, bool) = match blob {
        Some(r) if !r.is_empty() => {
            let (cow, changed) = r.apply(&buf);
            (cow.into_owned(), changed)
        }
        _ => (buf, false),
    };
    if changed {
        stats.blobs_rewritten += 1;
    }

    writeln!(writer, "data {}", out_bytes.len()).map_err(EngineError::Io)?;
    writer.write_all(&out_bytes).map_err(EngineError::Io)?;
    writer.write_all(b"\n").map_err(EngineError::Io)?;
    Ok(())
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

fn passthrough_data_block<R: BufRead, W: Write>(
    data_line: &[u8],
    br: &mut R,
    writer: &mut W,
) -> Result<(), EngineError> {
    let len = parse_data_len(data_line)?;
    writer.write_all(data_line).map_err(EngineError::Io)?;
    let mut buf = vec![0u8; len];
    br.read_exact(&mut buf).map_err(EngineError::Io)?;
    writer.write_all(&buf).map_err(EngineError::Io)?;
    consume_optional_lf(br)?;
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

fn handle_file_op<R: BufRead, W: Write>(
    line: &[u8],
    br: &mut R,
    writer: &mut W,
    files: Option<&FileMatcher>,
    stats: &mut EngineStats,
) -> Result<(), EngineError> {
    // deleteall and filedeleteall are passthrough — they wipe state, not
    // a single path.
    if line.starts_with(b"deleteall\n") || line.starts_with(b"filedeleteall\n") {
        writer.write_all(line).map_err(EngineError::Io)?;
        return Ok(());
    }

    let op = line.first().copied().unwrap_or(0);

    // Inline blobs: `M <mode> inline <path>\ndata N\n<N bytes>\n`
    let has_inline = op == b'M' && line.windows(8).any(|w| w == b" inline ");

    let path = extract_path(line, op);
    let purge = files.is_some_and(|m| path.as_ref().is_some_and(|p| m.should_purge(p)));

    if purge {
        stats.file_ops_dropped += 1;
        // For inline-blob modifies, we also need to consume the
        // `data N\n<N bytes>\n` follow-up so it doesn't reach fast-import.
        if has_inline {
            let mut data_line: Vec<u8> = Vec::with_capacity(32);
            br.read_until(b'\n', &mut data_line)
                .map_err(EngineError::Io)?;
            if data_line.starts_with(b"data ") {
                let len = parse_data_len(&data_line)?;
                let mut sink = vec![0u8; len];
                br.read_exact(&mut sink).map_err(EngineError::Io)?;
            }
        }
        return Ok(());
    }

    writer.write_all(line).map_err(EngineError::Io)?;
    if has_inline {
        let mut data_line: Vec<u8> = Vec::with_capacity(32);
        br.read_until(b'\n', &mut data_line)
            .map_err(EngineError::Io)?;
        passthrough_data_block(&data_line, br, writer)?;
    }
    Ok(())
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
        let stats = run_stream(&stream[..], &mut out, Some(&attr), None, None).unwrap();
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
        let stats = run_stream(&stream[..], &mut out, None, Some(&m), None).unwrap();
        assert_eq!(stats.file_ops_dropped, 1);
        let out_s = String::from_utf8_lossy(&out);
        assert!(!out_s.contains(".claude/notes.md"));
        assert!(out_s.contains("src/main.rs"));
    }

    #[test]
    fn passes_through_blob_records() {
        let stream = b"\
blob
mark :1
data 11
hello world

done
";
        let mut out: Vec<u8> = Vec::new();
        run_stream(&stream[..], &mut out, None, None, None).unwrap();
        assert!(
            out.windows(b"hello world".len())
                .any(|w| w == b"hello world")
        );
    }

    #[test]
    fn drops_inline_blob_modify_consumes_data() {
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
        let stats = run_stream(&stream[..], &mut out, None, Some(&m), None).unwrap();
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
}
