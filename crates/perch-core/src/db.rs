//! SQLite persistence. User-owned columns survive re-indexing.

use crate::model::{SessionRecord, Turn};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 1;

pub struct Db {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id                INTEGER PRIMARY KEY,
    slug              TEXT NOT NULL UNIQUE,
    real_path         TEXT NOT NULL,
    path_is_guess     INTEGER NOT NULL DEFAULT 0,
    parent_project_id INTEGER REFERENCES projects(id),
    -- user-owned below this line; never overwritten by indexing
    display_name      TEXT,
    status            TEXT NOT NULL DEFAULT 'active',
    pinned            INTEGER NOT NULL DEFAULT 0,
    note              TEXT,
    note_updated_at   INTEGER,
    archived          INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    project_id       INTEGER NOT NULL REFERENCES projects(id),
    file_path        TEXT NOT NULL,
    file_size        INTEGER NOT NULL DEFAULT 0,
    indexed_offset   INTEGER NOT NULL DEFAULT 0,
    started_at       INTEGER,
    last_activity_at INTEGER,
    cwd              TEXT,
    git_branch       TEXT,
    cc_version       TEXT,
    message_count    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);

CREATE TABLE IF NOT EXISTS turns (
    session_id     TEXT NOT NULL REFERENCES sessions(id),
    ts             INTEGER NOT NULL,
    model          TEXT NOT NULL,
    input          INTEGER NOT NULL DEFAULT 0,
    output         INTEGER NOT NULL DEFAULT 0,
    cache_read     INTEGER NOT NULL DEFAULT 0,
    cache_write_5m INTEGER NOT NULL DEFAULT 0,
    cache_write_1h INTEGER NOT NULL DEFAULT 0,
    thinking       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_turns_ts ON turns(ts);
CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id);

CREATE TABLE IF NOT EXISTS prices (
    model                 TEXT PRIMARY KEY,
    input_per_mtok        REAL NOT NULL,
    output_per_mtok       REAL NOT NULL,
    cache_read_per_mtok   REAL NOT NULL,
    cache_write_per_mtok  REAL NOT NULL
);
"#;

pub fn open(path: &Path) -> Result<Db> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    init(conn)
}

pub fn open_in_memory() -> Result<Db> {
    init(Connection::open_in_memory()?)
}

fn init(conn: Connection) -> Result<Db> {
    // `PRAGMA journal_mode` returns a row, so it must go through execute_batch —
    // pragma_update errors with ExecuteReturnedResults on statements that yield rows.
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(Db { conn })
}

impl Db {
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> Result<i32> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Insert or update the derived columns only. User-owned columns are untouched.
    pub fn upsert_project(&self, slug: &str, real_path: &str, path_is_guess: bool) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO projects (slug, real_path, path_is_guess) VALUES (?1, ?2, ?3)
             ON CONFLICT(slug) DO UPDATE SET real_path = ?2, path_is_guess = ?3",
            params![slug, real_path, path_is_guess as i32],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM projects WHERE slug = ?1",
            params![slug],
            |r| r.get(0),
        )?)
    }

    pub fn set_parent(&self, child_id: i64, parent_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET parent_project_id = ?2 WHERE id = ?1",
            params![child_id, parent_id],
        )?;
        Ok(())
    }

    pub fn parent_of(&self, id: i64) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT parent_project_id FROM projects WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?)
    }

    pub fn set_note(&self, project_id: i64, note: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET note = ?2, note_updated_at = ?3 WHERE id = ?1",
            params![project_id, note, chrono::Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }

    pub fn note(&self, project_id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT note FROM projects WHERE id = ?1",
            params![project_id],
            |r| r.get(0),
        )?)
    }

    pub fn set_pinned(&self, project_id: i64, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET pinned = ?2 WHERE id = ?1",
            params![project_id, pinned as i32],
        )?;
        Ok(())
    }

    pub fn pinned(&self, project_id: i64) -> Result<bool> {
        let v: i32 = self.conn.query_row(
            "SELECT pinned FROM projects WHERE id = ?1",
            params![project_id],
            |r| r.get(0),
        )?;
        Ok(v != 0)
    }

    pub fn set_display_name(&self, project_id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET display_name = ?2 WHERE id = ?1",
            params![project_id, name],
        )?;
        Ok(())
    }

    pub fn display_name(&self, project_id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT display_name FROM projects WHERE id = ?1",
            params![project_id],
            |r| r.get(0),
        )?)
    }

    pub fn upsert_session(&self, s: &SessionRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, project_id, file_path, file_size, indexed_offset,
                 started_at, last_activity_at, cwd, git_branch, cc_version, message_count)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET
                 project_id = ?2, file_path = ?3, file_size = ?4, indexed_offset = ?5,
                 started_at = COALESCE(sessions.started_at, ?6),
                 last_activity_at = ?7, cwd = ?8, git_branch = ?9,
                 cc_version = ?10, message_count = ?11",
            params![
                s.id,
                s.project_id,
                s.file_path,
                s.file_size as i64,
                s.indexed_offset as i64,
                s.started_at,
                s.last_activity_at,
                s.cwd,
                s.git_branch,
                s.cc_version,
                s.message_count as i64
            ],
        )?;
        Ok(())
    }

    pub fn session_offset(&self, session_id: &str) -> Result<u64> {
        let found = self.conn.query_row(
            "SELECT indexed_offset FROM sessions WHERE id = ?1",
            params![session_id],
            |r| r.get::<_, i64>(0),
        );
        match found {
            Ok(v) => Ok(v.max(0) as u64),
            // Never scanned before — the normal first-pass signal.
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    pub fn insert_turns(&self, session_id: &str, turns: &[Turn]) -> Result<()> {
        if turns.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO turns (session_id, ts, model, input, output,
                     cache_read, cache_write_5m, cache_write_1h, thinking)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )?;
            for t in turns {
                stmt.execute(params![
                    session_id,
                    t.ts,
                    t.model,
                    t.usage.input as i64,
                    t.usage.output as i64,
                    t.usage.cache_read as i64,
                    t.usage.cache_write_5m as i64,
                    t.usage.cache_write_1h as i64,
                    t.usage.thinking as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Drop derived rows only. Never touches `projects`.
    pub fn rebuild_derived(&self) -> Result<()> {
        self.conn
            .execute_batch("DELETE FROM turns; DELETE FROM sessions;")?;
        Ok(())
    }

    pub fn project_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?)
    }

    pub fn session_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?)
    }

    pub fn turn_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))?)
    }

    pub fn session_message_count(&self, session_id: &str) -> Result<u64> {
        let found = self.conn.query_row(
            "SELECT message_count FROM sessions WHERE id = ?1",
            params![session_id],
            |r| r.get::<_, i64>(0),
        );
        match found {
            Ok(v) => Ok(v.max(0) as u64),
            // Never scanned before — the normal first-pass signal.
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    /// Used when a transcript was truncated or replaced and must be rescanned
    /// from zero: the previously stored turns are stale duplicates.
    pub fn delete_turns_for_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM turns WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn turn_totals(&self) -> Result<(i64, i64, i64, i64, i64)> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(input),0), COALESCE(SUM(output),0), COALESCE(SUM(cache_read),0),
                    COALESCE(SUM(cache_write_5m),0), COALESCE(SUM(cache_write_1h),0) FROM turns",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TurnUsage;

    fn session(id: &str, project_id: i64, offset: u64) -> SessionRecord {
        SessionRecord {
            id: id.to_string(),
            project_id,
            file_path: format!("/tmp/{id}.jsonl"),
            file_size: 100,
            indexed_offset: offset,
            started_at: Some(1_700_000_000_000),
            last_activity_at: Some(1_700_000_100_000),
            cwd: Some("/Users/a/proj".into()),
            git_branch: Some("main".into()),
            cc_version: Some("2.1.1".into()),
            message_count: 3,
        }
    }

    #[test]
    fn creates_schema_and_records_version() {
        let db = open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("i.db");
        {
            let db = open(&p).unwrap();
            db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        }
        let db = open(&p).unwrap();
        assert_eq!(db.project_count().unwrap(), 1);
    }

    #[test]
    fn upsert_project_is_stable_on_slug() {
        let db = open_in_memory().unwrap();
        let a = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        let b = db
            .upsert_project("slug", "/Users/a/proj-moved", false)
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(db.project_count().unwrap(), 1);
    }

    /// The property that protects user data.
    #[test]
    fn reindexing_preserves_user_owned_columns() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        db.set_note(id, "left off on the CSV parser").unwrap();
        db.set_pinned(id, true).unwrap();
        db.set_display_name(id, "Personal Dashboard").unwrap();

        // A later index run sees the same project again.
        db.upsert_project("slug", "/Users/a/proj", false).unwrap();

        assert_eq!(
            db.note(id).unwrap().as_deref(),
            Some("left off on the CSV parser")
        );
        assert!(db.pinned(id).unwrap());
        assert_eq!(
            db.display_name(id).unwrap().as_deref(),
            Some("Personal Dashboard")
        );
    }

    #[test]
    fn rebuild_derived_clears_sessions_and_turns_but_not_projects() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        db.set_note(id, "keep me").unwrap();
        db.upsert_session(&session("s1", id, 10)).unwrap();
        db.insert_turns(
            "s1",
            &[Turn {
                ts: 1,
                model: "m".into(),
                usage: TurnUsage {
                    input: 1,
                    ..Default::default()
                },
            }],
        )
        .unwrap();

        db.rebuild_derived().unwrap();

        assert_eq!(db.session_count().unwrap(), 0);
        assert_eq!(db.turn_count().unwrap(), 0);
        assert_eq!(db.project_count().unwrap(), 1);
        assert_eq!(db.note(id).unwrap().as_deref(), Some("keep me"));
    }

    #[test]
    fn session_offset_round_trips_and_defaults_to_zero() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        assert_eq!(db.session_offset("missing").unwrap(), 0);
        db.upsert_session(&session("s1", id, 4096)).unwrap();
        assert_eq!(db.session_offset("s1").unwrap(), 4096);
        db.upsert_session(&session("s1", id, 8192)).unwrap();
        assert_eq!(db.session_offset("s1").unwrap(), 8192);
        assert_eq!(db.session_count().unwrap(), 1);
    }

    #[test]
    fn inserts_turns_with_all_token_classes() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        db.upsert_session(&session("s1", id, 0)).unwrap();
        db.insert_turns(
            "s1",
            &[Turn {
                ts: 1_700_000_000_000,
                model: "claude-fable-5".into(),
                usage: TurnUsage {
                    input: 2,
                    output: 231,
                    cache_read: 24221,
                    cache_write_5m: 0,
                    cache_write_1h: 23848,
                    thinking: 79,
                },
            }],
        )
        .unwrap();

        let (i, o, r, w5, w1) = db.turn_totals().unwrap();
        assert_eq!((i, o, r, w5, w1), (2, 231, 24221, 0, 23848));
    }

    #[test]
    fn parent_project_can_be_set() {
        let db = open_in_memory().unwrap();
        let parent = db.upsert_project("p", "/Users/a/proj", false).unwrap();
        let child = db
            .upsert_project("c", "/Users/a/proj/.claude/worktrees/x", false)
            .unwrap();
        db.set_parent(child, Some(parent)).unwrap();
        assert_eq!(db.parent_of(child).unwrap(), Some(parent));
    }
}
