//! OpenAI's Codex CLI, as a [`Provider`].
//!
//! Codex writes one "rollout" per session under
//! `sessions/YYYY/MM/DD/rollout-<time>-<uuid>.jsonl` in its home (`~/.codex`,
//! or `CODEX_HOME`), and moves archived ones to `archived_sessions/`. Each
//! response's usage is a `token_usage_record` line with the response's id.
//! Versions before those records report a running total in `token_count`
//! events instead, and a response is the step between two totals.
//!
//! Codex later compresses older rollouts with zstd, which Perch can't read
//! without a new dependency. For those it takes the session's place, time,
//! model and total tokens from Codex's own `state_N.sqlite`, opened read-only
//! and immutable, selecting named columns only: the table also holds the
//! session's opening message, which Perch never reads. Those sessions are
//! listed under the model name plus ", totals only", which has no price, so
//! their cost shows as unknown rather than wrong.
//!
//! Paths read: the two session folders above, three levels deep at most, and
//! `state_N.sqlite`. Nothing else in Codex's home, which also holds its login.

use crate::actions::TerminalCommand;
use crate::db::Db;
use crate::index::IndexStats;
use crate::live::LiveSession;
use crate::model::{SessionRecord, Turn, TurnUsage};
use crate::platform::ProcessProbe;
use crate::provider::{session_key, Capabilities, Provider};
use anyhow::Result;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub const CODEX: &str = "codex";

/// Codex's home: `CODEX_HOME` when set, otherwise `~/.codex`.
pub fn codex_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex"))
}

pub struct Codex {
    home: PathBuf,
}

impl Codex {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    /// Plain rollouts, oldest folders first.
    fn rollouts(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let sessions = self.home.join("sessions");
        for year in numbered_dirs(&sessions) {
            for month in numbered_dirs(&year) {
                for day in numbered_dirs(&month) {
                    out.extend(rollout_files(&day));
                }
            }
        }
        out.extend(rollout_files(&self.home.join("archived_sessions")));
        out
    }

    /// The newest `state_N.sqlite`, if Codex has one.
    fn state_db(&self) -> Option<PathBuf> {
        std::fs::read_dir(&self.home)
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let n: u32 = name
                    .strip_prefix("state_")?
                    .strip_suffix(".sqlite")?
                    .parse()
                    .ok()?;
                Some((n, e.path()))
            })
            .max_by_key(|(n, _)| *n)
            .map(|(_, p)| p)
    }
}

fn numbered_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                && e.file_type().is_ok_and(|t| t.is_dir())
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

fn rollout_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// The session's id: the uuid that ends a rollout's file name.
fn session_uuid(path: &Path) -> Option<String> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    (id.len() == 36 && id.bytes().filter(|&b| b == b'-').count() == 4).then(|| id.to_string())
}

/// Folder to project slug, the way Claude Code names its project folders, so
/// a folder both tools work in is one project.
fn slug_for(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn ts_ms(v: &Value) -> Option<i64> {
    let s = v.get("timestamp")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

fn count(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Codex counts cached and cache-written tokens inside `input_tokens`, and
/// reasoning inside `output_tokens`. Perch keeps them apart, as Claude Code
/// reports them, so nothing is counted twice.
fn usage_of(u: &Value) -> TurnUsage {
    let input = count(u, "input_tokens");
    let cached = count(u, "cached_input_tokens");
    let written = count(u, "cache_write_input_tokens");
    TurnUsage {
        input: input.saturating_sub(cached).saturating_sub(written),
        output: count(u, "output_tokens"),
        cache_read: cached,
        cache_write_5m: written,
        cache_write_1h: 0,
        thinking: count(u, "reasoning_output_tokens"),
    }
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Rollout {
    pub cwd: Option<String>,
    pub started_at: Option<i64>,
    pub last_at: Option<i64>,
    pub cli_version: Option<String>,
    pub turns: Vec<Turn>,
    pub lines_skipped: u64,
}

/// Reads a rollout for usage only. Message lines are parsed and dropped:
/// nothing but counts, models, times and the working folder is kept.
pub(crate) fn parse_rollout(reader: impl BufRead, key: &str) -> Rollout {
    let mut out = Rollout::default();
    let mut model = String::from("unknown");
    let mut records: Vec<Turn> = Vec::new();
    let mut steps: Vec<Turn> = Vec::new();
    let mut last_total: Option<u64> = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            out.lines_skipped += 1;
            continue;
        };
        let ts = ts_ms(&v);
        let payload = v.get("payload").unwrap_or(&Value::Null);
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                out.cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.cli_version = payload
                    .get("cli_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.started_at = ts_ms(payload).or(ts);
            }
            Some("turn_context") => {
                if let Some(m) = payload.get("model").and_then(Value::as_str) {
                    model = m.to_string();
                }
                if out.cwd.is_none() {
                    out.cwd = payload
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
            }
            Some("token_usage_record") => {
                let (Some(id), Some(u), Some(ts)) = (
                    payload.get("response_id").and_then(Value::as_str),
                    payload.get("usage"),
                    ts,
                ) else {
                    continue;
                };
                records.push(Turn {
                    message_id: Some(format!("{key}:{id}")),
                    ts,
                    model: model.clone(),
                    usage: usage_of(u),
                });
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("token_count") =>
            {
                // `info` is null on events that only carry rate limits.
                let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                    continue;
                };
                let total = info
                    .get("total_token_usage")
                    .map(|t| count(t, "total_tokens"));
                let (Some(total), Some(last), Some(ts)) = (total, info.get("last_token_usage"), ts)
                else {
                    continue;
                };
                // The same total again is the same response reported twice.
                if last_total == Some(total) {
                    continue;
                }
                last_total = Some(total);
                steps.push(Turn {
                    message_id: Some(format!("{key}:total:{total}")),
                    ts,
                    model: model.clone(),
                    usage: usage_of(last),
                });
            }
            _ => {}
        }
        if let Some(ts) = ts {
            out.last_at = Some(out.last_at.map_or(ts, |l| l.max(ts)));
        }
    }
    // Per-response records when the version writes them, running totals when
    // it doesn't. Never both: they describe the same responses.
    out.turns = if records.is_empty() { steps } else { records };
    out
}

/// One row of Codex's thread table, minus everything Perch doesn't read.
struct ThreadTotal {
    id: String,
    rollout_path: String,
    created_ms: i64,
    updated_ms: i64,
    cwd: String,
    model: Option<String>,
    tokens: u64,
    branch: Option<String>,
    cli_version: Option<String>,
}

fn read_thread_totals(db: &Path) -> Result<Vec<ThreadTotal>> {
    // Immutable: no lock, no journal, no write of any kind to Codex's file.
    let mut uri = String::from("file:");
    for c in db.to_string_lossy().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            ' ' => uri.push_str("%20"),
            c => uri.push(c),
        }
    }
    uri.push_str("?immutable=1");
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut stmt = conn.prepare(
        "SELECT id, rollout_path, created_at, updated_at, cwd, model, tokens_used,
                git_branch, cli_version
         FROM threads WHERE tokens_used > 0",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ThreadTotal {
                id: r.get(0)?,
                rollout_path: r.get(1)?,
                created_ms: r.get::<_, i64>(2)? * 1000,
                updated_ms: r.get::<_, i64>(3)? * 1000,
                cwd: r.get(4)?,
                model: r.get(5)?,
                tokens: r.get::<_, i64>(6)?.max(0) as u64,
                branch: r.get(7)?,
                cli_version: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn project_for(db: &Db, cwd: &str) -> Result<i64> {
    match db.project_at(cwd)? {
        Some(id) => Ok(id),
        None => db.upsert_project(&slug_for(cwd), cwd, false),
    }
}

impl Provider for Codex {
    fn id(&self) -> &'static str {
        CODEX
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            live_sessions: false,
            waiting: false,
            tokens: true,
            resume: true,
        }
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![
            self.home.join("sessions"),
            self.home.join("archived_sessions"),
        ]
    }

    fn index(&self, db: &Db) -> Result<IndexStats> {
        let mut stats = IndexStats::default();
        let mut seen = std::collections::HashSet::new();
        let mut projects = std::collections::HashSet::new();

        for path in self.rollouts() {
            let Some(uuid) = session_uuid(&path) else {
                stats.sessions_skipped += 1;
                continue;
            };
            let key = session_key(CODEX, &uuid);
            seen.insert(uuid);
            let Ok(len) = std::fs::metadata(&path).map(|m| m.len()) else {
                stats.sessions_skipped += 1;
                continue;
            };
            stats.sessions += 1;
            // Read whole each time it grows: it carries the model forward
            // from line to line, and active rollouts are few.
            if db.session_offset(&key)? == len {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else {
                stats.sessions_skipped += 1;
                continue;
            };
            let rollout = parse_rollout(BufReader::new(file), &key);
            let Some(cwd) = rollout.cwd.clone() else {
                stats.sessions_skipped += 1;
                continue;
            };
            let project = project_for(db, &cwd)?;
            projects.insert(project);
            stats.bytes_read += len;
            stats.lines_skipped += rollout.lines_skipped;
            stats.new_turns += rollout.turns.len();
            let record = SessionRecord {
                provider: CODEX.into(),
                id: key,
                project_id: project,
                file_path: path.to_string_lossy().into_owned(),
                file_size: len,
                indexed_offset: len,
                started_at: rollout.started_at,
                last_activity_at: rollout.last_at,
                cwd: Some(cwd),
                git_branch: None,
                cc_version: rollout.cli_version,
                title: None,
                message_count: rollout.turns.len() as u64,
            };
            db.apply_scan(&record, &rollout.turns, true)?;
        }

        // Sessions whose rollout is compressed: totals from Codex's database.
        if let Some(state) = self.state_db() {
            for t in read_thread_totals(&state)? {
                if seen.contains(&t.id) {
                    continue;
                }
                let key = session_key(CODEX, &t.id);
                // Read in detail before Codex compressed it: keep that.
                if db
                    .session_file_path(&key)?
                    .is_some_and(|p| p.ends_with(".jsonl"))
                {
                    continue;
                }
                let project = project_for(db, &t.cwd)?;
                projects.insert(project);
                let model = match t.model.as_deref().filter(|m| !m.is_empty()) {
                    Some(m) => format!("{m}, totals only"),
                    None => "Codex, totals only".to_string(),
                };
                let compressed = format!("{}.zst", t.rollout_path);
                let record = SessionRecord {
                    provider: CODEX.into(),
                    id: key.clone(),
                    project_id: project,
                    file_path: if Path::new(&compressed).exists() {
                        compressed
                    } else {
                        t.rollout_path.clone()
                    },
                    file_size: 0,
                    indexed_offset: 0,
                    started_at: Some(t.created_ms),
                    last_activity_at: Some(t.updated_ms),
                    cwd: Some(t.cwd.clone()),
                    git_branch: t.branch.filter(|b| !b.trim().is_empty()),
                    cc_version: t.cli_version.filter(|v| !v.is_empty()),
                    title: None,
                    message_count: 1,
                };
                // All of it as input: which part was output isn't known, and
                // the model name above has no price, so no cost is claimed.
                let turn = Turn {
                    message_id: Some(format!("{key}:totals")),
                    ts: t.updated_ms,
                    model,
                    usage: TurnUsage {
                        input: t.tokens,
                        ..TurnUsage::default()
                    },
                };
                db.apply_scan(&record, &[turn], true)?;
                stats.sessions += 1;
                stats.sessions_totals_only += 1;
            }
        }
        stats.projects = projects.len();
        Ok(stats)
    }

    fn live_sessions(&self, _probe: &dyn ProcessProbe) -> Vec<LiveSession> {
        Vec::new()
    }

    fn resume_command(
        &self,
        session_id: &str,
        cwd: &str,
        _bypass_permissions: bool,
    ) -> Option<TerminalCommand> {
        let raw = session_id
            .strip_prefix(&format!("{CODEX}:"))
            .unwrap_or(session_id);
        Some(TerminalCommand {
            program: "codex".into(),
            args: vec!["resume".into(), raw.into()],
            cwd: cwd.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    const KEY: &str = "codex:019f5ce2-b137-70e2-891e-eb2412158ea1";

    fn usage(input: u64, cached: u64, written: u64, output: u64, reasoning: u64) -> String {
        format!(
            r#"{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":{written},"output_tokens":{output},"reasoning_output_tokens":{reasoning},"total_tokens":{}}}"#,
            input + output
        )
    }

    fn meta(cwd: &str, version: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-11-05T10:00:00.000Z","type":"session_meta","payload":{{"id":"019f5ce2-b137-70e2-891e-eb2412158ea1","cwd":"{cwd}","cli_version":"{version}","timestamp":"2026-11-05T10:00:00.000Z","base_instructions":{{"text":"never kept"}}}}}}"#
        )
    }

    fn context(model: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-11-05T10:00:01.000Z","type":"turn_context","payload":{{"model":"{model}","cwd":"/w/p"}}}}"#
        )
    }

    fn message() -> String {
        r#"{"timestamp":"2026-11-05T10:00:02.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"secret prompt"}]}}"#.into()
    }

    fn token_count(total: u64, last: &str, at: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"total_tokens":{total}}},"last_token_usage":{last}}},"rate_limits":null}}}}"#
        )
    }

    fn record(id: &str, u: &str, at: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"token_usage_record","payload":{{"response_id":"{id}","usage":{u}}}}}"#
        )
    }

    /// Codex 0.149: running totals only, a repeated event, and one that only
    /// carries rate limits.
    #[test]
    fn an_older_rollout_counts_each_step_in_the_running_total_once() {
        let text = [
            meta("/w/p", "0.149.0"),
            context("gpt-6-luna"),
            message(),
            token_count(150, &usage(100, 60, 0, 50, 10), "2026-11-05T10:00:03.000Z"),
            token_count(150, &usage(100, 60, 0, 50, 10), "2026-11-05T10:00:04.000Z"),
            r#"{"timestamp":"2026-11-05T10:00:05.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{}}}"#.into(),
            token_count(400, &usage(200, 150, 20, 50, 0), "2026-11-05T10:00:06.000Z"),
            "not json".into(),
        ]
        .join("\n");
        let r = parse_rollout(text.as_bytes(), KEY);
        assert_eq!(
            r.turns.len(),
            2,
            "the repeat and the rate-limit event add nothing"
        );
        assert_eq!(
            r.turns[0].usage,
            TurnUsage {
                input: 40,
                output: 50,
                cache_read: 60,
                cache_write_5m: 0,
                cache_write_1h: 0,
                thinking: 10,
            },
            "cached tokens are taken out of input, not counted twice"
        );
        assert_eq!(r.turns[1].usage.input, 30, "200 - 150 cached - 20 written");
        assert_eq!(r.turns[1].usage.cache_write_5m, 20);
        assert!(r.turns.iter().all(|t| t.model == "gpt-6-luna"));
        assert_eq!(r.cwd.as_deref(), Some("/w/p"));
        assert_eq!(r.cli_version.as_deref(), Some("0.149.0"));
        assert_eq!(r.lines_skipped, 1);
        let total: u64 = r.turns.iter().map(|t| t.usage.total_tokens()).sum();
        assert_eq!(total, 400, "the steps add up to Codex's own final total");
    }

    /// Codex 0.167: a record per response, which is what's counted, with the
    /// running total alongside it ignored.
    #[test]
    fn a_newer_rollout_counts_each_response_record_once() {
        let text = [
            meta("/w/p", "0.167.0"),
            context("gpt-6-sol"),
            token_count(150, &usage(100, 0, 0, 50, 0), "2026-11-05T10:00:03.000Z"),
            record(
                "resp_a",
                &usage(100, 0, 0, 50, 0),
                "2026-11-05T10:00:03.000Z",
            ),
            context("gpt-6.1-sol"),
            record(
                "resp_b",
                &usage(300, 200, 0, 25, 5),
                "2026-11-05T10:00:09.000Z",
            ),
        ]
        .join("\n");
        let r = parse_rollout(text.as_bytes(), KEY);
        assert_eq!(r.turns.len(), 2);
        assert_eq!(r.turns[0].model, "gpt-6-sol");
        assert_eq!(
            r.turns[1].model, "gpt-6.1-sol",
            "the model in force at the time"
        );
        assert_eq!(
            r.turns[1].message_id.as_deref(),
            Some("codex:019f5ce2-b137-70e2-891e-eb2412158ea1:resp_b")
        );
        let last = chrono::DateTime::parse_from_rfc3339("2026-11-05T10:00:09.000Z").unwrap();
        assert_eq!(r.last_at, Some(last.timestamp_millis()));
    }

    #[test]
    fn nothing_but_counts_and_places_is_kept() {
        let text = [meta("/w/p", "0.167.0"), message()].join("\n");
        let r = parse_rollout(text.as_bytes(), KEY);
        assert!(!format!("{r:?}").contains("secret"), "{r:?}");
        assert!(!format!("{r:?}").contains("never kept"), "{r:?}");
    }

    #[test]
    fn only_rollouts_named_by_date_and_archive_are_read() {
        let home = tempfile::tempdir().unwrap();
        let day = home.path().join("sessions/2026/11/05");
        std::fs::create_dir_all(&day).unwrap();
        let name = "rollout-2026-11-05T10-00-00-019f5ce2-b137-70e2-891e-eb2412158ea1.jsonl";
        std::fs::write(
            day.join(name),
            [
                meta("/w/p", "0.167.0"),
                context("gpt-6-sol"),
                record(
                    "resp_a",
                    &usage(100, 0, 0, 50, 0),
                    "2026-11-05T10:00:03.000Z",
                ),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();
        std::fs::write(day.join(format!("{name}.zst")), b"compressed").unwrap();
        std::fs::write(home.path().join("auth.json"), b"{}").unwrap();
        std::fs::create_dir_all(home.path().join("sessions/notes")).unwrap();
        std::fs::write(home.path().join("sessions/notes/rollout-x.jsonl"), b"").unwrap();

        let codex = Codex::new(home.path());
        assert_eq!(codex.rollouts(), vec![day.join(name)]);

        let db = open_in_memory().unwrap();
        let stats = codex.index(&db).unwrap();
        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.new_turns, 1);
        assert_eq!(db.turn_count().unwrap(), 1);
        let again = codex.index(&db).unwrap();
        assert_eq!(again.new_turns, 0, "an unchanged rollout isn't read again");
        assert_eq!(db.turn_count().unwrap(), 1);
    }

    #[test]
    fn a_compressed_session_is_kept_as_totals_from_codex_state() {
        let home = tempfile::tempdir().unwrap();
        let rollout = home
            .path()
            .join("sessions/2026/07/13/rollout-2026-07-13T21-09-48-019f5ce2-b137-70e2-891e-eb2412158ea1.jsonl");
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        std::fs::write(format!("{}.zst", rollout.display()), b"compressed").unwrap();
        {
            let conn = rusqlite::Connection::open(home.path().join("state_5.sqlite")).unwrap();
            conn.execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
                     created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                     cwd TEXT NOT NULL, title TEXT NOT NULL, model TEXT,
                     tokens_used INTEGER NOT NULL DEFAULT 0, git_branch TEXT,
                     cli_version TEXT NOT NULL DEFAULT '',
                     first_user_message TEXT NOT NULL DEFAULT '');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO threads VALUES (?1, ?2, 1783000000, 1783000600, '/w/p',
                     'secret title', 'gpt-6-luna', 5000, 'main', '0.162.0', 'secret prompt')",
                rusqlite::params![
                    "019f5ce2-b137-70e2-891e-eb2412158ea1",
                    rollout.to_string_lossy()
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO threads VALUES ('empty', '/x', 1, 1, '/w/q', '', NULL, 0, NULL, '', '')",
                [],
            )
            .unwrap();
        }

        let db = open_in_memory().unwrap();
        crate::pricing::seed_default_prices(&db).unwrap();
        let stats = Codex::new(home.path()).index(&db).unwrap();
        assert_eq!(
            stats.sessions_totals_only, 1,
            "a thread with no tokens is left out"
        );
        let (model, input, ts): (String, i64, i64) = db
            .conn()
            .query_row("SELECT model, input, ts FROM turns", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(model, "gpt-6-luna, totals only");
        assert_eq!(input, 5000);
        assert_eq!(ts, 1_783_000_600_000);
        assert!(
            !crate::query::model_has_price(&db, &model).unwrap(),
            "totals can't be priced, so no cost is claimed"
        );
        let dump: String = db
            .conn()
            .query_row(
                "SELECT group_concat(COALESCE(title,'') || COALESCE(git_branch,'')) FROM sessions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!dump.contains("secret"), "{dump}");

        Codex::new(home.path()).index(&db).unwrap();
        assert_eq!(
            db.turn_count().unwrap(),
            1,
            "a second pass replaces, not adds"
        );
    }

    #[test]
    fn a_shared_folder_is_one_project_with_claude_code() {
        let db = open_in_memory().unwrap();
        let claude = db
            .upsert_project("-Users-a-my-app", "/Users/a/my.app", false)
            .unwrap();
        assert_eq!(project_for(&db, "/Users/a/my.app").unwrap(), claude);
        assert_eq!(slug_for("/Users/a/my.app"), "-Users-a-my-app");
    }

    #[test]
    fn resume_uses_codex_own_id() {
        let c = Codex::new("/nowhere")
            .resume_command(KEY, "/w/p", false)
            .unwrap();
        assert_eq!(c.program, "codex");
        assert_eq!(c.args, ["resume", "019f5ce2-b137-70e2-891e-eb2412158ea1"]);
    }
}
