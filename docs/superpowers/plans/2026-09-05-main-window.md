# Perch Main Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Perch's main window — every project (active or archived) with its stats and full session history, a usage view, and the two terminal actions (resume a past session, open a fresh one) — as a second window in the existing native macOS app.

**Architecture:** `perch-core` gains `ui::main_window`, `ui::usage`, and `actions` — the whole view-model plus command composition, all platform-neutral. `perch-ffi` mirrors them and adds the edit methods. The Swift app gains an `NSWindow` hosting SwiftUI: a sidebar of projects, a project detail pane, and a Usage tab drawn with Swift Charts. Rust composes the terminal command; Swift spawns it via a one-shot script and `open -a`.

**Tech Stack:** Rust 1.98, rusqlite (bundled SQLite), `uniffi` 0.32, Swift 6.3 / SwiftPM, SwiftUI + Swift Charts (in the macOS SDK — no new dependency), AppKit `NSWindow`, macOS 15+.

**Spec:** `docs/superpowers/specs/2026-09-05-main-window-design.md` — implements §2–§9. The 2026-08-30 spec's §9.2/§9.3/§9.4 and the 2026-09-04 native-app design remain binding.

## Global Constraints

- **Rust owns everything except drawing.** Every string the UI shows is produced in `perch-core`. Chart *values* cross as numbers; their *labels* cross as finished strings. A `String(format:)`, `NumberFormatter`, `DateFormatter`, or comparison against a Rust sentinel in Swift is a defect.
- **Read-only against the Claude Code directory.** Only `perch-core` reads it; nothing writes to it. Perch's own database and the one-shot launch script live in Perch's application-support directory.
- **No network, no telemetry** anywhere. **No new SPM or Cargo dependency** — Swift Charts is in the SDK; adding a package would be Perch's first fetch and weakens the CI audit.
- `perch-core` and `perch-ffi` must keep compiling on Linux; no `cfg(target_os)` outside the existing app-data path. `actions.rs` composes commands only — it must not spawn a process or write a file.
- Rust edition 2021; `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass. **114 tests pass before this plan starts.**
- The Tauri app (`src-tauri`, `src/`) stays untouched and must keep building (`cargo build -p perch-app`).
- Minimum macOS 15. Swift 6 for app code; the generated `PerchFFI` target stays at language mode v5.
- Generated output (`apps/macos/Perch/Sources/PerchFFI/`, `Frameworks/`) is git-ignored and must never be committed. Re-run `scripts/build-xcframework.sh` after any FFI change.
- **Never read `apps/macos/Perch/Sources/PerchFFI/perch_ffi.swift`** — 49 KB of generated code that has stalled agents. Use the API doc the controller provides.
- Run every command in the foreground; keep tool output small.

---

## File Structure

```
crates/perch-core/src/
├── db.rs              + set_archived, set_status, project_meta        (Task 1)
├── query.rs           + session_history, daily_usage, top_projects    (Task 2)
├── actions.rs   NEW   TerminalCommand + shell_line                     (Task 5)
└── ui/
    ├── main_window.rs NEW  ProjectRow, MainWindowModel, ProjectDetail  (Task 3)
    └── usage.rs       NEW  UsageModel and its parts                    (Task 4)

crates/perch-ffi/src/lib.rs   mirrors + new methods                     (Task 6)

apps/macos/Perch/Sources/Perch/
├── MainWindow/
│   ├── MainWindowController.swift  window + activation policy          (Task 7)
│   ├── Sidebar.swift               grouped project list                (Task 8)
│   ├── ProjectDetail.swift         note, stats, sparkline, history     (Task 9)
│   ├── Launcher.swift              script + `open -a`                  (Task 9)
│   └── UsageView.swift             the four §9.3 blocks                (Task 10)
└── StatusItemController.swift      + "Open Perch" item                 (Task 7)

.github/workflows/ci.yml, README.md, docs/BACKLOG.md                    (Task 11)
```

---

## Task 1: Project user-state — read and write

**Files:**
- Modify: `crates/perch-core/src/db.rs`

**Interfaces:**
- Consumes: existing `Db`, `set_pinned`, `pinned`, `set_note`, `note`, `set_display_name`, `display_name`.
- Produces:
  - `pub struct ProjectMeta { pub display_name: Option<String>, pub status: String, pub pinned: bool, pub note: Option<String>, pub archived: bool }`
  - `pub fn project_meta(&self, project_id: i64) -> Result<ProjectMeta>`
  - `pub fn set_archived(&self, project_id: i64, archived: bool) -> Result<()>`
  - `pub fn set_status(&self, project_id: i64, status: &str) -> Result<()>`

The `projects` table already has `display_name`, `status`, `pinned`, `note`, `note_updated_at`, `archived` — all marked "user-owned; never overwritten by indexing". This task only adds accessors.

- [ ] **Step 1: Write the failing tests**

Append to `db.rs`'s `mod tests`:

```rust
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
        assert!(db.project_meta(9999).is_err(), "a missing project must not read as defaults");
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core db::tests::project_meta`
Expected: FAIL — `no method named project_meta`.

- [ ] **Step 3: Implement**

Add near the other user-owned accessors in `db.rs`:

```rust
/// The user-owned half of a project row. Indexing never overwrites these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMeta {
    pub display_name: Option<String>,
    pub status: String,
    pub pinned: bool,
    pub note: Option<String>,
    pub archived: bool,
}

impl Db {
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
}
```

Put these inside the existing `impl Db` block rather than opening a second one if the file's style keeps a single block.

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core db` → PASS.
Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/db.rs
git commit -m "feat(core): project archive/status accessors and a meta reader"
```

---

## Task 2: Queries the window needs

**Files:**
- Modify: `crates/perch-core/src/query.rs`

**Interfaces:**
- Consumes: `Db`, `TurnUsage`, the private helpers `usage_rows`, `SUMS`, `priced_usage`, and `session_usage`.
- Produces:
  - `pub struct HistorySession { pub id: String, pub started_at: Option<i64>, pub last_activity_at: Option<i64>, pub git_branch: Option<String>, pub message_count: u64, pub usage: TurnUsage, pub cost_usd: f64 }`
  - `pub fn session_history(db: &Db, project_id: i64) -> Result<Vec<HistorySession>>` — newest first by `last_activity_at`, nulls last.
  - `pub struct DayUsage { pub day_start_ms: i64, pub usage: TurnUsage, pub cost_usd: f64 }`
  - `pub fn daily_usage(db: &Db, days: usize, now_ms: i64) -> Result<Vec<DayUsage>>` — exactly `days` entries ending with the day containing `now_ms`, **including days with no activity** (zeroed), oldest first.
  - `pub fn top_projects(db: &Db, since_ms: i64, limit: usize) -> Result<Vec<(String, TurnUsage, f64)>>` — `(display label, usage, cost)` ranked by total tokens descending.

Day boundaries are UTC (`day_start_ms = ts - ts.rem_euclid(86_400_000)`), matching how the rest of the crate treats timestamps.

- [ ] **Step 1: Write the failing tests**

Append to `query.rs`'s `mod tests` (reuse the module's existing `seed_session` helper):

```rust
    const DAY: i64 = 86_400_000;

    #[test]
    fn session_history_is_newest_first_with_usage_attached() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        seed_session(&db, pid, "old", "/a/p", 1_000, 1_000_000);
        seed_session(&db, pid, "new", "/a/p", 5_000, 2_000_000);

        let rows = session_history(&db, pid).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
        assert_eq!(rows[0].usage.input, 2_000_000);
        assert!((rows[0].cost_usd - 30.0).abs() < 1e-9, "2 MTok fable input at 15.0");
    }

    #[test]
    fn session_history_for_a_project_with_no_sessions_is_empty_not_an_error() {
        let db = open_in_memory().unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        assert!(session_history(&db, pid).unwrap().is_empty());
    }

    #[test]
    fn daily_usage_includes_quiet_days_as_zeroes() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        let now = 10 * DAY + 3_600_000; // mid-day on day 10
        // Activity on day 10 and day 8 only; day 9 must still appear, zeroed.
        seed_session(&db, pid, "s10", "/a/p", 10 * DAY + 1, 1_000_000);
        seed_session(&db, pid, "s8", "/a/p", 8 * DAY + 1, 3_000_000);

        let days = daily_usage(&db, 3, now).unwrap();
        assert_eq!(days.len(), 3, "exactly the window requested");
        assert_eq!(days[0].day_start_ms, 8 * DAY, "oldest first");
        assert_eq!(days[2].day_start_ms, 10 * DAY);
        assert_eq!(days[0].usage.input, 3_000_000);
        assert_eq!(days[1].usage.total_tokens(), 0, "a quiet day is present and zeroed");
        assert_eq!(days[1].cost_usd, 0.0);
        assert_eq!(days[2].usage.input, 1_000_000);
    }

    #[test]
    fn daily_usage_with_an_empty_index_still_returns_the_full_window() {
        let db = open_in_memory().unwrap();
        let days = daily_usage(&db, 14, 100 * DAY).unwrap();
        assert_eq!(days.len(), 14);
        assert!(days.iter().all(|d| d.usage.total_tokens() == 0));
    }

    #[test]
    fn top_projects_ranks_by_tokens_and_respects_the_limit() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let big = db.upsert_project("-a-big", "/a/big", false).unwrap();
        let mid = db.upsert_project("-a-mid", "/a/mid", false).unwrap();
        let small = db.upsert_project("-a-small", "/a/small", false).unwrap();
        seed_session(&db, big, "b", "/a/big", 5_000, 9_000_000);
        seed_session(&db, mid, "m", "/a/mid", 5_000, 5_000_000);
        seed_session(&db, small, "s", "/a/small", 5_000, 1_000_000);

        let rows = top_projects(&db, 0, 2).unwrap();
        assert_eq!(rows.len(), 2, "limit honoured");
        assert_eq!(rows[0].0, "big", "ranked by tokens, labelled by directory name");
        assert_eq!(rows[1].0, "mid");
        assert!(rows[0].1.input > rows[1].1.input);
    }

    #[test]
    fn top_projects_excludes_activity_before_the_cutoff() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        seed_session(&db, pid, "old", "/a/p", 1_000, 5_000_000);
        assert!(top_projects(&db, 10_000, 5).unwrap().is_empty(), "all activity predates the cutoff");
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core query`
Expected: FAIL — `cannot find function session_history`.

- [ ] **Step 3: Implement**

Add to `query.rs`:

```rust
pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone)]
pub struct HistorySession {
    pub id: String,
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub git_branch: Option<String>,
    pub message_count: u64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
}

/// Every session ever recorded for a project, newest first. Sessions with no
/// recorded activity sort last rather than being dropped — they exist, and the
/// window's job is to show what exists.
pub fn session_history(db: &Db, project_id: i64) -> Result<Vec<HistorySession>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, started_at, last_activity_at, git_branch, message_count
         FROM sessions WHERE project_id = ?1
         ORDER BY last_activity_at IS NULL, last_activity_at DESC",
    )?;
    let rows = stmt.query_map(params![project_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)? as u64,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, started_at, last_activity_at, git_branch, message_count) = row?;
        let (usage, cost_usd) = session_usage(db, &id)?;
        out.push(HistorySession {
            id,
            started_at,
            last_activity_at,
            git_branch,
            message_count,
            usage,
            cost_usd,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct DayUsage {
    pub day_start_ms: i64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
}

fn day_start(ts_ms: i64) -> i64 {
    ts_ms - ts_ms.rem_euclid(DAY_MS)
}

/// `days` consecutive days ending with the one containing `now_ms`, oldest first.
/// Quiet days are present and zeroed: a bar chart with gaps silently lies about
/// the shape of the week.
pub fn daily_usage(db: &Db, days: usize, now_ms: i64) -> Result<Vec<DayUsage>> {
    let last = day_start(now_ms);
    let first = last - (days as i64 - 1).max(0) * DAY_MS;

    let mut per_day: std::collections::HashMap<i64, (TurnUsage, f64)> = std::collections::HashMap::new();
    let mut stmt = db.conn().prepare(&format!(
        "SELECT (ts - (ts % {DAY_MS} + {DAY_MS}) % {DAY_MS}) AS day, model, {SUMS}
         FROM turns WHERE ts >= ?1 GROUP BY day, model"
    ))?;
    let rows = stmt.query_map(params![first], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            TurnUsage {
                input: r.get::<_, i64>(2)? as u64,
                output: r.get::<_, i64>(3)? as u64,
                cache_read: r.get::<_, i64>(4)? as u64,
                cache_write_5m: r.get::<_, i64>(5)? as u64,
                cache_write_1h: r.get::<_, i64>(6)? as u64,
                thinking: r.get::<_, i64>(7)? as u64,
            },
        ))
    })?;
    for row in rows {
        let (day, model, u) = row?;
        let cost = cost_of(db, &model, &u)?;
        let slot = per_day.entry(day).or_insert((TurnUsage::default(), 0.0));
        slot.0 = slot.0.plus(&u);
        slot.1 += cost;
    }

    Ok((0..days)
        .map(|i| {
            let day_start_ms = first + i as i64 * DAY_MS;
            let (usage, cost_usd) = per_day.get(&day_start_ms).cloned().unwrap_or_default();
            DayUsage { day_start_ms, usage, cost_usd }
        })
        .collect())
}

/// Projects ranked by tokens since `since_ms`. The label is the directory name,
/// which is what the user recognises — not the slug and not the whole path.
pub fn top_projects(db: &Db, since_ms: i64, limit: usize) -> Result<Vec<(String, TurnUsage, f64)>> {
    let mut stmt = db.conn().prepare(&format!(
        "SELECT p.real_path, p.display_name, t.model, {SUMS}
         FROM turns t
         JOIN sessions s ON s.id = t.session_id
         JOIN projects p ON p.id = s.project_id
         WHERE t.ts >= ?1
         GROUP BY p.id, t.model"
    ))?;
    let rows = stmt.query_map(params![since_ms], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
            TurnUsage {
                input: r.get::<_, i64>(3)? as u64,
                output: r.get::<_, i64>(4)? as u64,
                cache_read: r.get::<_, i64>(5)? as u64,
                cache_write_5m: r.get::<_, i64>(6)? as u64,
                cache_write_1h: r.get::<_, i64>(7)? as u64,
                thinking: r.get::<_, i64>(8)? as u64,
            },
        ))
    })?;

    let mut totals: std::collections::HashMap<String, (TurnUsage, f64)> = std::collections::HashMap::new();
    for row in rows {
        let (real_path, display_name, model, u) = row?;
        let label = display_name.unwrap_or_else(|| {
            std::path::Path::new(&real_path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(real_path.clone())
        });
        let cost = cost_of(db, &model, &u)?;
        let slot = totals.entry(label).or_insert((TurnUsage::default(), 0.0));
        slot.0 = slot.0.plus(&u);
        slot.1 += cost;
    }

    let mut out: Vec<(String, TurnUsage, f64)> =
        totals.into_iter().map(|(k, (u, c))| (k, u, c)).collect();
    out.sort_by(|a, b| {
        b.1.total_tokens()
            .cmp(&a.1.total_tokens())
            .then_with(|| a.0.cmp(&b.0))
    });
    out.truncate(limit);
    Ok(out)
}
```

`TurnUsage` must derive `Default` and `Clone` for `unwrap_or_default()`/`cloned()` — it already derives both; confirm rather than assume.

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core query` → PASS.
Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/query.rs
git commit -m "feat(core): session history, daily usage, and top-project queries"
```

---

## Task 3: `ui::main_window` — the window's view-model

**Files:**
- Create: `crates/perch-core/src/ui/main_window.rs`
- Modify: `crates/perch-core/src/ui/mod.rs`

**Interfaces:**
- Consumes: `query::{project_summaries, session_history, daily_usage}`, `db::{Db, ProjectMeta}`, `ui::format::*`, `ui::model::PopoverModel`, `live::LiveSession`.
- Produces:

```rust
pub enum ProjectGroup { Pinned, Active, Recent, Archived }
pub struct ProjectRow { pub id: i64, pub name: String, pub path: String, pub group: ProjectGroup,
                        pub session_count: String, pub tokens: String, pub cost: String,
                        pub last_active: String, pub live_session_count: u32, pub subtitle: String }
pub struct SparkPoint { pub day_index: i32, pub tokens: u64, pub label: String }
pub struct SessionHistoryRow { pub id: String, pub name: String, pub started: String,
                               pub duration: String, pub tokens: String, pub cost: String,
                               pub branch: Option<String>, pub is_live: bool, pub detail_line: String }
pub struct ProjectDetail { pub id: i64, pub name: String, pub path: String, pub note: String,
                           pub tokens: String, pub cost: String, pub session_count: String,
                           pub sparkline: Vec<SparkPoint>, pub sessions: Vec<SessionHistoryRow>,
                           pub pinned: bool, pub archived: bool, pub path_exists: bool }
pub struct MainWindowModel { pub now: PopoverModel, pub projects: Vec<ProjectRow>, pub error: Option<String> }

pub fn build_main_window(db: Option<&Db>, now: PopoverModel, live: &[LiveSession], now_ms: i64) -> MainWindowModel
pub fn build_project_detail(db: &Db, project_id: i64, live: &[LiveSession], now_ms: i64) -> anyhow::Result<ProjectDetail>
```

**Grouping rules** (the ordering the sidebar renders): `Archived` when the meta says so — archived wins over everything, including pinned. Otherwise `Pinned` when pinned. Otherwise `Active` when `last_activity_at` is within 7 days of `now_ms`. Otherwise `Recent`. Within a group, order by `last_activity_at` descending, nulls last, then by name.

**`path_exists`** is `std::path::Path::new(&path).is_dir()` — the UI disables the terminal actions and says why when a project's directory is gone (spec §7).

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/ui/main_window.rs` with the header and tests:

```rust
//! The main window's view-model: every project the index knows, and one
//! project's full history. Same rule as the popover — every string is final here.

use crate::db::Db;
use crate::live::LiveSession;
use crate::query;
use crate::ui::format::{human_cost, human_elapsed, human_tokens};
use crate::ui::model::PopoverModel;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

    const DAY: i64 = 86_400_000;

    fn project_with_session(db: &Db, slug: &str, path: &str, sid: &str, last: i64, input: u64) -> i64 {
        let pid = db.upsert_project(slug, path, false).unwrap();
        db.upsert_session(&SessionRecord {
            id: sid.into(), project_id: pid, file_path: format!("/tmp/{sid}.jsonl"), file_size: 0,
            indexed_offset: 0, started_at: Some(last - 60_000), last_activity_at: Some(last),
            cwd: Some(path.into()), git_branch: Some("main".into()), cc_version: None, message_count: 3,
        }).unwrap();
        db.insert_turns(sid, &[Turn { ts: last, model: "claude-fable-5".into(),
            usage: TurnUsage { input, ..Default::default() } }]).unwrap();
        pid
    }

    #[test]
    fn projects_are_grouped_with_archived_winning_over_pinned() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let fresh = project_with_session(&db, "-a-fresh", "/a/fresh", "s1", now - DAY, 1_000);
        let stale = project_with_session(&db, "-a-stale", "/a/stale", "s2", now - 30 * DAY, 1_000);
        let pinned = project_with_session(&db, "-a-pin", "/a/pin", "s3", now - 40 * DAY, 1_000);
        let arch = project_with_session(&db, "-a-arch", "/a/arch", "s4", now - DAY, 1_000);
        db.set_pinned(pinned, true).unwrap();
        db.set_pinned(arch, true).unwrap();
        db.set_archived(arch, true).unwrap();

        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now);
        let group_of = |id: i64| m.projects.iter().find(|p| p.id == id).unwrap().group;
        assert_eq!(group_of(pinned), ProjectGroup::Pinned);
        assert_eq!(group_of(fresh), ProjectGroup::Active, "activity within 7 days");
        assert_eq!(group_of(stale), ProjectGroup::Recent);
        assert_eq!(group_of(arch), ProjectGroup::Archived, "archived beats pinned");
    }

    #[test]
    fn a_project_row_carries_finished_strings() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        project_with_session(&db, "-a-p", "/a/proj", "s1", now - 3_600_000, 2_000_000);

        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now);
        let row = &m.projects[0];
        assert_eq!(row.name, "proj", "directory name, not the slug");
        assert_eq!(row.tokens, "2.0M");
        assert_eq!(row.cost, "$30.00");
        assert_eq!(row.session_count, "1 session", "singular");
        assert_eq!(row.last_active, "1h");
        assert!(row.subtitle.contains("1 session"), "subtitle is composed in Rust");
    }

    #[test]
    fn session_count_is_pluralised_in_rust() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 1000, 10);
        db.upsert_session(&SessionRecord {
            id: "s2".into(), project_id: pid, file_path: "/tmp/s2.jsonl".into(), file_size: 0,
            indexed_offset: 0, started_at: Some(now - 2000), last_activity_at: Some(now - 2000),
            cwd: Some("/a/proj".into()), git_branch: None, cc_version: None, message_count: 1,
        }).unwrap();
        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now);
        assert_eq!(m.projects[0].session_count, "2 sessions");
    }

    #[test]
    fn no_database_yields_an_empty_list_and_no_panic() {
        let m = build_main_window(None, PopoverModel::empty(), &[], 100 * DAY);
        assert!(m.projects.is_empty());
        assert!(m.error.is_some(), "the user is told why the list is empty");
    }

    #[test]
    fn project_detail_has_a_full_fourteen_day_sparkline_even_when_quiet() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 1000, 1_000_000);

        let d = build_project_detail(&db, pid, &[], now).unwrap();
        assert_eq!(d.sparkline.len(), 14);
        assert_eq!(d.sparkline[0].day_index, 0, "indices are 0..13, oldest first");
        assert_eq!(d.sparkline[13].day_index, 13);
        assert!(d.sparkline[13].tokens > 0, "today has the activity");
        assert!(d.sparkline[0].tokens == 0, "thirteen days ago was quiet");
        assert!(!d.sparkline[0].label.is_empty(), "every point carries its own label");
    }

    #[test]
    fn project_detail_lists_sessions_newest_first_and_marks_live_ones() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "old", now - 5 * DAY, 1_000);
        db.upsert_session(&SessionRecord {
            id: "livesess".into(), project_id: pid, file_path: "/tmp/l.jsonl".into(), file_size: 0,
            indexed_offset: 0, started_at: Some(now - 1000), last_activity_at: Some(now - 500),
            cwd: Some("/a/proj".into()), git_branch: None, cc_version: None, message_count: 1,
        }).unwrap();

        let live = vec![LiveSession {
            pid: 1, session_id: "livesess".into(), cwd: "/a/proj".into(), name: "n".into(),
            kind: "interactive".into(), status: crate::live::SessionStatus::Working,
            started_at: now - 1000, status_updated_at: now - 500, cc_version: None, socket_path: None,
        }];
        let d = build_project_detail(&db, pid, &live, now).unwrap();
        assert_eq!(d.sessions[0].id, "livesess");
        assert!(d.sessions[0].is_live);
        assert!(!d.sessions[1].is_live, "the older one has ended");
        assert_eq!(d.sessions[1].branch.as_deref(), Some("main"));
    }

    #[test]
    fn project_detail_reports_a_missing_directory_rather_than_failing() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        project_with_session(&db, "-nope", "/definitely/not/here", "s1", 100 * DAY, 1);
        let pid = db.upsert_project("-nope", "/definitely/not/here", false).unwrap();
        let d = build_project_detail(&db, pid, &[], 100 * DAY).unwrap();
        assert!(!d.path_exists, "a vanished project directory is reported, not fatal");
    }
```

Close the test module after these. `PopoverModel::empty()` does not exist yet — add it in Step 3.

- [ ] **Step 2: Register and run to see them fail**

Add `pub mod main_window;` to `crates/perch-core/src/ui/mod.rs`.
Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::main_window`
Expected: FAIL — `cannot find function build_main_window`.

- [ ] **Step 3: Implement**

First add to `crates/perch-core/src/ui/model.rs` (tests and the window both need a neutral model):

```rust
impl PopoverModel {
    /// A model with nothing in it — the "before the first tick" state, and what
    /// the window shows for `Now` when there is no engine data yet.
    pub fn empty() -> Self {
        PopoverModel {
            stats: dashed_stats(),
            live: Vec::new(),
            recent: Vec::new(),
            tray_title: String::new(),
            error: None,
            waiting_banner: None,
        }
    }
}
```

Then implement `main_window.rs` above its test module:

```rust
const SPARK_DAYS: usize = 14;
const ACTIVE_WINDOW_MS: i64 = 7 * 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ProjectGroup { Pinned, Active, Recent, Archived }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectRow {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub group: ProjectGroup,
    pub session_count: String,
    pub tokens: String,
    pub cost: String,
    pub last_active: String,
    pub live_session_count: u32,
    /// "N sessions · 2.0M · $30.00 · 1h" — composed here so no shell rebuilds it.
    pub subtitle: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SparkPoint { pub day_index: i32, pub tokens: u64, pub label: String }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionHistoryRow {
    pub id: String,
    pub name: String,
    pub started: String,
    pub duration: String,
    pub tokens: String,
    pub cost: String,
    pub branch: Option<String>,
    pub is_live: bool,
    pub detail_line: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectDetail {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub note: String,
    pub tokens: String,
    pub cost: String,
    pub session_count: String,
    pub sparkline: Vec<SparkPoint>,
    pub sessions: Vec<SessionHistoryRow>,
    pub pinned: bool,
    pub archived: bool,
    pub path_exists: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MainWindowModel {
    pub now: PopoverModel,
    pub projects: Vec<ProjectRow>,
    pub error: Option<String>,
}

fn dir_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn plural(n: i64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn since(now_ms: i64, ts: Option<i64>) -> String {
    match ts {
        Some(t) if t > 0 => human_elapsed(now_ms - t),
        _ => "—".to_string(),
    }
}

/// Every project the index knows, grouped for the sidebar. `db: None` (or a
/// failing read) yields an empty list with the reason attached — never a silently
/// empty window.
pub fn build_main_window(
    db: Option<&Db>,
    now: PopoverModel,
    live: &[LiveSession],
    now_ms: i64,
) -> MainWindowModel {
    let Some(db) = db else {
        return MainWindowModel { now, projects: Vec::new(), error: Some("index unavailable".into()) };
    };
    let summaries = match query::project_summaries(db) {
        Ok(s) => s,
        Err(e) => {
            return MainWindowModel { now, projects: Vec::new(), error: Some(format!("index unavailable: {e}")) }
        }
    };

    let mut projects: Vec<ProjectRow> = summaries
        .into_iter()
        .map(|s| {
            let meta = db.project_meta(s.id).ok();
            let archived = meta.as_ref().is_some_and(|m| m.archived);
            let pinned = meta.as_ref().is_some_and(|m| m.pinned);
            let group = if archived {
                ProjectGroup::Archived
            } else if pinned {
                ProjectGroup::Pinned
            } else if s.last_activity_at.is_some_and(|t| now_ms - t <= ACTIVE_WINDOW_MS) {
                ProjectGroup::Active
            } else {
                ProjectGroup::Recent
            };
            let name = meta
                .as_ref()
                .and_then(|m| m.display_name.clone())
                .unwrap_or_else(|| dir_name(&s.real_path));
            let session_count = plural(s.sessions, "session", "sessions");
            let tokens = human_tokens(s.usage.total_tokens());
            let cost = human_cost(s.cost_usd);
            let last_active = since(now_ms, s.last_activity_at);
            let live_session_count = live.iter().filter(|l| l.cwd == s.real_path).count() as u32;
            ProjectRow {
                id: s.id,
                name,
                path: s.real_path.clone(),
                group,
                subtitle: format!("{session_count} · {tokens} · {cost} · {last_active}"),
                session_count,
                tokens,
                cost,
                last_active,
                live_session_count,
            }
        })
        .collect();

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    MainWindowModel { now, projects, error: None }
}

/// One project in full: its note, totals, a fourteen-day sparkline, and every
/// session ever recorded for it.
pub fn build_project_detail(
    db: &Db,
    project_id: i64,
    live: &[LiveSession],
    now_ms: i64,
) -> anyhow::Result<ProjectDetail> {
    let meta = db.project_meta(project_id)?;
    let summary = query::project_summaries(db)?
        .into_iter()
        .find(|s| s.id == project_id)
        .ok_or_else(|| anyhow::anyhow!("no project with id {project_id}"))?;

    let name = meta.display_name.clone().unwrap_or_else(|| dir_name(&summary.real_path));

    let days = query::daily_usage(db, SPARK_DAYS, now_ms)?;
    let sparkline = days
        .iter()
        .enumerate()
        .map(|(i, d)| SparkPoint {
            day_index: i as i32,
            tokens: d.usage.total_tokens(),
            label: format!("{} ago", human_elapsed(now_ms - d.day_start_ms)),
        })
        .collect();

    let sessions = query::session_history(db, project_id)?
        .into_iter()
        .map(|h| {
            let is_live = live.iter().any(|l| l.session_id == h.id);
            let tokens = human_tokens(h.usage.total_tokens());
            let cost = human_cost(h.cost_usd);
            let started = since(now_ms, h.started_at);
            let duration = match (h.started_at, h.last_activity_at) {
                (Some(a), Some(b)) if b >= a => human_elapsed(b - a),
                _ => "—".to_string(),
            };
            let mut parts = vec![format!("{tokens} · {cost}"), format!("{duration} long")];
            if let Some(b) = &h.branch {
                parts.push(b.clone());
            }
            SessionHistoryRow {
                name: h.id.chars().take(8).collect(),
                detail_line: parts.join(" · "),
                id: h.id,
                started,
                duration,
                tokens,
                cost,
                branch: h.branch,
                is_live,
            }
        })
        .collect();

    Ok(ProjectDetail {
        id: project_id,
        path_exists: std::path::Path::new(&summary.real_path).is_dir(),
        path: summary.real_path,
        name,
        note: meta.note.unwrap_or_default(),
        tokens: human_tokens(summary.usage.total_tokens()),
        cost: human_cost(summary.cost_usd),
        session_count: plural(summary.sessions, "session", "sessions"),
        sparkline,
        sessions,
        pinned: meta.pinned,
        archived: meta.archived,
    })
}
```

`dashed_stats()` is currently private in `model.rs` — make it `pub(crate)` so `empty()` can use it, or inline the same values.

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::main_window` → PASS.
Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/ui
git commit -m "feat(core): main-window view-model — grouped projects and project detail"
```

---

## Task 4: `ui::usage` — the usage view's view-model

**Files:**
- Create: `crates/perch-core/src/ui/usage.rs`
- Modify: `crates/perch-core/src/ui/mod.rs`

**Interfaces:**
- Consumes: `query::{daily_usage, top_projects, usage_by_model, usage_since, DAY_MS}`, `ui::format::*`.
- Produces:

```rust
pub struct HeroStat { pub label: String, pub value: String, pub caption: Option<String> }
pub struct DailyBar { pub day_index: i32, pub label: String,
                      pub input: u64, pub output: u64, pub cache_read: u64,
                      pub cache_write: u64, pub thinking: u64, pub total_label: String }
pub struct RankedProject { pub name: String, pub tokens: u64, pub tokens_label: String, pub cost: String }
pub struct ModelUsage { pub model: String, pub tokens: String, pub cost: String }
pub struct UsageModel { pub hero: Vec<HeroStat>, pub daily: Vec<DailyBar>,
                        pub top_projects: Vec<RankedProject>, pub by_model: Vec<ModelUsage>,
                        pub burn_rate: Option<String>, pub has_data: bool }
pub fn build_usage(db: &Db, now_ms: i64) -> anyhow::Result<UsageModel>
```

**Burn rate is honest or absent.** It is `Some` only when the current 5-hour window has at least 30 minutes elapsed *and* non-zero usage; then it projects tokens-per-hour and renders "≈N/h at this pace". It is never a percentage of a ceiling Perch does not know (original spec §8).

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/ui/usage.rs`:

```rust
//! The usage view's view-model. Chart values cross as numbers; every label is
//! finished here.

use crate::db::Db;
use crate::query::{self, DAY_MS};
use crate::ui::format::{human_cost, human_tokens};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

    fn seed(db: &Db, sid: &str, ts: i64, model: &str, u: TurnUsage) {
        let pid = db.upsert_project("-a-p", "/a/proj", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: sid.into(), project_id: pid, file_path: format!("/tmp/{sid}.jsonl"), file_size: 0,
            indexed_offset: 0, started_at: Some(ts), last_activity_at: Some(ts),
            cwd: Some("/a/proj".into()), git_branch: None, cc_version: None, message_count: 1,
        }).unwrap();
        db.insert_turns(sid, &[Turn { ts, model: model.into(), usage: u }]).unwrap();
    }

    #[test]
    fn an_empty_index_reports_no_data_rather_than_zeroes() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let m = build_usage(&db, 100 * DAY_MS).unwrap();
        assert!(!m.has_data);
        assert!(m.burn_rate.is_none(), "no projection without data");
        assert!(m.top_projects.is_empty());
        assert_eq!(m.daily.len(), 14, "the window is still drawn, just empty");
    }

    #[test]
    fn daily_bars_carry_each_token_class_separately() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY_MS + 3_600_000;
        seed(&db, "s1", 100 * DAY_MS + 1, "claude-fable-5", TurnUsage {
            input: 10, output: 20, cache_read: 30, cache_write_5m: 40, cache_write_1h: 5, thinking: 6,
        });

        let m = build_usage(&db, now).unwrap();
        let today = m.daily.last().unwrap();
        assert_eq!(today.input, 10);
        assert_eq!(today.output, 20);
        assert_eq!(today.cache_read, 30);
        assert_eq!(today.cache_write, 45, "5m and 1h cache writes are one visual class");
        assert_eq!(today.thinking, 6);
        assert_eq!(today.total_label, human_tokens(111));
        assert!(m.has_data);
    }

    #[test]
    fn top_projects_and_by_model_are_ranked_and_labelled() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY_MS;
        seed(&db, "s1", now - 1000, "claude-fable-5", TurnUsage { input: 2_000_000, ..Default::default() });
        seed(&db, "s2", now - 2000, "claude-sonnet-5", TurnUsage { input: 1_000_000, ..Default::default() });

        let m = build_usage(&db, now).unwrap();
        assert_eq!(m.top_projects[0].name, "proj");
        assert_eq!(m.top_projects[0].tokens, 3_000_000, "chart value is a number");
        assert_eq!(m.top_projects[0].tokens_label, "3.0M", "its label is a finished string");

        let models: Vec<&str> = m.by_model.iter().map(|x| x.model.as_str()).collect();
        assert_eq!(models[0], "claude-fable-5", "ranked by tokens, biggest first");
        assert_eq!(m.by_model[0].cost, "$30.00");
    }

    #[test]
    fn burn_rate_is_absent_when_the_window_is_too_young_to_project() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        // Usage exists, but only 5 minutes into the 5-hour window.
        let now = 100 * DAY_MS + 300_000;
        seed(&db, "s1", now - 60_000, "claude-fable-5", TurnUsage { input: 1_000, ..Default::default() });
        let m = build_usage(&db, now).unwrap();
        assert!(m.burn_rate.is_none(), "five minutes is not enough to project from");
    }

    #[test]
    fn burn_rate_appears_once_the_window_has_enough_history() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY_MS + 2 * 3_600_000; // two hours into the window
        seed(&db, "s1", now - 3_600_000, "claude-fable-5", TurnUsage { input: 2_000_000, ..Default::default() });
        let m = build_usage(&db, now).unwrap();
        let rate = m.burn_rate.expect("two hours of history is projectable");
        assert!(rate.contains("/h"), "reads as a rate: {rate}");
        assert!(!rate.contains('%'), "never a percentage of a ceiling Perch does not know");
    }
}
```

- [ ] **Step 2: Register and run to see them fail**

Add `pub mod usage;` to `crates/perch-core/src/ui/mod.rs`.
Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::usage`
Expected: FAIL — `cannot find function build_usage`.

- [ ] **Step 3: Implement**

```rust
const CHART_DAYS: usize = 14;
const TOP_N: usize = 8;
const WINDOW_MS: i64 = 5 * 3_600_000;
const MIN_PROJECTABLE_MS: i64 = 30 * 60_000;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HeroStat { pub label: String, pub value: String, pub caption: Option<String> }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DailyBar {
    pub day_index: i32,
    pub label: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub thinking: u64,
    pub total_label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RankedProject { pub name: String, pub tokens: u64, pub tokens_label: String, pub cost: String }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ModelUsage { pub model: String, pub tokens: String, pub cost: String }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageModel {
    pub hero: Vec<HeroStat>,
    pub daily: Vec<DailyBar>,
    pub top_projects: Vec<RankedProject>,
    pub by_model: Vec<ModelUsage>,
    pub burn_rate: Option<String>,
    pub has_data: bool,
}

/// Projected tokens per hour for the current 5-hour window, or `None` when the
/// window is too young or too quiet to project from honestly. Deliberately not a
/// percentage: the ceiling needs the tier-1 endpoint, and the original spec §8
/// forbids inventing one.
fn burn_rate(window_tokens: u64, elapsed_ms: i64) -> Option<String> {
    if window_tokens == 0 || elapsed_ms < MIN_PROJECTABLE_MS {
        return None;
    }
    let hours = elapsed_ms as f64 / 3_600_000.0;
    let per_hour = (window_tokens as f64 / hours).round() as u64;
    Some(format!("≈{}/h at this pace", human_tokens(per_hour)))
}

pub fn build_usage(db: &Db, now_ms: i64) -> anyhow::Result<UsageModel> {
    let (window, window_cost) = query::usage_since(db, now_ms - WINDOW_MS)?;
    let (day, day_cost) = query::usage_since(db, now_ms - DAY_MS)?;
    let (week, _) = query::usage_since(db, now_ms - 7 * DAY_MS)?;

    let days = query::daily_usage(db, CHART_DAYS, now_ms)?;
    let daily: Vec<DailyBar> = days
        .iter()
        .enumerate()
        .map(|(i, d)| DailyBar {
            day_index: i as i32,
            label: format!("{}d", (CHART_DAYS - 1 - i)),
            input: d.usage.input,
            output: d.usage.output,
            cache_read: d.usage.cache_read,
            cache_write: d.usage.cache_write_total(),
            thinking: d.usage.thinking,
            total_label: human_tokens(d.usage.total_tokens()),
        })
        .collect();

    let top_projects = query::top_projects(db, now_ms - 30 * DAY_MS, TOP_N)?
        .into_iter()
        .map(|(name, u, cost)| RankedProject {
            name,
            tokens: u.total_tokens(),
            tokens_label: human_tokens(u.total_tokens()),
            cost: human_cost(cost),
        })
        .collect();

    let mut by_model_rows = query::usage_by_model(db)?;
    by_model_rows.sort_by(|a, b| b.1.total_tokens().cmp(&a.1.total_tokens()));
    let by_model = by_model_rows
        .into_iter()
        .map(|(model, u, cost)| ModelUsage {
            model,
            tokens: human_tokens(u.total_tokens()),
            cost: human_cost(cost),
        })
        .collect();

    // The window's elapsed time is measured from its start, not from first use:
    // a window that has been open two hours has been burning for two hours.
    let elapsed_in_window = now_ms.rem_euclid(WINDOW_MS);

    let hero = vec![
        HeroStat { label: "5-hour window".into(), value: human_tokens(window.total_tokens()),
                   caption: Some(human_cost(window_cost)) },
        HeroStat { label: "Week".into(), value: human_tokens(week.total_tokens()), caption: None },
        HeroStat { label: "24 hours".into(), value: human_tokens(day.total_tokens()),
                   caption: Some(human_cost(day_cost)) },
    ];

    Ok(UsageModel {
        has_data: db.turn_count()? > 0,
        burn_rate: burn_rate(window.total_tokens(), elapsed_in_window),
        hero,
        daily,
        top_projects,
        by_model,
    })
}
```

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::usage` → PASS.
Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/ui
git commit -m "feat(core): usage view-model with honest burn-rate projection"
```

---

## Task 5: `actions` — composing terminal commands

**Files:**
- Create: `crates/perch-core/src/actions.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Produces:

```rust
pub struct TerminalCommand { pub program: String, pub args: Vec<String>, pub cwd: String }
impl TerminalCommand {
    pub fn resume(session_id: &str, cwd: &str) -> Self;   // claude --resume <id>
    pub fn open(cwd: &str) -> Self;                        // claude
    pub fn shell_line(&self) -> String;                    // cd '<cwd>' && claude ...
}
```

**This module never spawns a process and never writes a file.** It composes; the shell runs. That keeps `perch-core` free of filesystem writes (which CI enforces) and makes the logic reusable by a Linux shell.

Quoting uses POSIX single quotes with the `'\''` escape — the only form that is safe for every character a path can contain.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/actions.rs`:

```rust
//! What to run in the user's terminal. Composed here, spawned by the shell —
//! keeping perch-core free of process spawning and filesystem writes, and
//! letting every platform reuse the same command.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_builds_the_documented_command() {
        let c = TerminalCommand::resume("abc-123", "/Users/a/proj");
        assert_eq!(c.program, "claude");
        assert_eq!(c.args, vec!["--resume", "abc-123"]);
        assert_eq!(c.cwd, "/Users/a/proj");
        assert_eq!(c.shell_line(), "cd '/Users/a/proj' && claude --resume 'abc-123'");
    }

    #[test]
    fn open_starts_a_fresh_session_with_no_arguments() {
        let c = TerminalCommand::open("/Users/a/proj");
        assert!(c.args.is_empty());
        assert_eq!(c.shell_line(), "cd '/Users/a/proj' && claude");
    }

    #[test]
    fn paths_with_spaces_survive_quoting() {
        let c = TerminalCommand::open("/Users/a/My Projects/thing");
        assert_eq!(c.shell_line(), "cd '/Users/a/My Projects/thing' && claude");
    }

    #[test]
    fn a_single_quote_in_a_path_cannot_break_out_of_the_quoting() {
        let c = TerminalCommand::open("/Users/a/it's mine");
        // POSIX: close, escaped literal quote, reopen.
        assert_eq!(c.shell_line(), r#"cd '/Users/a/it'\''s mine' && claude"#);
        assert!(!c.shell_line().contains("; "), "no statement separator can be injected");
    }

    #[test]
    fn a_session_id_is_quoted_too() {
        let c = TerminalCommand::resume("a'; rm -rf /", "/tmp");
        let line = c.shell_line();
        assert!(line.contains(r#"'a'\''; rm -rf /'"#), "the injection is inert: {line}");
    }
}
```

- [ ] **Step 2: Register and run to see them fail**

Add `pub mod actions;` to `crates/perch-core/src/lib.rs`.
Run: `source "$HOME/.cargo/env" && cargo test -p perch-core actions`
Expected: FAIL — `cannot find type TerminalCommand`.

- [ ] **Step 3: Implement**

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TerminalCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}

/// POSIX single-quoting: wrap in single quotes and replace each embedded quote
/// with `'\''`. This is the only escaping that is safe for every byte a path can
/// hold, which matters because these strings come from the user's filesystem.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl TerminalCommand {
    pub fn resume(session_id: &str, cwd: &str) -> Self {
        TerminalCommand {
            program: "claude".into(),
            args: vec!["--resume".into(), session_id.into()],
            cwd: cwd.into(),
        }
    }

    pub fn open(cwd: &str) -> Self {
        TerminalCommand { program: "claude".into(), args: Vec::new(), cwd: cwd.into() }
    }

    /// A single shell line the caller can hand to a terminal. `--resume` stays
    /// unquoted as a literal flag; every value is quoted.
    pub fn shell_line(&self) -> String {
        let mut line = format!("cd {} && {}", sq(&self.cwd), self.program);
        for a in &self.args {
            if a.starts_with("--") {
                line.push(' ');
                line.push_str(a);
            } else {
                line.push(' ');
                line.push_str(&sq(a));
            }
        }
        line
    }
}
```

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core actions` → PASS.
Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

Confirm the CI write-audit still passes over `perch-core` with the new file present (it scans for `fs::write|fs::remove_file|fs::remove_dir|File::create|OpenOptions`; `actions.rs` has none).

```bash
git add crates/perch-core/src/actions.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): compose terminal commands for resume and open"
```

---

## Task 6: FFI surface for the window

**Files:**
- Modify: `crates/perch-ffi/src/lib.rs`

**Interfaces:**
- Produces, on the existing `Perch` object:

```
MainWindowModel main_window()
ProjectDetail   project_detail(i64 project_id)      throws PerchError
UsageModel      usage()                              throws PerchError
ProjectDetail   set_note(i64 project_id, String note)      throws PerchError
ProjectDetail   set_pinned(i64 project_id, bool pinned)    throws PerchError
ProjectDetail   set_archived(i64 project_id, bool archived) throws PerchError
ProjectDetail   rename_project(i64 project_id, String name) throws PerchError
TerminalCommand resume_command(String session_id, String cwd)
TerminalCommand open_command(String cwd)
```

Every edit returns the **refreshed `ProjectDetail`**, so the shell never has to re-fetch or guess what changed. Errors are typed (`PerchError`), never swallowed — the seam the last milestone's final review found broken.

- [ ] **Step 1: Write the failing tests**

Append to `crates/perch-ffi/src/lib.rs`'s test module (reuse the existing `DataDirGuard` and `ENV_LOCK`):

```rust
    #[test]
    fn main_window_and_usage_are_reachable_against_an_empty_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let w = perch.main_window();
        assert!(w.projects.is_empty(), "an empty config dir has no projects");
        let u = perch.usage().unwrap();
        assert!(!u.has_data);
        assert_eq!(u.daily.len(), 14, "the chart window is drawn even when empty");
    }

    #[test]
    fn editing_an_unknown_project_is_a_typed_error_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        assert!(perch.set_pinned(4242, true).is_err());
        assert!(perch.project_detail(4242).is_err());
    }

    #[test]
    fn commands_cross_the_boundary_with_their_shell_line_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let c = perch.resume_command("abc".into(), "/a/b".into());
        assert_eq!(c.shell_line, "cd '/a/b' && claude --resume 'abc'");
        let o = perch.open_command("/a/b".into());
        assert_eq!(o.shell_line, "cd '/a/b' && claude");
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-ffi`
Expected: FAIL — `no method named main_window`.

- [ ] **Step 3: Implement**

Mirror each new core record as a `#[derive(uniffi::Record)]` with a `From` impl that **destructures** the core value (so a new core field becomes a compile error — the coupling the last review asked for). `ProjectGroup` becomes a `#[derive(uniffi::Enum)]`.

`TerminalCommand`'s FFI mirror carries `program`, `args`, `cwd`, **and** a precomputed `shell_line: String`, so the shell never rebuilds it:

```rust
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TerminalCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub shell_line: String,
}

impl From<perch_core::actions::TerminalCommand> for TerminalCommand {
    fn from(c: perch_core::actions::TerminalCommand) -> Self {
        let shell_line = c.shell_line();
        let perch_core::actions::TerminalCommand { program, args, cwd } = c;
        TerminalCommand { program, args, cwd, shell_line }
    }
}
```

Add the methods to the exported `impl Perch`. Each one opens the database, does its work, and maps failure to `PerchError::Database { message }`:

```rust
    /// The whole window: `Now` plus every project the index knows.
    pub fn main_window(&self) -> MainWindowModel {
        let sessions = live::live_sessions(&config::sessions_dir(&self.config_dir), &RealProcessProbe);
        let now = self.model_for(sessions.clone());
        let db = db::open(&self.db_path).ok();
        main_window::build_main_window(db.as_ref(), now.into_core(), &sessions, now_ms()).into()
    }
```

If `PopoverModel` cannot round-trip back into its core form cleanly, build the core popover model directly instead (`core_model::build_model(db.as_ref(), &sessions, now_ms())`) rather than adding a conversion that exists only for this call — say which you chose in your report.

Edit methods follow one shape:

```rust
    pub fn set_pinned(&self, project_id: i64, pinned: bool) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        database.set_pinned(project_id, pinned).map_err(db_err)?;
        self.detail(&database, project_id)
    }
```

with two small private helpers — `open_db()` returning `Result<Db, PerchError>` and `detail(&Db, i64)` building the refreshed `ProjectDetail` — so the seven edit/read methods do not repeat the same four lines seven times.

- [ ] **Step 4: Verify, regenerate, commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-ffi` → PASS.
Run: `cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Run: `scripts/build-xcframework.sh` (regenerates the bindings; idempotent).
Run: `cargo build -p perch-app` (the Tauri app still builds).

**Record the exact generated Swift names** for every new type and method in your report — the Swift tasks are written against them, and UniFFI camelCases (`mainWindow`, `projectDetail`, `setPinned`, `shellLine`, `liveSessionCount`) while error enum cases stay PascalCase.

Confirm `git status` shows no generated artefact staged.

```bash
git add crates/perch-ffi/src/lib.rs
git commit -m "feat(ffi): main-window, usage, project edits, and terminal commands"
```

---

## Task 7: The window itself — shell, activation policy, and the menu item

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/MainWindow/MainWindowController.swift`
- Modify: `apps/macos/Perch/Sources/Perch/StatusItemController.swift`, `apps/macos/Perch/Sources/Perch/Bridge/PerchEngine.swift`, `apps/macos/Perch/Sources/Perch/PerchApp.swift`

**Interfaces:**
- Consumes: the generated `Perch.mainWindow()`, plus `PerchEngine`.
- Produces: `MainWindowController` (`@MainActor final class`) with `func show()`; `PerchEngine` gains `func mainWindow() async -> MainWindowModel` and the other reads, each hopping to a background queue and back.

**Activation policy:** the app runs `.accessory` (no Dock icon) as a menu-bar app. While the window is open it must be `.regular` so the window can take focus, appear in the Dock, and be reachable with Cmd-Tab; on close it returns to `.accessory`. Use `NSWindowDelegate.windowWillClose` for the return trip.

- [ ] **Step 1: Reads off the main thread**

In `PerchEngine.swift`, add methods that do the FFI call on a background queue and return on the main actor. The FFI object is `Sendable`-safe to call from any thread (it takes its own locks); the *result* must reach the UI on the main actor:

```swift
    /// Reads run off the main thread: a large index makes these slow, and this
    /// is called while AppKit is preparing to show a window.
    func mainWindow() async -> MainWindowModel? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) { perch.mainWindow() }.value
    }

    func projectDetail(_ id: Int64) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.projectDetail(projectId: id) }
        }.value
    }

    func usage() async -> Result<UsageModel, Error> { /* same shape */ }
```

Add `struct EngineUnavailable: LocalizedError { var errorDescription: String? { "Perch's engine is not running." } }`.

- [ ] **Step 2: The window controller**

```swift
import AppKit
import SwiftUI

/// Hosts the main window. The app is a menu-bar accessory, so it has no Dock
/// icon — but a window needs one to take focus and appear in Cmd-Tab, so the
/// activation policy flips to `.regular` while the window is up and back on close.
@MainActor
final class MainWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private let engine: PerchEngine

    init(engine: PerchEngine) {
        self.engine = engine
        super.init()
    }

    func show() {
        if let window {
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            return
        }

        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1000, height: 660),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        w.title = "Perch"
        w.titlebarAppearsTransparent = false
        w.minSize = NSSize(width: 820, height: 520)
        w.center()
        w.isReleasedWhenClosed = false
        w.delegate = self
        w.contentView = NSHostingView(rootView: MainWindowRoot(engine: engine))
        window = w

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // Back to a pure menu-bar app: no Dock icon, no Cmd-Tab entry.
        NSApp.setActivationPolicy(.accessory)
    }
}
```

`MainWindowRoot` is a placeholder for this task — a `VStack` showing `model.projects.count` and any `model.error` — replaced in Task 8. Keep it in `MainWindowController.swift` for now and move it to `Sidebar.swift` in Task 8.

- [ ] **Step 3: Wire the menu item and the app delegate**

In `StatusItemController.swift`, add an **Open Perch** item above the Quit separator, with `keyEquivalent: "o"`, targeting a closure that calls the controller's `show()`. Give `StatusItemController` an `var onOpenWindow: (() -> Void)?` set by `AppDelegate`, rather than having it own the window controller — the status item's job is the menu, not window lifetime.

In `PerchApp.swift`, hold a `MainWindowController` alongside the engine and status item, and wire `status.onOpenWindow = { [weak self] in self?.mainWindow?.show() }`.

- [ ] **Step 4: Build, run, verify**

Run: `cd apps/macos/Perch && swift build -c release 2>&1 | tail -3`
Run: `make bundle && open build/Perch.app`

Verify and report: the menu has **Open Perch**; choosing it opens a titled, resizable window; a Dock icon appears while it is open and disappears when it closes; the status item survives both; Quit still works. Report anything you could not check.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Perch/Sources/Perch
git commit -m "feat(macos): main window shell with activation-policy switching"
```

---

## Task 8: The sidebar — every project, grouped

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/MainWindow/Sidebar.swift`
- Modify: `apps/macos/Perch/Sources/Perch/MainWindow/MainWindowController.swift`

**Interfaces:**
- Consumes: `MainWindowModel`, `ProjectRow`, `ProjectGroup`, `PerchEngine.mainWindow()`.
- Produces: `MainWindowRoot` (the real one, replacing Task 7's placeholder) — a `NavigationSplitView` whose sidebar lists **Now** followed by the four project groups, and whose detail pane shows the selection.

Sidebar sections in order: **Now**, then Pinned, Active, Recent, Archived — each section omitted entirely when it has no rows, so an empty section never shows as a bare header. Each project row shows its name, its `subtitle` (already composed in Rust), and a green dot with the count when `liveSessionCount > 0`.

**Every string comes from the model.** The section titles ("Pinned", "Active", …) are static UI chrome and may live in Swift; anything derived from data may not.

- [ ] **Step 1: Implement the sidebar**

```swift
import SwiftUI
import PerchFFI

struct MainWindowRoot: View {
    let engine: PerchEngine

    @State private var model: MainWindowModel?
    @State private var selection: Selection? = .now

    enum Selection: Hashable { case now, project(Int64) }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Label("Now", systemImage: "dot.radiowaves.left.and.right").tag(Selection.now)
                if let model {
                    section("Pinned", .pinned, model)
                    section("Active", .active, model)
                    section("Recent", .recent, model)
                    section("Archived", .archived, model)
                }
            }
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 360)
        } detail: {
            detailPane
        }
        .task { await reload() }
        .frame(minWidth: 820, minHeight: 520)
    }

    @ViewBuilder
    private func section(_ title: String, _ group: ProjectGroup, _ model: MainWindowModel) -> some View {
        let rows = model.projects.filter { $0.group == group }
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows, id: \.id) { row in
                    ProjectRowView(row: row).tag(Selection.project(row.id))
                }
            }
        }
    }

    @ViewBuilder
    private var detailPane: some View {
        switch selection {
        case .now, .none:
            NowPane(model: model)
        case .project(let id):
            ProjectDetailPane(engine: engine, projectId: id, onChanged: { await reload() })
        }
    }

    private func reload() async {
        model = await engine.mainWindow()
    }
}

struct ProjectRowView: View {
    let row: ProjectRow
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(row.name).fontWeight(.medium).lineLimit(1)
                if row.liveSessionCount > 0 {
                    Circle().fill(Color.perchWorking).frame(width: 7, height: 7)
                }
                Spacer()
            }
            Text(row.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(.vertical, 2)
    }
}

/// The `Now` pane reuses the popover's live model rather than inventing a second
/// one — same data, more room.
struct NowPane: View {
    let model: MainWindowModel?
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let err = model?.error {
                    Text(err).foregroundStyle(.secondary)
                }
                if let banner = model?.now.waitingBanner {
                    Text(banner).font(.headline).foregroundStyle(Color.perchWaiting)
                }
                if let live = model?.now.live, !live.isEmpty {
                    ForEach(live, id: \.id) { SessionRowView(row: $0) }
                } else {
                    Text("No sessions running").foregroundStyle(.secondary)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
```

`ProjectDetailPane` arrives in Task 9 — for this task, stub it as a `Text(verbatim:)` showing the id, and replace it there. Note the stub in your report so the reviewer knows it is deliberate.

`Color.perchWorking` and `Color.perchWaiting` already exist in `Cards/PerchStyle.swift`; reuse them rather than defining new literals.

- [ ] **Step 2: Remove Task 7's placeholder**

Delete the placeholder `MainWindowRoot` from `MainWindowController.swift`; it now lives in `Sidebar.swift`.

- [ ] **Step 3: Build, run, verify**

Run: `cd apps/macos/Perch && swift build -c release 2>&1 | tail -3`
Run: `make bundle && open build/Perch.app`

Verify and report: the sidebar lists **Now** plus your real projects grouped as Pinned/Active/Recent/Archived; empty groups are absent, not empty headers; each row shows a subtitle; a project with a live session shows a green dot; selecting **Now** shows the live sessions. Report what you could not check.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Perch/Sources/Perch
git commit -m "feat(macos): sidebar listing every project, grouped"
```

---

## Task 9: Project detail and the terminal actions

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/MainWindow/ProjectDetail.swift`, `apps/macos/Perch/Sources/Perch/MainWindow/Launcher.swift`
- Modify: `apps/macos/Perch/Sources/Perch/MainWindow/Sidebar.swift`

**Interfaces:**
- Consumes: `ProjectDetail`, `SessionHistoryRow`, `SparkPoint`, `TerminalCommand`, and `PerchEngine.{projectDetail, setNote, setPinned, setArchived, renameProject, resumeCommand, openCommand}`.
- Produces: `ProjectDetailPane` (replacing Task 8's stub) and `Launcher.run(_ command: TerminalCommand) throws`.

**The launcher, and why it is shaped this way:** running a command in the user's terminal without an Automation permission prompt is done by writing a single-use script into Perch's own application-support directory and calling `open -a <TerminalApp> <script>`. AppleScript would need `NSAppleEventsUsageDescription` and a permission dialog; this needs neither. The script goes in Perch's data directory — **never** in the Claude Code directory.

```swift
import AppKit
import PerchFFI

enum LauncherError: LocalizedError {
    case noTerminal
    case writeFailed(String)
    var errorDescription: String? {
        switch self {
        case .noTerminal: "No terminal application could be found to run the command."
        case .writeFailed(let m): "Could not prepare the command: \(m)"
        }
    }
}

/// Runs a Rust-composed command in the user's terminal.
enum Launcher {
    static func run(_ command: TerminalCommand, terminal: String = "Terminal") throws {
        let dir = try scriptDirectory()
        let script = dir.appendingPathComponent("run-\(UUID().uuidString).command")
        let body = "#!/bin/sh\n\(command.shellLine)\n"
        do {
            try body.write(to: script, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
        } catch {
            throw LauncherError.writeFailed(error.localizedDescription)
        }

        let config = NSWorkspace.OpenConfiguration()
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId(for: terminal))
            ?? NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Terminal")
        else { throw LauncherError.noTerminal }
        NSWorkspace.shared.open([script], withApplicationAt: app, configuration: config)
    }

    private static func bundleId(for name: String) -> String {
        name == "iTerm2" ? "com.googlecode.iterm2" : "com.apple.Terminal"
    }

    /// Perch's own directory — the read-only promise about ~/.claude is unaffected.
    private static func scriptDirectory() throws -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Perch/commands", isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base
    }
}
```

- [ ] **Step 1: The detail pane**

`ProjectDetailPane` shows, top to bottom: the project name (editable via a rename affordance), its path, pin and archive toggles; the note in a `TextEditor` that saves on blur; the three totals; the 14-day sparkline as a `Chart` of `BarMark(x: day_index, y: tokens)`; then the session list. Each session row shows `name`, `started`, `detailLine`, a live indicator when `isLive`, and a **Resume** button. The header carries an **Open new session** button. Both are disabled with an explanatory label when `pathExists` is false.

Every action funnels through one method so failures surface consistently:

```swift
    private func launch(_ make: () async -> TerminalCommand?) async {
        guard let command = await make() else { return }
        do { try Launcher.run(command) } catch { self.actionError = error.localizedDescription }
    }
```

`actionError` renders as an inline banner in the pane — never a silent failure, per spec §7.

Edits call the engine and adopt the returned `ProjectDetail` directly (every edit method returns the refreshed model), so the pane never re-fetches or guesses.

- [ ] **Step 2: Replace the stub**

In `Sidebar.swift`, swap the Task 8 stub for the real `ProjectDetailPane`.

- [ ] **Step 3: Build, run, verify**

Run: `cd apps/macos/Perch && swift build -c release 2>&1 | tail -3`
Run: `make bundle && open build/Perch.app`

Verify and report, being precise about what you actually observed:
- selecting a project shows its note, totals, sparkline, and full session history
- editing the note persists across reselecting the project
- pin and archive move the project between sidebar groups
- **Resume** on an ended session opens your terminal and runs `claude --resume <id>` in that directory
- **Open new session** opens a terminal running `claude` there
- a project whose directory is missing shows disabled actions with a reason

**Do not claim the terminal actions work unless you saw a terminal open.** If you cannot verify them in your environment, say so plainly and leave them for the user.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Perch/Sources/Perch
git commit -m "feat(macos): project detail with note, history, and terminal actions"
```

---

## Task 10: The usage view

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/MainWindow/UsageView.swift`
- Modify: `apps/macos/Perch/Sources/Perch/MainWindow/Sidebar.swift`

**Interfaces:**
- Consumes: `UsageModel`, `HeroStat`, `DailyBar`, `RankedProject`, `ModelUsage`, `PerchEngine.usage()`.
- Produces: `UsageView`, reached from a tab or segmented control in the detail pane alongside Overview.

Four blocks, per spec §9.3: hero stats with the burn-rate line (omitted entirely when `burnRate` is nil); daily stacked bars by token class; top projects; by model.

**Chart rules from the spec, which are requirements not suggestions:** categorical hues in fixed order from the validated palette — `#3987e5`, `#d95926`, `#199e70`, `#c98500`, plus a fifth for thinking; a legend with direct labels, always present; **no dual-axis charts** — dollars ride as secondary labels, never a second y-scale; tabular numerals throughout; recessive grid lines; 2 pt gaps between stacked segments.

Swift Charts is in the SDK — do not add a package.

```swift
import SwiftUI
import Charts
import PerchFFI

struct UsageView: View {
    let engine: PerchEngine
    @State private var model: UsageModel?
    @State private var error: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if let error { Text(error).foregroundStyle(.secondary) }
                if let model {
                    heroBlock(model)
                    dailyBlock(model)
                    topProjectsBlock(model)
                    byModelBlock(model)
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task { await load() }
    }

    private func load() async {
        switch await engine.usage() {
        case .success(let m): model = m; error = nil
        case .failure(let e): error = e.localizedDescription
        }
    }
}
```

The daily chart flattens each `DailyBar` into one `BarMark` per class so `foregroundStyle(by:)` can stack them, with `chartForegroundStyleScale` pinning the palette order.

- [ ] **Step 1: Implement the four blocks**

Write `UsageView.swift` in full. Every displayed value comes from the model: `stat.value`, `stat.caption`, `bar.totalLabel`, `p.tokensLabel`, `p.cost`, `m.tokens`, `m.cost`. Chart axes use the numeric fields (`tokens`, `input`, `output`, …) and label with the model's strings.

When `hasData` is false, show one honest line ("No usage indexed yet") instead of four empty charts.

- [ ] **Step 2: Route to it**

Give the detail pane an Overview/Usage control. **Now** and a project selection both show Overview by default; the Usage tab shows `UsageView`. Keep the switch in `Sidebar.swift`'s detail pane so navigation stays in one place.

- [ ] **Step 3: Build, run, verify**

Run: `cd apps/macos/Perch && swift build -c release 2>&1 | tail -3`
Run: `make bundle && open build/Perch.app`

Verify and report: the Usage tab shows the hero stats (with a burn-rate line only when there is enough history), 14 daily stacked bars with a legend, top projects, and the by-model table; nothing is clipped; light and dark both read correctly.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Perch/Sources/Perch
git commit -m "feat(macos): usage view with daily bars, top projects, and by-model"
```

---

## Task 11: CI and docs

**Files:**
- Modify: `.github/workflows/ci.yml`, `README.md`, `docs/BACKLOG.md`

- [ ] **Step 1: Confirm the audits still cover everything**

The existing globs (`crates/*/Cargo.toml`, `crates/*/src/*`, `apps/macos/Perch/Sources/Perch/**/*.swift`) already reach the new files. **Verify by running each guard's shell body against the real tree**, exactly as CI would, and confirm each passes and its self-test fires. In particular the filesystem-write audit now sees `Launcher.swift` — that is Swift, which the write audit does not scan (it audits Rust), so confirm the audit's scope is still correct and say so.

Add nothing new unless a guard fails. If one does, that is a finding to report, not a licence to weaken it.

- [ ] **Step 2: README**

Add the main window to the native-app section: what it shows (every project with stats and full session history, a usage view), how to open it (the status item's **Open Perch**), and the two terminal actions. Keep the privacy claims exactly as true as they are now — the launcher writes a script into *Perch's own* directory and starts a terminal; it makes no network request and touches nothing in the Claude Code directory. Say that plainly rather than leaving it to inference.

- [ ] **Step 3: Backlog**

Move to **Shipped**: the main window, the usage view, jump/resume, and per-project stats. The Tauri-removal item moves from "Next — settings & polish" into **Now**, since this milestone is the parity it was waiting on. Rewrite each moved line to describe what shipped.

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test --workspace` (sanity — Markdown and YAML only).

```bash
git add .github/workflows/ci.yml README.md docs/BACKLOG.md
git commit -m "docs: document the main window; update the backlog"
```

---

## Self-Review

**Spec coverage.** §2 scope — project list (T3, T8), project detail (T3, T9), usage view (T4, T10), actions (T5, T9), Open Perch item (T7). §3 architecture — every file in the spec's tree has a task. §4 data flow: the pull model is T7's async engine reads; edits returning refreshed models is T6. §5 view-model — T3, T4. §6 actions: Rust composes (T5), Swift spawns (T9), script + `open -a` (T9), Jump activates the app (T9). §7 error handling — typed FFI errors (T6), the disabled-action path (T3's `path_exists`, T9), the action error banner (T9). §8 testing — Rust tests in T1–T5, FFI tests in T6, hand-tests in T7–T10, CI in T11. §9 cross-platform — `actions.rs` and both `ui` modules are platform-neutral by construction.

**Deviation recorded:** the spec's §4 says a watcher tick also refreshes the window's live rows. The plan implements the window as pull-only (T7/T8 reload on appear and after edits); the popover keeps its push. Wiring the push into the window is a small follow-up and is called out in T8's report rather than half-built here.

**Type consistency checked:** `ProjectRow`/`ProjectDetail`/`SessionHistoryRow`/`SparkPoint`/`MainWindowModel` field names are identical across T3 (core), T6 (FFI mirrors), and T8/T9 (Swift, camelCased by UniFFI). `TerminalCommand` gains `shell_line` only at the FFI boundary (T6), which T9 consumes as `shellLine`. `query::{session_history, daily_usage, top_projects}` (T2) are exactly what T3 and T4 call. `db::{project_meta, set_archived, set_status}` (T1) are what T3 and T6 call.

**Placeholder scan:** none. Task 8 contains a deliberate, named stub that Task 9 replaces, which is recorded in both tasks.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks.
2. **Inline Execution** — tasks executed in this session with batch checkpoints.
