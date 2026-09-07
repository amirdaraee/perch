//! SQLite persistence. User-owned columns survive re-indexing.

use crate::model::{SessionRecord, Turn};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 3;

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
    pub notify: NotifyOverride,
}

/// Per-project override of the global notification setting. Stored on the
/// project itself (below the user-owned line) so re-indexing never touches
/// it, same as `note` and `pinned`. `Serialize` so it can ride along on
/// `ui::main_window::ProjectDetail`, which derives it uniformly with every
/// other view-model in this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum NotifyOverride {
    /// Follow whatever the global setting says.
    Default,
    /// Never notify for this project, regardless of the global setting.
    Off,
    /// Notify after this many minutes, regardless of the global setting.
    Custom { after_minutes: u32 },
}

/// Same bound `Settings::validated` clamps `waiting_after_minutes` to. Rust
/// owns this validation everywhere, not just in the global setting, so a
/// shell (Swift stepper or otherwise) can't store a nonsense value by simply
/// skipping its own bounds check.
const NOTIFY_AFTER_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 1..=240;

impl NotifyOverride {
    fn mode_str(&self) -> &'static str {
        match self {
            NotifyOverride::Default => "default",
            NotifyOverride::Off => "off",
            NotifyOverride::Custom { .. } => "custom",
        }
    }

    fn after_minutes(&self) -> Option<u32> {
        match self {
            NotifyOverride::Custom { after_minutes } => Some(*after_minutes),
            _ => None,
        }
    }

    /// Clamp `Custom { after_minutes }` into `NOTIFY_AFTER_MINUTES_RANGE`,
    /// mirroring `Settings::validated`'s clamp of `waiting_after_minutes`.
    /// `Default` and `Off` carry no minute count and are returned unchanged.
    fn clamped(self) -> NotifyOverride {
        match self {
            NotifyOverride::Custom { after_minutes } => NotifyOverride::Custom {
                after_minutes: after_minutes.clamp(
                    *NOTIFY_AFTER_MINUTES_RANGE.start(),
                    *NOTIFY_AFTER_MINUTES_RANGE.end(),
                ),
            },
            other => other,
        }
    }

    /// Reconstruct from the stored columns. An unknown `mode` string (written
    /// by a newer Perch) falls back to `Default` rather than failing — the
    /// same tolerance the settings enum already has.
    fn from_columns(mode: &str, after_minutes: Option<i64>) -> NotifyOverride {
        match mode {
            "off" => NotifyOverride::Off,
            "custom" => match after_minutes {
                Some(n) => NotifyOverride::Custom {
                    after_minutes: n as u32,
                },
                None => NotifyOverride::Default,
            },
            "default" => NotifyOverride::Default,
            _ => NotifyOverride::Default,
        }
    }
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
    archived             INTEGER NOT NULL DEFAULT 0,
    notify_mode          TEXT NOT NULL DEFAULT 'default',
    notify_after_minutes INTEGER
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
        // Unlike `has_title` above, this reset has no "is this actually a v1
        // database" guard of its own — it runs for every `existing_version <
        // 2`, which includes a brand-new database (`user_version` starts at
        // 0). That is harmless only because `migrate` is called from `init`
        // immediately after `SCHEMA` creates the tables and before any row
        // can exist: `sessions` and `turns` are still empty, so the reset and
        // delete are no-ops. If this call is ever moved to run later, after
        // real data could already be present, this becomes a silent
        // data-loss path and needs its own guard.
        conn.execute_batch("UPDATE sessions SET indexed_offset = 0; DELETE FROM turns;")?;
    }
    if existing_version < 3 {
        // v2 -> v3: `notify_mode` / `notify_after_minutes` are new, user-owned
        // columns on `projects` (the per-project notification override).
        // Unlike the v1 -> v2 step above, adding them needs no re-scan —
        // nothing derived from transcripts changes shape — so `sessions` and
        // `turns` are left completely alone here.
        let has_notify_mode: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('projects') WHERE name = 'notify_mode'")?
            .exists([])?;
        if !has_notify_mode {
            conn.execute(
                "ALTER TABLE projects ADD COLUMN notify_mode TEXT NOT NULL DEFAULT 'default'",
                [],
            )?;
        }
        let has_notify_after_minutes: bool = conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('projects') WHERE name = 'notify_after_minutes'",
            )?
            .exists([])?;
        if !has_notify_after_minutes {
            conn.execute(
                "ALTER TABLE projects ADD COLUMN notify_after_minutes INTEGER",
                [],
            )?;
        }
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
            "SELECT display_name, status, pinned, note, archived, notify_mode, notify_after_minutes
             FROM projects WHERE id = ?1",
            params![project_id],
            |r| {
                let mode: String = r.get(5)?;
                let after_minutes: Option<i64> = r.get(6)?;
                Ok(ProjectMeta {
                    display_name: r.get(0)?,
                    status: r.get(1)?,
                    pinned: r.get::<_, i64>(2)? != 0,
                    note: r.get(3)?,
                    archived: r.get::<_, i64>(4)? != 0,
                    notify: NotifyOverride::from_columns(&mode, after_minutes),
                })
            },
        )?)
    }

    /// Set the per-project notification override. `notify_mode` and
    /// `notify_after_minutes` are user-owned columns on `projects` (below the
    /// "never overwritten by indexing" line), so this never touches the
    /// derived columns `upsert_project` maintains.
    pub fn set_notify_override(&self, project_id: i64, o: &NotifyOverride) -> Result<()> {
        let o = o.clamped();
        self.conn.execute(
            "UPDATE projects SET notify_mode = ?2, notify_after_minutes = ?3 WHERE id = ?1",
            params![project_id, o.mode_str(), o.after_minutes()],
        )?;
        Ok(())
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

    /// The most recent turn timestamp anywhere in the index, or `None` when
    /// nothing has been indexed yet. `ui::diagnostics` uses this as its one
    /// freshness signal ("last_indexed") rather than a wall-clock time of
    /// when a scan last ran, which nothing in this schema records.
    pub fn last_turn_ts(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT MAX(ts) FROM turns", [], |r| r.get(0))?)
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

        // Offset reset and turns cleared, read BEFORE any new write touches
        // this session — otherwise a later `upsert_session` call could set
        // the offset back to a nonzero value and this would pass whether or
        // not the migration ever reset it.
        assert_eq!(
            db.session_offset("s1").unwrap(),
            0,
            "the migration must reset the offset so the next index pass rescans from zero"
        );
        assert_eq!(db.turn_count().unwrap(), 0, "turns must be cleared");

        // `title` column exists and is settable.
        db.upsert_session(&session("s1", 1, 500)).unwrap();
        assert_eq!(db.session_offset("s1").unwrap(), 500);

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
    fn last_turn_ts_is_none_until_something_is_indexed() {
        let db = open_in_memory().unwrap();
        assert_eq!(
            db.last_turn_ts().unwrap(),
            None,
            "an empty index has no last-indexed timestamp to report"
        );

        let id = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        db.upsert_session(&session("s1", id, 0)).unwrap();
        db.insert_turns(
            "s1",
            &[
                Turn {
                    ts: 10,
                    model: "m".into(),
                    usage: TurnUsage::default(),
                },
                Turn {
                    ts: 30,
                    model: "m".into(),
                    usage: TurnUsage::default(),
                },
                Turn {
                    ts: 20,
                    model: "m".into(),
                    usage: TurnUsage::default(),
                },
            ],
        )
        .unwrap();

        assert_eq!(
            db.last_turn_ts().unwrap(),
            Some(30),
            "the newest turn's timestamp, not insertion order"
        );
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

    #[test]
    fn a_project_defaults_to_following_the_global_setting() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        assert_eq!(db.project_meta(id).unwrap().notify, NotifyOverride::Default);
    }

    #[test]
    fn the_three_override_states_round_trip() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        for want in [
            NotifyOverride::Off,
            NotifyOverride::Custom { after_minutes: 45 },
            NotifyOverride::Default,
        ] {
            db.set_notify_override(id, &want).unwrap();
            assert_eq!(db.project_meta(id).unwrap().notify, want);
        }
    }

    #[test]
    fn an_override_survives_reindexing_like_every_other_user_owned_column() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.set_notify_override(id, &NotifyOverride::Custom { after_minutes: 20 })
            .unwrap();
        db.set_note(id, "keep me").unwrap();

        assert_eq!(db.upsert_project("-a-b", "/a/b", false).unwrap(), id);

        let m = db.project_meta(id).unwrap();
        assert_eq!(m.notify, NotifyOverride::Custom { after_minutes: 20 });
        assert_eq!(m.note.as_deref(), Some("keep me"));
    }

    #[test]
    fn an_unknown_notify_mode_string_degrades_to_default() {
        // A newer Perch's mode value must not break an older one reading it.
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.conn()
            .execute(
                "UPDATE projects SET notify_mode = 'some-future-mode' WHERE id = ?1",
                params![id],
            )
            .unwrap();
        assert_eq!(db.project_meta(id).unwrap().notify, NotifyOverride::Default);
    }

    #[test]
    fn a_custom_mode_with_no_minute_count_degrades_to_default() {
        // Unlike the unknown-mode test above, this exercises the "custom"
        // arm of `from_columns` specifically: a `notify_mode = 'custom'` row
        // whose `notify_after_minutes` is NULL (never produced by
        // `set_notify_override` itself, but a hand-edited or corrupted row
        // could have it) must not panic or fabricate a minute count.
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.conn()
            .execute(
                "UPDATE projects SET notify_mode = 'custom', notify_after_minutes = NULL WHERE id = ?1",
                params![id],
            )
            .unwrap();
        assert_eq!(db.project_meta(id).unwrap().notify, NotifyOverride::Default);
    }

    #[test]
    fn custom_after_minutes_is_clamped_to_the_same_bound_as_the_global_setting() {
        // Rust must own this validation everywhere, not just in the Swift
        // stepper: `set_notify_override` clamps into the same 1..=240 range
        // `Settings::validated` uses for `waiting_after_minutes`.
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("-a-b", "/a/b", false).unwrap();

        db.set_notify_override(id, &NotifyOverride::Custom { after_minutes: 0 })
            .unwrap();
        assert_eq!(
            db.project_meta(id).unwrap().notify,
            NotifyOverride::Custom { after_minutes: 1 },
            "0 must clamp up to the lower bound"
        );

        db.set_notify_override(
            id,
            &NotifyOverride::Custom {
                after_minutes: 9_999,
            },
        )
        .unwrap();
        assert_eq!(
            db.project_meta(id).unwrap().notify,
            NotifyOverride::Custom { after_minutes: 240 },
            "an absurdly large value must clamp down to the upper bound"
        );
    }

    #[test]
    fn the_v3_migration_adds_the_columns_without_touching_indexed_data() {
        // A v2-shaped database with real turns: unlike v2, this migration must
        // not force a re-scan, so nothing indexed may be lost.
        let db = open_in_memory().unwrap();
        let pid = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "s1".into(),
            project_id: pid,
            file_path: "/tmp/s1.jsonl".into(),
            file_size: 0,
            indexed_offset: 777,
            started_at: Some(1),
            last_activity_at: Some(2),
            cwd: None,
            git_branch: None,
            cc_version: None,
            message_count: 1,
            title: None,
        })
        .unwrap();
        db.insert_turns(
            "s1",
            &[Turn {
                ts: 2,
                model: "m".into(),
                usage: TurnUsage::default(),
            }],
        )
        .unwrap();

        assert_eq!(db.turn_count().unwrap(), 1, "turns survive a v3 migration");
        assert_eq!(
            db.session_offset("s1").unwrap(),
            777,
            "offsets are not reset"
        );
        assert_eq!(
            db.project_meta(pid).unwrap().notify,
            NotifyOverride::Default
        );
    }

    /// Stronger version of the above: build a genuinely v2-stamped database
    /// on disk (no `notify_mode`/`notify_after_minutes` columns, `user_version
    /// = 2`) with real indexed data already present *before* the v3 migration
    /// runs, then reopen with today's code. This is the regression the v2
    /// migration's own comment warns about: a reset/delete that runs after
    /// real rows exist would be a silent data-loss path.
    #[test]
    fn v2_database_migrates_to_v3_and_preserves_turns_offsets_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v2.db");
        {
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
                    title            TEXT,
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
                "INSERT INTO projects (id, slug, real_path, note) VALUES (1, 'slug', '/a/proj', 'keep me')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, project_id, file_path, file_size, indexed_offset, message_count)
                 VALUES ('s1', 1, '/tmp/s1.jsonl', 900, 900, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO turns (session_id, ts, model, input, output) VALUES ('s1', 2, 'm', 1, 2)",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 2i32).unwrap();
        }

        let db = open(&p).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(
            db.session_offset("s1").unwrap(),
            900,
            "a v2 -> v3 migration must not reset offsets"
        );
        assert_eq!(
            db.turn_count().unwrap(),
            1,
            "a v2 -> v3 migration must not delete turns"
        );
        assert_eq!(db.note(1).unwrap().as_deref(), Some("keep me"));
        assert_eq!(db.project_meta(1).unwrap().notify, NotifyOverride::Default);
    }
}
