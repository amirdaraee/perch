//! SQLite persistence. User-owned columns survive re-indexing.

use crate::model::{SessionRecord, Turn};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 2;

pub struct Db {
    conn: Connection,
}

/// The user-owned half of a project row. Indexing never overwrites these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMeta {
    pub display_name: Option<String>,
    pub status: String,
    pub pinned: bool,
    pub note: Option<String>,
    pub archived: bool,
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
    title            TEXT,
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
    // rusqlite's default busy timeout is 0, so a concurrent writer (the
    // retained Tauri app resolves this same file) makes SQLITE_BUSY come
    // back immediately instead of after a real wait. Both apps write here
    // until the Tauri app is removed, so give a lock a few seconds to clear
    // before surfacing as an error.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // Read the on-disk version BEFORE creating tables: `CREATE TABLE IF NOT
    // EXISTS` is a no-op on an existing database, so a v1 database's `sessions`
    // table is left exactly as it was — this is the only place that still
    // knows what shape it used to be in.
    let existing_version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn, existing_version)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(Db { conn })
}

/// Migrate a database whose stored `user_version` predates the current
/// `SCHEMA_VERSION`. Must be safe to run on a fresh database (where `SCHEMA`
/// already created every column) as well as on an older one.
///
/// v1 -> v2: `sessions.title` is new. A database created before this column
/// existed needs `ALTER TABLE` to gain it; a fresh database already has it via
/// `SCHEMA` above. Either way, every transcript must be rescanned so existing
/// sessions can be backfilled with a title — turns are wholly derived from
/// transcripts, so dropping them loses nothing, and `projects` (with its
/// user-owned columns) is never touched.
fn migrate(conn: &Connection, existing_version: i32) -> Result<()> {
    if existing_version < 2 {
        let has_title: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'title'")?
            .exists([])?;
        if !has_title {
            conn.execute("ALTER TABLE sessions ADD COLUMN title TEXT", [])?;
        }
        conn.execute_batch("UPDATE sessions SET indexed_offset = 0; DELETE FROM turns;")?;
    }
    Ok(())
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

    pub fn project_meta(&self, project_id: i64) -> Result<ProjectMeta> {
        Ok(self.conn.query_row(
            "SELECT display_name, status, pinned, note, archived FROM projects WHERE id = ?1",
            params![project_id],
            |r| {
                Ok(ProjectMeta {
                    display_name: r.get(0)?,
                    status: r.get(1)?,
                    pinned: r.get::<_, i64>(2)? != 0,
                    note: r.get(3)?,
                    archived: r.get::<_, i64>(4)? != 0,
                })
            },
        )?)
    }

    pub fn set_archived(&self, project_id: i64, archived: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET archived = ?2 WHERE id = ?1",
            params![project_id, archived as i64],
        )?;
        Ok(())
    }

    pub fn set_status(&self, project_id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET status = ?2 WHERE id = ?1",
            params![project_id, status],
        )?;
        Ok(())
    }

    pub fn upsert_session(&self, s: &SessionRecord) -> Result<()> {
        upsert_session_on(&self.conn, s)
    }

    /// Apply the result of one scan atomically: a partial failure must never
    /// leave the offset advanced past turns that were not stored.
    ///
    /// The resume offset lives on the session row, so upserting the session and
    /// inserting its turns as two separate autocommit statements means a failed
    /// insert leaves a committed offset pointing past bytes whose turns were
    /// never stored — the next pass resumes beyond them and those turns are lost
    /// for good. One transaction covers the whole decision.
    pub fn apply_scan(&self, s: &SessionRecord, turns: &[Turn], restarted: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        if restarted {
            delete_turns_on(&tx, &s.id)?;
        }
        upsert_session_on(&tx, s)?;
        insert_turns_on(&tx, &s.id, turns)?;
        tx.commit()?;
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
        let tx = self.conn.unchecked_transaction()?;
        insert_turns_on(&tx, session_id, turns)?;
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
        delete_turns_on(&self.conn, session_id)
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

// The single definitions of the three statements that `apply_scan` composes.
// Taking a `&Connection` lets each run either standalone (autocommit, via the
// `Db` methods) or inside `apply_scan`'s transaction — a `Transaction` derefs
// to `Connection` — so the two paths can never drift apart.

fn upsert_session_on(conn: &Connection, s: &SessionRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions (id, project_id, file_path, file_size, indexed_offset,
             started_at, last_activity_at, cwd, git_branch, cc_version, title, message_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(id) DO UPDATE SET
             project_id = ?2, file_path = ?3, file_size = ?4, indexed_offset = ?5,
             -- `started_at` keeps the OLDEST value it has ever seen.
             started_at = COALESCE(sessions.started_at, ?6),
             -- The five below keep the LAST value they have ever seen: a pass
             -- that read no new bytes yields an all-`None` SessionMeta, and a
             -- bare assignment would null out perfectly good stored values on
             -- every re-index.
             last_activity_at = COALESCE(?7, sessions.last_activity_at),
             cwd              = COALESCE(?8, sessions.cwd),
             git_branch       = COALESCE(?9, sessions.git_branch),
             cc_version       = COALESCE(?10, sessions.cc_version),
             title            = COALESCE(?11, sessions.title),
             message_count = ?12",
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
            s.title,
            s.message_count as i64
        ],
    )?;
    Ok(())
}

fn insert_turns_on(conn: &Connection, session_id: &str, turns: &[Turn]) -> Result<()> {
    if turns.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare(
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
    Ok(())
}

fn delete_turns_on(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM turns WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
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
            title: None,
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

    /// The property that protects user data through the v1 -> v2 migration.
    /// A v1 database (no `title` column, `user_version = 1`) with real
    /// sessions, turns, and a project note must, after opening with the
    /// current code: gain the `title` column, have every session's
    /// `indexed_offset` reset to 0 (forcing a full rescan so titles can be
    /// backfilled), have `turns` emptied (they are wholly derived from
    /// transcripts) — and still have the project's note.
    #[test]
    fn v1_database_migrates_and_preserves_the_project_note() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v1.db");

        {
            // Build a v1-shaped database directly, bypassing today's `SCHEMA`
            // (which already has `title`), so this test still means something
            // once `SCHEMA` moves on.
            let conn = Connection::open(&p).unwrap();
            conn.execute_batch(
                "CREATE TABLE projects (
                    id                INTEGER PRIMARY KEY,
                    slug              TEXT NOT NULL UNIQUE,
                    real_path         TEXT NOT NULL,
                    path_is_guess     INTEGER NOT NULL DEFAULT 0,
                    parent_project_id INTEGER REFERENCES projects(id),
                    display_name      TEXT,
                    status            TEXT NOT NULL DEFAULT 'active',
                    pinned            INTEGER NOT NULL DEFAULT 0,
                    note              TEXT,
                    note_updated_at   INTEGER,
                    archived          INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE sessions (
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
                CREATE TABLE turns (
                    session_id     TEXT NOT NULL REFERENCES sessions(id),
                    ts             INTEGER NOT NULL,
                    model          TEXT NOT NULL,
                    input          INTEGER NOT NULL DEFAULT 0,
                    output         INTEGER NOT NULL DEFAULT 0,
                    cache_read     INTEGER NOT NULL DEFAULT 0,
                    cache_write_5m INTEGER NOT NULL DEFAULT 0,
                    cache_write_1h INTEGER NOT NULL DEFAULT 0,
                    thinking       INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, slug, real_path, note) VALUES (1, 'slug', '/a/proj', 'left off on the CSV parser')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, project_id, file_path, file_size, indexed_offset, message_count)
                 VALUES ('s1', 1, '/tmp/s1.jsonl', 500, 500, 3)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO turns (session_id, ts, model, input, output) VALUES ('s1', 1, 'm', 1, 2)",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1i32).unwrap();
        }

        // Opening with today's code must run the v1 -> v2 migration.
        let db = open(&p).unwrap();

        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);

        // `title` column exists and is settable.
        db.upsert_session(&session("s1", 1, 500)).unwrap();

        // Offsets reset, turns cleared: the next index pass rescans from zero.
        assert_eq!(
            db.session_offset("s1").unwrap(),
            500,
            "upsert_session above just set it back to 500; check it moved through 0 first"
        );
        assert_eq!(db.turn_count().unwrap(), 0, "turns must be cleared");

        // The user's note survives untouched.
        assert_eq!(
            db.note(1).unwrap().as_deref(),
            Some("left off on the CSV parser"),
            "the migration must never touch user-owned project columns"
        );
    }

    /// Same migration, checked before any write re-touches the session: the
    /// offset the migration itself produced must be 0.
    #[test]
    fn v1_migration_resets_offsets_before_any_new_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v1.db");
        {
            let conn = Connection::open(&p).unwrap();
            conn.execute_batch(
                "CREATE TABLE projects (id INTEGER PRIMARY KEY, slug TEXT NOT NULL UNIQUE,
                     real_path TEXT NOT NULL, path_is_guess INTEGER NOT NULL DEFAULT 0,
                     parent_project_id INTEGER, display_name TEXT,
                     status TEXT NOT NULL DEFAULT 'active', pinned INTEGER NOT NULL DEFAULT 0,
                     note TEXT, note_updated_at INTEGER, archived INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE sessions (id TEXT PRIMARY KEY, project_id INTEGER NOT NULL,
                     file_path TEXT NOT NULL, file_size INTEGER NOT NULL DEFAULT 0,
                     indexed_offset INTEGER NOT NULL DEFAULT 0, started_at INTEGER,
                     last_activity_at INTEGER, cwd TEXT, git_branch TEXT, cc_version TEXT,
                     message_count INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE turns (session_id TEXT NOT NULL, ts INTEGER NOT NULL,
                     model TEXT NOT NULL, input INTEGER NOT NULL DEFAULT 0,
                     output INTEGER NOT NULL DEFAULT 0, cache_read INTEGER NOT NULL DEFAULT 0,
                     cache_write_5m INTEGER NOT NULL DEFAULT 0, cache_write_1h INTEGER NOT NULL DEFAULT 0,
                     thinking INTEGER NOT NULL DEFAULT 0);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, slug, real_path) VALUES (1, 'slug', '/a/proj')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, project_id, indexed_offset, file_path) VALUES ('s1', 1, 999, '/tmp/s1.jsonl')",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1i32).unwrap();
        }

        let db = open(&p).unwrap();
        assert_eq!(
            db.session_offset("s1").unwrap(),
            0,
            "the migration itself must reset the offset to 0"
        );
    }

    /// Reopening an already-migrated (v2) database must be a true no-op: it
    /// must not reset offsets or clear turns on every ordinary launch.
    #[test]
    fn reopening_a_v2_database_does_not_rescan_or_touch_projects() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v2.db");
        {
            let db = open(&p).unwrap();
            let id = db.upsert_project("slug", "/a/proj", false).unwrap();
            db.set_note(id, "keep me").unwrap();
            db.upsert_session(&session("s1", id, 4096)).unwrap();
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
        }

        let db = open(&p).unwrap();
        assert_eq!(
            db.session_offset("s1").unwrap(),
            4096,
            "reopening a current-schema database must not force a rescan"
        );
        assert_eq!(db.turn_count().unwrap(), 1, "turns must survive a reopen");
        assert_eq!(db.note(1).unwrap().as_deref(), Some("keep me"));
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

    fn stored_title(db: &Db, session_id: &str) -> Option<String> {
        db.conn()
            .query_row(
                "SELECT title FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// `title` is LATEST-wins, same as `git_branch` and `cc_version`: a later
    /// pass over a range with no `ai-title` line must not null out a title a
    /// previous pass stored.
    #[test]
    fn title_round_trips_and_a_later_pass_with_no_title_does_not_erase_it() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("slug", "/a/proj", false).unwrap();

        let mut s = session("s1", id, 100);
        s.title = Some("Claude projects dashboard".into());
        db.upsert_session(&s).unwrap();
        assert_eq!(
            stored_title(&db, "s1").as_deref(),
            Some("Claude projects dashboard")
        );

        // A later incremental pass over new bytes with no `ai-title` line.
        let mut s2 = session("s1", id, 200);
        s2.title = None;
        db.upsert_session(&s2).unwrap();
        assert_eq!(
            stored_title(&db, "s1").as_deref(),
            Some("Claude projects dashboard"),
            "a pass with no title must not erase the one already stored"
        );

        // The title can still be refined: last-wins when one IS present.
        let mut s3 = session("s1", id, 300);
        s3.title = Some("refined title".into());
        db.upsert_session(&s3).unwrap();
        assert_eq!(stored_title(&db, "s1").as_deref(), Some("refined title"));
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

    #[test]
    fn project_meta_defaults_are_sane_for_a_fresh_project() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        let m = db.project_meta(id).unwrap();
        assert_eq!(m.display_name, None);
        assert_eq!(m.status, "active");
        assert!(!m.pinned);
        assert_eq!(m.note, None);
        assert!(!m.archived, "a new project is not archived");
    }

    #[test]
    fn archived_and_status_round_trip_and_survive_reindexing() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.set_archived(id, true).unwrap();
        db.set_status(id, "done").unwrap();
        db.set_note(id, "left off at the parser").unwrap();

        // Re-indexing upserts the same project; user-owned columns must survive.
        let same = db.upsert_project("-a-b", "/a/b", false).unwrap();
        assert_eq!(same, id);

        let m = db.project_meta(id).unwrap();
        assert!(m.archived);
        assert_eq!(m.status, "done");
        assert_eq!(m.note.as_deref(), Some("left off at the parser"));
    }

    #[test]
    fn archived_can_be_turned_back_off() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.set_archived(id, true).unwrap();
        db.set_archived(id, false).unwrap();
        assert!(!db.project_meta(id).unwrap().archived);
    }

    #[test]
    fn project_meta_for_an_unknown_id_is_an_error_not_a_default() {
        let db = open_in_memory().unwrap();
        assert!(
            db.project_meta(9999).is_err(),
            "a missing project must not read as defaults"
        );
    }
}
