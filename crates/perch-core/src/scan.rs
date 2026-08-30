//! Incremental scanning of append-only transcript files.

use crate::model::{SessionMeta, Turn};
use crate::transcript::parse_line;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ScanOutcome {
    pub turns: Vec<Turn>,
    pub meta: SessionMeta,
    pub new_offset: u64,
    pub lines_parsed: u64,
    pub lines_skipped: u64,
}

/// Read new bytes from `path` starting at `offset`.
///
/// Transcripts are append-only, so `offset` is a valid resume point. The scan
/// stops at the last complete line: a session being written right now will have
/// a truncated final line, and consuming it would corrupt the resume point.
pub fn scan_from(path: &Path, offset: u64) -> std::io::Result<ScanOutcome> {
    let len = std::fs::metadata(path)?.len();

    // File replaced or truncated: the stored offset is meaningless.
    let start = if len < offset { 0 } else { offset };

    if len == start {
        return Ok(ScanOutcome {
            new_offset: start,
            ..Default::default()
        });
    }

    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buf)?;

    // Consume only through the final newline.
    let last_newline = match buf.iter().rposition(|b| *b == b'\n') {
        Some(i) => i,
        None => {
            // No complete line in the new bytes yet.
            return Ok(ScanOutcome {
                new_offset: start,
                ..Default::default()
            });
        }
    };
    let complete = &buf[..=last_newline];
    let new_offset = start + complete.len() as u64;

    let mut out = ScanOutcome {
        new_offset,
        ..Default::default()
    };

    for raw in complete.split(|b| *b == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let text = match std::str::from_utf8(raw) {
            Ok(t) => t,
            Err(_) => {
                out.lines_skipped += 1;
                continue;
            }
        };
        if text.trim().is_empty() {
            continue;
        }

        let Some(parsed) = parse_line(text) else {
            out.lines_skipped += 1;
            continue;
        };
        out.lines_parsed += 1;
        out.meta.message_count += 1;

        if out.meta.cwd.is_none() {
            out.meta.cwd = parsed.cwd.clone();
        }
        if parsed.git_branch.is_some() {
            out.meta.git_branch = parsed.git_branch.clone();
        }
        if parsed.cc_version.is_some() {
            out.meta.cc_version = parsed.cc_version.clone();
        }
        if let Some(ts) = parsed.ts {
            out.meta.first_ts = Some(out.meta.first_ts.map_or(ts, |f| f.min(ts)));
            out.meta.last_ts = Some(out.meta.last_ts.map_or(ts, |l| l.max(ts)));
        }

        if let (Some(usage), Some(ts)) = (parsed.usage, parsed.ts) {
            out.turns.push(Turn {
                ts,
                model: parsed.model.unwrap_or_else(|| "unknown".to_string()),
                usage,
            });
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn assistant(ts: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","cwd":"/Users/a/proj","gitBranch":"main","version":"2.1.1","timestamp":"{ts}","message":{{"model":"claude-fable-5","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        )
    }

    fn write_lines(path: &Path, lines: &[String]) {
        let mut f = File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    fn append_lines(path: &Path, lines: &[String]) {
        let mut f = File::options().append(true).open(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    #[test]
    fn scans_a_whole_file_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(
            &p,
            &[
                assistant("2026-08-18T10:00:00.000Z", 1, 2),
                assistant("2026-08-18T10:01:00.000Z", 3, 4),
            ],
        );

        let out = scan_from(&p, 0).unwrap();
        assert_eq!(out.turns.len(), 2);
        assert_eq!(out.turns[0].usage.input, 1);
        assert_eq!(out.turns[1].usage.output, 4);
        assert_eq!(out.new_offset, std::fs::metadata(&p).unwrap().len());
        assert_eq!(out.meta.cwd.as_deref(), Some("/Users/a/proj"));
        assert_eq!(out.meta.message_count, 2);
    }

    /// THE critical property: incremental scanning equals a full rescan.
    #[test]
    fn incremental_scan_equals_full_scan() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(
            &p,
            &[
                assistant("2026-08-18T10:00:00.000Z", 1, 2),
                assistant("2026-08-18T10:01:00.000Z", 3, 4),
            ],
        );

        let first = scan_from(&p, 0).unwrap();

        append_lines(
            &p,
            &[
                assistant("2026-08-18T10:02:00.000Z", 5, 6),
                assistant("2026-08-18T10:03:00.000Z", 7, 8),
            ],
        );

        let second = scan_from(&p, first.new_offset).unwrap();
        assert_eq!(second.turns.len(), 2, "must only see the appended lines");

        let mut incremental: Vec<Turn> = first.turns.clone();
        incremental.extend(second.turns.clone());

        let full = scan_from(&p, 0).unwrap();
        assert_eq!(incremental, full.turns);
        assert_eq!(second.new_offset, full.new_offset);
    }

    #[test]
    fn does_not_consume_a_partial_final_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(&p, &[assistant("2026-08-18T10:00:00.000Z", 1, 2)]);
        let complete_len = std::fs::metadata(&p).unwrap().len();

        // Simulate a line mid-write: no trailing newline.
        let mut f = File::options().append(true).open(&p).unwrap();
        write!(f, r#"{{"type":"assistant","timestamp":"2026-08-18T10:0"#).unwrap();
        drop(f);

        let out = scan_from(&p, 0).unwrap();
        assert_eq!(out.turns.len(), 1, "partial line must be ignored");
        assert_eq!(
            out.new_offset, complete_len,
            "offset must stop at the last newline"
        );
    }

    #[test]
    fn resumes_correctly_after_a_partial_line_completes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(&p, &[assistant("2026-08-18T10:00:00.000Z", 1, 2)]);

        let mut f = File::options().append(true).open(&p).unwrap();
        write!(f, "{}", assistant("2026-08-18T10:01:00.000Z", 3, 4)).unwrap();
        drop(f);

        let first = scan_from(&p, 0).unwrap();
        assert_eq!(first.turns.len(), 1);

        // The writer finishes the line.
        let mut f = File::options().append(true).open(&p).unwrap();
        writeln!(f).unwrap();
        drop(f);

        let second = scan_from(&p, first.new_offset).unwrap();
        assert_eq!(second.turns.len(), 1);
        assert_eq!(second.turns[0].usage.input, 3);
    }

    #[test]
    fn skips_malformed_lines_and_counts_them() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(
            &p,
            &[
                assistant("2026-08-18T10:00:00.000Z", 1, 2),
                "{ not json".to_string(),
                assistant("2026-08-18T10:02:00.000Z", 5, 6),
            ],
        );

        let out = scan_from(&p, 0).unwrap();
        assert_eq!(out.turns.len(), 2);
        assert_eq!(out.lines_skipped, 1);
        assert_eq!(out.lines_parsed, 2);
    }

    #[test]
    fn rescans_from_zero_when_file_shrank() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(
            &p,
            &[
                assistant("2026-08-18T10:00:00.000Z", 1, 2),
                assistant("2026-08-18T10:01:00.000Z", 3, 4),
            ],
        );
        let big_offset = std::fs::metadata(&p).unwrap().len();

        write_lines(&p, &[assistant("2026-08-18T11:00:00.000Z", 9, 9)]);

        let out = scan_from(&p, big_offset).unwrap();
        assert_eq!(out.turns.len(), 1, "shrunk file must be rescanned from 0");
        assert_eq!(out.turns[0].usage.input, 9);
    }

    #[test]
    fn returns_empty_when_no_new_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(&p, &[assistant("2026-08-18T10:00:00.000Z", 1, 2)]);
        let first = scan_from(&p, 0).unwrap();

        let second = scan_from(&p, first.new_offset).unwrap();
        assert!(second.turns.is_empty());
        assert_eq!(second.new_offset, first.new_offset);
    }

    #[test]
    fn tracks_first_and_last_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(
            &p,
            &[
                assistant("2026-08-18T10:00:00.000Z", 1, 2),
                assistant("2026-08-18T12:00:00.000Z", 3, 4),
            ],
        );
        let out = scan_from(&p, 0).unwrap();
        assert!(out.meta.first_ts.unwrap() < out.meta.last_ts.unwrap());
    }
}
