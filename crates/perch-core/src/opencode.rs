//! OpenCode, as a [`Provider`].
//!
//! OpenCode keeps everything in one SQLite database,
//! `~/.local/share/opencode/opencode.db` (or under `XDG_DATA_HOME`), in WAL
//! mode and written while it runs. Opening it, even read-only, can create or
//! touch its `-shm` file, which would break Perch's promise never to change
//! another tool's files. And opening it as immutable would miss whatever is
//! still in the WAL, which is the session in progress.
//!
//! So Perch copies the database and its WAL into its own data folder whenever
//! either changes, and reads the copy. OpenCode's folder is only ever read.
//!
//! From the copy it reads `session_v2`, by named columns: each session's
//! folder, model, times and token totals. Never the message, credential or
//! account tables. A session is counted on the day it was last active, since
//! per-response usage would mean reading the message table.

use crate::actions::TerminalCommand;
use crate::db::Db;
use crate::index::IndexStats;
use crate::live::LiveSession;
use crate::model::{SessionRecord, Turn, TurnUsage};
use crate::platform::ProcessProbe;
use crate::provider::{project_for, session_key, Capabilities, Provider};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const OPENCODE: &str = "opencode";

/// OpenCode's data folder: `$XDG_DATA_HOME/opencode`, otherwise
/// `~/.local/share/opencode`.
pub fn opencode_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("opencode"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/opencode"))
}

pub struct OpenCode {
    home: PathBuf,
    /// Perch's own folder for the copy.
    cache: PathBuf,
}

impl OpenCode {
    pub fn new(home: impl Into<PathBuf>, cache: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            cache: cache.into(),
        }
    }

    fn source(&self) -> PathBuf {
        self.home.join("opencode.db")
    }
}

/// `"anthropic/claude-sonnet-5"` or `{"providerID":…,"modelID":…}` to the
/// model's own name, so Perch's prices apply to it.
fn model_name(raw: Option<&str>) -> String {
    let raw = raw.map(str::trim).unwrap_or("");
    if raw.starts_with('{') {
        if let Some(id) = serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| {
                v.get("modelID")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
        {
            return id;
        }
    }
    match raw.rsplit('/').next() {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => "unknown".to_string(),
    }
}

struct SessionTotal {
    id: String,
    directory: String,
    model: Option<String>,
    created: i64,
    updated: i64,
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    version: String,
}

fn read_sessions(copy: &Path) -> Result<Vec<SessionTotal>> {
    let conn = rusqlite::Connection::open(copy)?;
    let mut stmt = conn.prepare(
        "SELECT id, directory, model, time_created, time_updated, tokens_input,
                tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write,
                version
         FROM session_v2",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SessionTotal {
                id: r.get(0)?,
                directory: r.get(1)?,
                model: r.get(2)?,
                created: r.get(3)?,
                updated: r.get(4)?,
                input: r.get(5)?,
                output: r.get(6)?,
                reasoning: r.get(7)?,
                cache_read: r.get(8)?,
                cache_write: r.get(9)?,
                version: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// OpenCode reports reasoning beside output rather than inside it; Perch
/// counts it inside output, as thinking, the way Claude Code reports it.
fn usage_of(s: &SessionTotal) -> TurnUsage {
    let n = |v: i64| v.max(0) as u64;
    TurnUsage {
        input: n(s.input),
        output: n(s.output) + n(s.reasoning),
        cache_read: n(s.cache_read),
        cache_write_5m: n(s.cache_write),
        cache_write_1h: 0,
        thinking: n(s.reasoning),
    }
}

impl Provider for OpenCode {
    fn id(&self) -> &'static str {
        OPENCODE
    }

    fn name(&self) -> &'static str {
        "OpenCode"
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
        vec![self.home.clone()]
    }

    fn index(&self, db: &Db) -> Result<IndexStats> {
        let mut stats = IndexStats::default();
        let Some(copy) = crate::snapshot::refresh(&self.source(), &self.cache)? else {
            return Ok(stats);
        };
        let source = self.source().to_string_lossy().into_owned();
        let mut projects = std::collections::HashSet::new();
        for s in read_sessions(&copy)? {
            let usage = usage_of(&s);
            if usage.total_tokens() == 0 {
                continue;
            }
            let key = session_key(OPENCODE, &s.id);
            let project = project_for(db, &s.directory)?;
            projects.insert(project);
            let record = SessionRecord {
                provider: OPENCODE.into(),
                id: key.clone(),
                project_id: project,
                file_path: source.clone(),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(s.created),
                last_activity_at: Some(s.updated),
                cwd: Some(s.directory.clone()),
                git_branch: None,
                cc_version: Some(s.version.clone()).filter(|v| !v.is_empty()),
                title: None,
                message_count: 1,
            };
            let turn = Turn {
                message_id: Some(format!("{key}:totals")),
                ts: s.updated,
                model: model_name(s.model.as_deref()),
                usage,
            };
            db.apply_scan(&record, &[turn], true)?;
            stats.sessions += 1;
            stats.new_turns += 1;
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
            .strip_prefix(&format!("{OPENCODE}:"))
            .unwrap_or(session_id);
        Some(TerminalCommand {
            program: "opencode".into(),
            args: vec!["--session".into(), raw.into()],
            cwd: cwd.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    /// A WAL-mode database shaped like OpenCode's, with one finished session
    /// checkpointed and one still in the WAL, held open as OpenCode would.
    fn opencode_db(home: &Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(home.join("opencode.db")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE session_v2 (id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
                 directory TEXT NOT NULL, title TEXT, version TEXT NOT NULL,
                 tokens_input INTEGER NOT NULL DEFAULT 0, tokens_output INTEGER NOT NULL DEFAULT 0,
                 tokens_reasoning INTEGER NOT NULL DEFAULT 0, tokens_cache_read INTEGER NOT NULL DEFAULT 0,
                 tokens_cache_write INTEGER NOT NULL DEFAULT 0, model TEXT,
                 time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL);
             CREATE TABLE credential (id TEXT, secret TEXT);
             INSERT INTO credential VALUES ('c', 'secret-token');
             INSERT INTO session_v2 VALUES ('ses_done', 'p', '/w/app', 'secret title', '1.2.0',
                 100, 40, 10, 500, 20, 'anthropic/claude-sonnet-5', 1790000000000, 1790000600000);
             PRAGMA wal_checkpoint(TRUNCATE);
             INSERT INTO session_v2 VALUES ('ses_live', 'p', '/w/app', 't', '1.2.0',
                 7, 3, 0, 0, 0, '{\"providerID\":\"x\",\"modelID\":\"local-llm\"}',
                 1790000700000, 1790000800000);",
        )
        .unwrap();
        conn
    }

    /// Every file in a folder, with its size and modification time.
    fn listing(dir: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
        let mut out: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let m = e.metadata().unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    m.len(),
                    m.modified().unwrap(),
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn reading_never_changes_anything_in_opencodes_folder() {
        let home = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _open = opencode_db(home.path());
        let before = listing(home.path());
        assert!(
            before.iter().any(|(n, _, _)| n == "opencode.db-wal"),
            "the fixture keeps a live WAL, as OpenCode does"
        );

        let oc = OpenCode::new(home.path(), cache.path().join("opencode"));
        let db = open_in_memory().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        oc.index(&db).unwrap();
        oc.index(&db).unwrap();

        assert_eq!(
            listing(home.path()),
            before,
            "no file added, grown or touched"
        );
    }

    #[test]
    fn sessions_still_in_the_wal_are_read_with_their_totals() {
        let home = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _open = opencode_db(home.path());
        let db = open_in_memory().unwrap();
        crate::pricing::seed_default_prices(&db).unwrap();
        let stats = OpenCode::new(home.path(), cache.path()).index(&db).unwrap();
        assert_eq!(stats.sessions, 2, "the session in progress is there too");

        let rows: Vec<(String, String, i64, i64, i64, i64)> = db
            .conn()
            .prepare("SELECT session_id, model, input, output, thinking, cache_read FROM turns ORDER BY ts")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows[0],
            (
                "opencode:ses_done".into(),
                "claude-sonnet-5".into(),
                100,
                50,
                10,
                500
            ),
            "reasoning counts inside output, and the provider prefix comes off the model"
        );
        assert_eq!(rows[1].1, "local-llm");
        assert!(crate::query::model_has_price(&db, "claude-sonnet-5").unwrap());
        assert!(
            !crate::query::model_has_price(&db, "local-llm").unwrap(),
            "shown as no price"
        );

        let titles: Option<String> = db
            .conn()
            .query_row("SELECT group_concat(title) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(titles, None, "no title, no message, no credential is read");
    }

    #[test]
    fn an_unchanged_database_is_not_copied_again_and_a_changed_one_is() {
        let home = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let open = opencode_db(home.path());
        let oc = OpenCode::new(home.path(), cache.path());
        let db = open_in_memory().unwrap();
        oc.index(&db).unwrap();
        let copied = std::fs::metadata(cache.path().join("opencode.db"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        oc.index(&db).unwrap();
        assert_eq!(
            std::fs::metadata(cache.path().join("opencode.db"))
                .unwrap()
                .modified()
                .unwrap(),
            copied
        );

        open.execute(
            "UPDATE session_v2 SET tokens_output = 99, time_updated = 1790000900000 WHERE id = 'ses_live'",
            [],
        )
        .unwrap();
        oc.index(&db).unwrap();
        let output: i64 = db
            .conn()
            .query_row(
                "SELECT output FROM turns WHERE session_id = 'opencode:ses_live'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(output, 99, "a newer total replaces the old one");
        assert_eq!(db.turn_count().unwrap(), 2);
    }

    #[test]
    fn no_database_means_nothing_to_read() {
        let home = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let stats = OpenCode::new(home.path().join("missing"), cache.path())
            .index(&open_in_memory().unwrap())
            .unwrap();
        assert_eq!(stats.sessions, 0);
    }

    #[test]
    fn model_names_lose_their_provider() {
        assert_eq!(model_name(Some("anthropic/claude-opus-5")), "claude-opus-5");
        assert_eq!(model_name(Some("gpt-6-sol")), "gpt-6-sol");
        assert_eq!(
            model_name(Some(r#"{"providerID":"o","modelID":"m1"}"#)),
            "m1"
        );
        assert_eq!(model_name(None), "unknown");
    }
}
