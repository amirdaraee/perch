# Perch Native macOS App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Tauri popover with a native AppKit `NSMenu` app whose cards are SwiftUI views, driven by a Rust view-model over UniFFI — so the menu bar stays visible over fullscreen apps and the presentation layer becomes reusable for Linux/Windows shells.

**Architecture:** `perch-core` gains a `ui` module (formatting, `PopoverModel`, the watcher — all moved out of `perch-cli`/`src-tauri`). A thin `perch-ffi` crate exports `Perch`, `PerchListener`, and the records via UniFFI proc-macros and builds as a static library. A script produces `PerchCore.xcframework` with generated Swift bindings. `apps/macos/Perch` is a SwiftPM executable: `NSStatusItem` → `NSMenu` → `NSMenuItem.view = NSHostingView(card)`. Rust owns every string the UI shows; Swift only draws.

**Tech Stack:** Rust 1.98, `uniffi` 0.32 (proc-macros, `cli` feature), `notify` 8, Swift 6.3 / SwiftPM (no Xcode project), macOS 15+, `xcodebuild -create-xcframework`.

**Spec:** `docs/superpowers/specs/2026-09-04-native-macos-app-design.md` — implements §2–§6. The 2026-08-30 spec's data-layer sections stay binding.

## Global Constraints

- **Rust owns everything except drawing.** Formatting, ordering, labels, `—` for absent data, the tray title — all produced in `perch-core`. Swift never formats a number or duration.
- **Read-only:** only `perch-core` reads the Claude Code config dir; only it writes Perch's app-data dir. No Swift file touches either.
- **No network, no telemetry** anywhere. `perch-ffi` and the Swift app add no network dependency; CI's `no-network` job must cover the new crate.
- **No transcript/message contents** cross the FFI. Names, cwds, statuses, counts, formatted totals only.
- Rust edition 2021; `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and all existing tests (87) must pass throughout.
- `perch-core` and `perch-ffi` must keep compiling on Linux. Only `apps/macos` is macOS-only.
- Minimum macOS **15**. Swift tools **6.0** manifest. SwiftPM only.
- The Tauri app (`src-tauri`, `src/`) stays in the repo untouched until parity. Moving code *out* of `src-tauri` is allowed only where this plan says so, and `src-tauri` must keep building.
- Blocked (`Waiting`) sessions sort first, then by name, then by pid — unchanged from the live module.
- Every step is one action. Commit after each task. Run each cargo/swift build as its own isolated tool call.

---

## File Structure

```
crates/perch-core/src/ui/
├── mod.rs        pub mod format; pub mod model; pub mod watcher;
├── format.rs     human_tokens, human_cost, human_elapsed   (moved from perch-cli)
├── model.rs      PopoverModel & friends; build_model()     (new)
└── watcher.rs    Watcher (moved from src-tauri, Tauri-free)  (new)
crates/perch-core/src/query.rs      + recent_sessions(), session_usage()

crates/perch-ffi/
├── Cargo.toml
├── src/lib.rs               uniffi exports: Perch, PerchListener, PerchError, records
├── src/bin/uniffi-bindgen.rs
└── src/bin/uniffi-bindgen-swift.rs

scripts/build-xcframework.sh

apps/macos/Perch/
├── Package.swift
├── Makefile
├── Resources/Info.plist
└── Sources/Perch/
    ├── PerchApp.swift               @main, AppDelegate, Accessory policy
    ├── Bridge/PerchEngine.swift     owns Perch, main-actor model publishing
    ├── StatusItemController.swift  NSStatusItem + NSMenu + card hosting
    └── Cards/{StatsCard,SessionRow,RecentCard,EmptyCard}.swift
```

---

## Task 1: Move formatting into `perch-core::ui::format`

**Files:**
- Create: `crates/perch-core/src/ui/mod.rs`, `crates/perch-core/src/ui/format.rs`
- Modify: `crates/perch-core/src/lib.rs`, `crates/perch-cli/src/main.rs`

**Interfaces:**
- Produces: `pub fn human_tokens(n: u64) -> String`, `pub fn human_cost(usd: f64) -> String`, `pub fn human_elapsed(ms: i64) -> String`, `pub fn elapsed_or_dash(now_ms: i64, since_ms: i64) -> String` — identical behaviour to today's `perch-cli` helpers.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/ui/format.rs`:

```rust
//! Human formatting. One place, shared by every shell.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_compact() {
        assert_eq!(human_tokens(0), "0");
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(1_500), "1.5k");
        assert_eq!(human_tokens(24_221), "24.2k");
        assert_eq!(human_tokens(4_100_000), "4.1M");
    }

    #[test]
    fn cost_has_two_decimals() {
        assert_eq!(human_cost(0.0), "$0.00");
        assert_eq!(human_cost(8.204), "$8.20");
    }

    #[test]
    fn elapsed_buckets_without_days() {
        assert_eq!(human_elapsed(0), "0s");
        assert_eq!(human_elapsed(45_000), "45s");
        assert_eq!(human_elapsed(90_000), "1m");
        assert_eq!(human_elapsed(3_600_000), "1h");
        assert_eq!(human_elapsed(115_200_000), "32h");
        assert_eq!(human_elapsed(-5_000), "0s", "clock skew must not panic or go negative");
    }

    #[test]
    fn absent_timestamp_is_a_dash_not_the_epoch() {
        assert_eq!(elapsed_or_dash(1_000_000, 0), "—");
        assert_eq!(elapsed_or_dash(1_000_000, -1), "—");
        assert_eq!(elapsed_or_dash(1_000_000, 940_000), "1m");
    }

    #[test]
    fn a_real_timestamp_formats_the_same_as_human_elapsed() {
        assert_eq!(elapsed_or_dash(10_000_000, 2_800_000), human_elapsed(7_200_000));
    }
}
```

Create `crates/perch-core/src/ui/mod.rs`:

```rust
//! View-model layer: everything a shell needs to draw, computed here so every
//! platform renders the same thing.
pub mod format;
```

- [ ] **Step 2: Register and run to see them fail**

Add `pub mod ui;` to `crates/perch-core/src/lib.rs`.
Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::format`
Expected: FAIL — `cannot find function human_tokens`.

- [ ] **Step 3: Implement (moved verbatim from `perch-cli`, plus `elapsed_or_dash`)**

Insert above the tests in `format.rs`:

```rust
pub fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn human_cost(usd: f64) -> String {
    format!("${usd:.2}")
}

/// Deliberately no "days" bucket: a session blocked for 32 hours should read
/// as 32h, which is more alarming than 1d 8h. That alarm is the point.
pub fn human_elapsed(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// A missing timestamp is `0` on the wire. Formatting `now - 0` renders ~56 years,
/// so the caller's "absent" must become a dash, never a duration.
pub fn elapsed_or_dash(now_ms: i64, since_ms: i64) -> String {
    if since_ms <= 0 {
        "—".to_string()
    } else {
        human_elapsed(now_ms - since_ms)
    }
}
```

- [ ] **Step 4: Point `perch-cli` at the shared helpers**

In `crates/perch-cli/src/main.rs`, delete the local `human_tokens`, `human_cost`, `human_elapsed`, and `elapsed_or_dash` functions and their tests, and add at the top:

```rust
use perch_core::ui::format::{elapsed_or_dash, human_cost, human_elapsed, human_tokens};
```

Keep `truncate` in the CLI (it is display-width logic specific to a terminal table).

- [ ] **Step 5: Verify**

Run: `source "$HOME/.cargo/env" && cargo test --workspace`
Expected: the five CLI formatting tests are gone, five core tests pass; total unchanged at 87.
Run: `source "$HOME/.cargo/env" && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/ui crates/perch-core/src/lib.rs crates/perch-cli/src/main.rs
git commit -m "refactor(core): move human formatting into perch-core::ui::format"
```

---

## Task 2: Read-side queries the popover needs

**Files:**
- Modify: `crates/perch-core/src/query.rs`

**Interfaces:**
- Consumes: `db::Db`, `pricing::price_for`, `model::TurnUsage`.
- Produces:
  - `pub struct RecentSession { pub id: String, pub cwd: Option<String>, pub last_activity_at: i64, pub usage: TurnUsage, pub cost_usd: f64 }`
  - `pub fn recent_sessions(db: &Db, exclude_ids: &[String], limit: usize) -> Result<Vec<RecentSession>>` — most recent first by `last_activity_at`, skipping ids that are live.
  - `pub fn session_usage(db: &Db, session_id: &str) -> Result<(TurnUsage, f64)>` — per-model priced, summed.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` in `crates/perch-core/src/query.rs`:

```rust
    fn seed_session(db: &Db, pid: i64, id: &str, cwd: &str, last: i64, input: u64) {
        db.upsert_session(&SessionRecord {
            id: id.into(), project_id: pid, file_path: format!("/tmp/{id}.jsonl"),
            file_size: 0, indexed_offset: 0, started_at: Some(last - 1000),
            last_activity_at: Some(last), cwd: Some(cwd.into()), git_branch: None,
            cc_version: None, message_count: 1,
        }).unwrap();
        db.insert_turns(id, &[Turn { ts: last, model: "claude-fable-5".into(),
            usage: TurnUsage { input, ..Default::default() } }]).unwrap();
    }

    #[test]
    fn session_usage_prices_per_model() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-Users-a-one", "/Users/a/one", false).unwrap();
        seed_session(&db, pid, "s-a", "/Users/a/one", 10_000, 2_000_000);
        let (u, cost) = session_usage(&db, "s-a").unwrap();
        assert_eq!(u.input, 2_000_000);
        assert!((cost - 30.0).abs() < 1e-9, "2 MTok at fable input 15.0");
    }

    #[test]
    fn session_usage_for_unknown_session_is_zero_not_error() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let (u, cost) = session_usage(&db, "nope").unwrap();
        assert_eq!(u.total_tokens(), 0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn recent_sessions_are_newest_first_and_skip_live_ones() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-Users-a-one", "/Users/a/one", false).unwrap();
        seed_session(&db, pid, "old", "/Users/a/one", 1_000, 1);
        seed_session(&db, pid, "mid", "/Users/a/one", 2_000, 1);
        seed_session(&db, pid, "new", "/Users/a/one", 3_000, 1);
        seed_session(&db, pid, "live", "/Users/a/one", 4_000, 1);

        let rows = recent_sessions(&db, &["live".to_string()], 2).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "mid"], "newest first, live excluded, limited to 2");
        assert_eq!(rows[0].cwd.as_deref(), Some("/Users/a/one"));
        assert_eq!(rows[0].usage.input, 1);
    }

    #[test]
    fn recent_sessions_with_empty_index_is_empty() {
        let db = open_in_memory().unwrap();
        assert!(recent_sessions(&db, &[], 3).unwrap().is_empty());
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core query`
Expected: FAIL — `cannot find function session_usage`.

- [ ] **Step 3: Implement**

Insert above the tests in `query.rs` (after `usage_by_model`):

```rust
#[derive(Debug, Clone)]
pub struct RecentSession {
    pub id: String,
    pub cwd: Option<String>,
    pub last_activity_at: i64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
}

/// Tokens and estimated cost for one session, priced per model and summed.
/// An unknown session is simply zero — it is not an error to ask.
pub fn session_usage(db: &Db, session_id: &str) -> Result<(TurnUsage, f64)> {
    let args: [&dyn rusqlite::ToSql; 1] = [&session_id];
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE session_id = ?1 GROUP BY model"),
        &args,
    )?;
    let mut usage = TurnUsage::default();
    let mut cost = 0.0;
    for (model, u) in &per_model {
        usage = usage.plus(u);
        cost += cost_of(db, model, u)?;
    }
    Ok((usage, cost))
}

/// The most recently active sessions that are NOT currently live, newest first.
pub fn recent_sessions(db: &Db, exclude_ids: &[String], limit: usize) -> Result<Vec<RecentSession>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, cwd, last_activity_at FROM sessions
         WHERE last_activity_at IS NOT NULL
         ORDER BY last_activity_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, cwd, last) = row?;
        if exclude_ids.iter().any(|x| x == &id) {
            continue;
        }
        let (usage, cost_usd) = session_usage(db, &id)?;
        out.push(RecentSession { id, cwd, last_activity_at: last, usage, cost_usd });
        if out.len() == limit {
            break;
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core query` → PASS, 10 tests.
Run: `source "$HOME/.cargo/env" && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/query.rs
git commit -m "feat(core): per-session usage and recent-sessions queries"
```

---

## Task 3: `PopoverModel` — the view-model, in Rust

**Files:**
- Create: `crates/perch-core/src/ui/model.rs`
- Modify: `crates/perch-core/src/ui/mod.rs`

**Interfaces:**
- Consumes: `live::{LiveSession, SessionStatus}`, `query::{usage_since, session_usage, recent_sessions}`, `db::Db`, `ui::format::*`.
- Produces (all `Clone + Debug + PartialEq + serde::Serialize`):

```rust
pub enum Status { Working, Idle, Waiting, Background }
pub struct Stats { pub window_tokens: String, pub week_tokens: String, pub day_tokens: String,
                   pub day_cost: String, pub estimated: bool, pub has_data: bool }
pub struct SessionRow { pub id: String, pub pid: i32, pub name: String, pub project: String,
                        pub kind: String, pub version: String, pub status: Status,
                        pub status_label: String, pub elapsed: String, pub tokens: String,
                        pub cost: String }
pub struct RecentRow { pub id: String, pub name: String, pub project: String,
                       pub ended_ago: String, pub tokens: String }
pub struct PopoverModel { pub stats: Stats, pub live: Vec<SessionRow>, pub recent: Vec<RecentRow>,
                          pub tray_title: String, pub error: Option<String> }
pub fn build_model(db: Option<&Db>, live: &[LiveSession], now_ms: i64) -> PopoverModel
pub fn tray_title(live: &[LiveSession]) -> String
```

`build_model` takes `Option<&Db>` so a database failure degrades to "sessions only" (spec §5). `project` is the last path component of `cwd`. `RecentRow.name` is the project name (ended sessions have no live record).

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/ui/model.rs`:

```rust
//! The view-model every shell renders. All strings are final here.

use crate::db::Db;
use crate::live::{LiveSession, SessionStatus};
use crate::ui::format::{elapsed_or_dash, human_cost, human_elapsed, human_tokens};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

    fn live(pid: i32, id: &str, name: &str, cwd: &str, status: SessionStatus, kind: &str) -> LiveSession {
        LiveSession {
            pid, session_id: id.into(), cwd: cwd.into(), name: name.into(), kind: kind.into(),
            status, started_at: 1_000, status_updated_at: 5_000,
            cc_version: Some("2.1.251".into()), socket_path: None,
        }
    }

    #[test]
    fn no_db_yields_sessions_only_with_dashes() {
        let s = live(1, "a", "alpha", "/Users/a/proj", SessionStatus::Working, "interactive");
        let m = build_model(None, &[s], 10_000);
        assert!(!m.stats.has_data);
        assert_eq!(m.stats.window_tokens, "—");
        assert_eq!(m.live.len(), 1);
        assert_eq!(m.live[0].tokens, "—", "no db → no per-session usage, shown as a dash");
        assert!(m.recent.is_empty());
    }

    #[test]
    fn rows_carry_project_kind_version_and_labels() {
        let s = live(7, "a", "alpha", "/Users/a/proj", SessionStatus::Idle, "interactive");
        let m = build_model(None, &[s], 10_000);
        let r = &m.live[0];
        assert_eq!(r.project, "proj", "last path component of cwd");
        assert_eq!(r.kind, "interactive");
        assert_eq!(r.version, "2.1.251");
        assert_eq!(r.status, Status::Idle);
        assert_eq!(r.status_label, "idle");
        assert_eq!(r.elapsed, "5s", "now 10_000 - status_updated_at 5_000");
    }

    #[test]
    fn waiting_label_includes_the_reason_and_elapsed_since_waiting() {
        let mut s = live(7, "a", "alpha", "/x", SessionStatus::Working, "interactive");
        s.status = SessionStatus::Waiting { reason: Some("dialog open".into()), since_ms: 2_000 };
        let m = build_model(None, &[s], 10_000);
        assert_eq!(m.live[0].status_label, "waiting · dialog open");
        assert_eq!(m.live[0].elapsed, "8s");
        assert_eq!(m.live[0].status, Status::Waiting);
    }

    #[test]
    fn absent_status_timestamp_is_a_dash() {
        let mut s = live(7, "a", "alpha", "/x", SessionStatus::Working, "interactive");
        s.status_updated_at = 0;
        let m = build_model(None, &[s], 10_000);
        assert_eq!(m.live[0].elapsed, "—");
    }

    #[test]
    fn background_kind_maps_to_background_status_unless_waiting() {
        let bg = live(1, "a", "alpha", "/x", SessionStatus::Working, "bg");
        let mut bg_wait = live(2, "b", "beta", "/x", SessionStatus::Working, "bg");
        bg_wait.status = SessionStatus::Waiting { reason: None, since_ms: 1 };
        let m = build_model(None, &[bg, bg_wait], 10_000);
        let by_id = |id: &str| m.live.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id("a").status, Status::Background);
        assert_eq!(by_id("b").status, Status::Waiting, "a blocked bg session is still blocked");
    }

    #[test]
    fn tray_title_counts_and_flags_waiting() {
        let w = {
            let mut s = live(1, "a", "a", "/x", SessionStatus::Working, "interactive");
            s.status = SessionStatus::Waiting { reason: None, since_ms: 1 };
            s
        };
        let b = live(2, "b", "b", "/x", SessionStatus::Working, "interactive");
        assert_eq!(tray_title(&[]), "");
        assert_eq!(tray_title(&[b.clone()]), "1");
        assert_eq!(tray_title(&[w, b]), "1 ⏳");
    }

    #[test]
    fn stats_and_per_session_usage_come_from_the_index() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-Users-a-proj", "/Users/a/proj", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "a".into(), project_id: pid, file_path: "/tmp/a.jsonl".into(), file_size: 0,
            indexed_offset: 0, started_at: Some(1_000), last_activity_at: Some(9_000),
            cwd: Some("/Users/a/proj".into()), git_branch: None, cc_version: None, message_count: 1,
        }).unwrap();
        db.insert_turns("a", &[Turn { ts: 9_000, model: "claude-fable-5".into(),
            usage: TurnUsage { input: 2_000_000, ..Default::default() } }]).unwrap();

        let s = live(7, "a", "alpha", "/Users/a/proj", SessionStatus::Working, "interactive");
        let m = build_model(Some(&db), &[s], 10_000);
        assert!(m.stats.has_data);
        assert!(m.stats.estimated);
        assert_eq!(m.stats.window_tokens, "2.0M");
        assert_eq!(m.stats.day_cost, "$30.00");
        assert_eq!(m.live[0].tokens, "2.0M");
        assert_eq!(m.live[0].cost, "$30.00");
    }

    #[test]
    fn recent_excludes_live_and_names_by_project() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-Users-a-proj", "/Users/a/proj", false).unwrap();
        for (id, last) in [("live", 9_000), ("gone", 8_000)] {
            db.upsert_session(&SessionRecord {
                id: id.into(), project_id: pid, file_path: format!("/tmp/{id}.jsonl"), file_size: 0,
                indexed_offset: 0, started_at: Some(1), last_activity_at: Some(last),
                cwd: Some("/Users/a/proj".into()), git_branch: None, cc_version: None, message_count: 1,
            }).unwrap();
        }
        let s = live(7, "live", "alpha", "/Users/a/proj", SessionStatus::Working, "interactive");
        let m = build_model(Some(&db), &[s], 10_000);
        assert_eq!(m.recent.len(), 1);
        assert_eq!(m.recent[0].id, "gone");
        assert_eq!(m.recent[0].project, "proj");
        assert_eq!(m.recent[0].ended_ago, "2s");
    }
}
```

- [ ] **Step 2: Register and run to see them fail**

In `crates/perch-core/src/ui/mod.rs` add `pub mod model;`.
Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::model`
Expected: FAIL — `cannot find function build_model`.

- [ ] **Step 3: Implement**

Insert above the tests in `model.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Status {
    Working,
    Idle,
    Waiting,
    Background,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Stats {
    pub window_tokens: String,
    pub week_tokens: String,
    pub day_tokens: String,
    pub day_cost: String,
    /// Always true until the tier-1 usage endpoint lands (spec §8).
    pub estimated: bool,
    pub has_data: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionRow {
    pub id: String,
    pub pid: i32,
    pub name: String,
    pub project: String,
    pub kind: String,
    pub version: String,
    pub status: Status,
    pub status_label: String,
    pub elapsed: String,
    pub tokens: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RecentRow {
    pub id: String,
    pub name: String,
    pub project: String,
    pub ended_ago: String,
    pub tokens: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PopoverModel {
    pub stats: Stats,
    pub live: Vec<SessionRow>,
    pub recent: Vec<RecentRow>,
    pub tray_title: String,
    pub error: Option<String>,
}

const DASH: &str = "—";
const RECENT_LIMIT: usize = 3;
const FIVE_HOURS_MS: i64 = 5 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_MS: i64 = 7 * DAY_MS;

fn project_of(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}

/// `SessionStatus` (live.rs) has Working | Idle | Waiting | Background | Ended.
/// Blocked wins over everything; a `bg` *kind* renders as Background even when
/// its status says busy, because the user cannot type into it either way.
fn status_of(s: &LiveSession) -> (Status, String, i64) {
    match &s.status {
        SessionStatus::Waiting { reason, since_ms } => (
            Status::Waiting,
            format!("waiting · {}", reason.as_deref().unwrap_or("unknown")),
            *since_ms,
        ),
        SessionStatus::Background => (Status::Background, "background".into(), s.status_updated_at),
        _ if s.kind == "bg" => (Status::Background, "background".into(), s.status_updated_at),
        SessionStatus::Idle => (Status::Idle, "idle".into(), s.status_updated_at),
        SessionStatus::Ended => (Status::Idle, "ended".into(), s.status_updated_at),
        SessionStatus::Working => (Status::Working, "working".into(), s.status_updated_at),
    }
}

/// Menu-bar text: blocked count with an hourglass, else live count, else nothing.
pub fn tray_title(live: &[LiveSession]) -> String {
    let waiting = live
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Waiting { .. }))
        .count();
    if waiting > 0 {
        format!("{waiting} ⏳")
    } else if live.is_empty() {
        String::new()
    } else {
        live.len().to_string()
    }
}

fn dashed_stats() -> Stats {
    Stats {
        window_tokens: DASH.into(),
        week_tokens: DASH.into(),
        day_tokens: DASH.into(),
        day_cost: DASH.into(),
        estimated: true,
        has_data: false,
    }
}

fn stats_from(db: &Db, now_ms: i64) -> crate::db::Result<Stats> {
    use crate::query::usage_since;
    if db.turn_count()? == 0 {
        return Ok(dashed_stats());
    }
    let (w, _) = usage_since(db, now_ms - FIVE_HOURS_MS)?;
    let (k, _) = usage_since(db, now_ms - WEEK_MS)?;
    let (d, d_cost) = usage_since(db, now_ms - DAY_MS)?;
    Ok(Stats {
        window_tokens: human_tokens(w.total_tokens()),
        week_tokens: human_tokens(k.total_tokens()),
        day_tokens: human_tokens(d.total_tokens()),
        day_cost: human_cost(d_cost),
        estimated: true,
        has_data: true,
    })
}

/// Build the whole view-model. `db: None` (or a failing db) degrades to sessions-only:
/// the sessions list needs no index, and an honest dash beats a fabricated zero.
pub fn build_model(db: Option<&Db>, live: &[LiveSession], now_ms: i64) -> PopoverModel {
    let mut error = None;

    let stats = match db.map(|d| stats_from(d, now_ms)) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            error = Some(format!("index unavailable: {e}"));
            dashed_stats()
        }
        None => dashed_stats(),
    };

    let rows: Vec<SessionRow> = live
        .iter()
        .map(|s| {
            let (status, status_label, since) = status_of(s);
            let (tokens, cost) = match db.and_then(|d| crate::query::session_usage(d, &s.session_id).ok()) {
                Some((u, c)) if u.total_tokens() > 0 => (human_tokens(u.total_tokens()), human_cost(c)),
                _ => (DASH.into(), DASH.into()),
            };
            SessionRow {
                id: s.session_id.clone(),
                pid: s.pid,
                name: if s.name.is_empty() { s.session_id.chars().take(8).collect() } else { s.name.clone() },
                project: project_of(&s.cwd),
                kind: s.kind.clone(),
                version: s.cc_version.clone().unwrap_or_default(),
                status,
                status_label,
                elapsed: elapsed_or_dash(now_ms, since),
                tokens,
                cost,
            }
        })
        .collect();

    let live_ids: Vec<String> = live.iter().map(|s| s.session_id.clone()).collect();
    let recent: Vec<RecentRow> = db
        .and_then(|d| crate::query::recent_sessions(d, &live_ids, RECENT_LIMIT).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let project = r.cwd.as_deref().map(project_of).unwrap_or_else(|| r.id.chars().take(8).collect());
            RecentRow {
                id: r.id,
                name: project.clone(),
                project,
                ended_ago: human_elapsed(now_ms - r.last_activity_at),
                tokens: if r.usage.total_tokens() > 0 { human_tokens(r.usage.total_tokens()) } else { DASH.into() },
            }
        })
        .collect();

    PopoverModel { stats, live: rows, recent, tray_title: tray_title(live), error }
}
```

Note: `crate::db::Result` — if `db.rs` does not export a `Result` alias, use `anyhow::Result<Stats>` and `format!("{e}")` works the same. The `live` slice is expected already sorted by `live_sessions` (blocked first, name, pid); `build_model` preserves order.

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::model` → PASS, 8 tests.
Run: `source "$HOME/.cargo/env" && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/ui
git commit -m "feat(core): PopoverModel view-model with formatted rows, stats, and recent"
```

---

## Task 4: Move the watcher into `perch-core::ui::watcher`

**Files:**
- Create: `crates/perch-core/src/ui/watcher.rs`
- Modify: `crates/perch-core/src/ui/mod.rs`, `crates/perch-core/Cargo.toml`, root `Cargo.toml`

**Interfaces:**
- Consumes: `live::live_sessions`, `platform::{ProcessProbe, RealProcessProbe}`, `ui::model::build_model`, `config`.
- Produces:
  - `pub struct WatcherConfig { pub sessions_dir: PathBuf, pub debounce: Duration, pub poll: Duration }`
  - `pub struct WatcherHandle` with `pub fn stop(self)` and `pub fn refresh(&self)`.
  - `pub fn spawn<F>(cfg: WatcherConfig, probe: Arc<dyn ProcessProbe>, on_sessions: F) -> WatcherHandle where F: Fn(Vec<LiveSession>) + Send + 'static`

The watcher emits **sessions**, not models — the FFI layer combines sessions with the database into a `PopoverModel`, keeping the watcher free of database concerns. Behaviour is the current `src-tauri/src/watcher.rs` verbatim: initial emit, 250 ms debounce with trailing-edge re-emit, 5 s poll backstop, poll-only fallback when the directory is missing with a retry each tick, logging on failures, never creating the directory. Plus two things Tauri's version lacked: a `refresh()` that forces an emit now, and `stop()`.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/ui/watcher.rs`:

```rust
//! Watches the sessions directory and reports the live session list.
//! Moved from the Tauri shell; no UI framework in sight.

use crate::live::{live_sessions, LiveSession};
use crate::platform::ProcessProbe;
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct AllAlive;
    impl ProcessProbe for AllAlive {
        fn is_alive(&self, _pid: i32) -> bool { true }
        fn process_name(&self, _pid: i32) -> Option<String> { Some("claude".into()) }
    }

    fn write_record(dir: &Path, pid: i32, id: &str) {
        std::fs::write(
            dir.join(format!("{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"{id}","cwd":"/t","name":"n","kind":"interactive","status":"busy","startedAt":1,"statusUpdatedAt":2}}"#),
        ).unwrap();
    }

    fn cfg(dir: &Path) -> WatcherConfig {
        WatcherConfig { sessions_dir: dir.to_path_buf(), debounce: Duration::from_millis(50), poll: Duration::from_millis(300) }
    }

    #[test]
    fn emits_once_on_start_and_again_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 1, "one");
        let (tx, rx) = mpsc::channel();
        let h = spawn(cfg(tmp.path()), Arc::new(AllAlive), move |s| { let _ = tx.send(s); });

        let first = rx.recv_timeout(Duration::from_secs(2)).expect("initial emit");
        assert_eq!(first.len(), 1);

        write_record(tmp.path(), 2, "two");
        let mut seen: HashSet<usize> = HashSet::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) { seen.insert(s.len()); }
            if seen.contains(&2) { break; }
        }
        assert!(seen.contains(&2), "a new record must surface via watch or poll");
        h.stop();
    }

    #[test]
    fn missing_directory_falls_back_to_polling_and_never_creates_it() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("sessions");
        let (tx, rx) = mpsc::channel();
        let h = spawn(cfg(&missing), Arc::new(AllAlive), move |s| { let _ = tx.send(s); });

        let first = rx.recv_timeout(Duration::from_secs(2)).expect("poll-only still emits");
        assert!(first.is_empty());
        assert!(!missing.exists(), "watcher must never create the sessions dir");

        // Directory appears later: the retry should pick up the record within a few polls.
        std::fs::create_dir_all(&missing).unwrap();
        write_record(&missing, 3, "three");
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got = false;
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) { if s.len() == 1 { got = true; break; } }
        }
        assert!(got, "record written after the dir appears must surface");
        h.stop();
    }

    #[test]
    fn refresh_forces_an_immediate_emit() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let h = spawn(WatcherConfig { sessions_dir: tmp.path().into(), debounce: Duration::from_millis(50), poll: Duration::from_secs(30) }, Arc::new(AllAlive), move |s| { let _ = tx.send(s); });
        rx.recv_timeout(Duration::from_secs(2)).expect("initial");
        h.refresh();
        rx.recv_timeout(Duration::from_secs(1)).expect("refresh must not wait for the 30s poll");
        h.stop();
    }
}
```

- [ ] **Step 2: Dependencies and registration**

Root `Cargo.toml` `[workspace.dependencies]`: add `notify = "8"`.
`crates/perch-core/Cargo.toml` `[dependencies]`: add `notify.workspace = true`.
`crates/perch-core/src/ui/mod.rs`: add `pub mod watcher;`.

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::watcher`
Expected: FAIL — `cannot find function spawn`.

- [ ] **Step 3: Implement**

Insert above the tests in `watcher.rs`:

```rust
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub sessions_dir: PathBuf,
    pub debounce: Duration,
    pub poll: Duration,
}

impl WatcherConfig {
    /// Production defaults: 250 ms coalescing, 5 s backstop.
    pub fn for_dir(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir, debounce: Duration::from_millis(250), poll: Duration::from_secs(5) }
    }
}

enum Cmd {
    Refresh,
    Stop,
}

pub struct WatcherHandle {
    cmd: mpsc::Sender<Cmd>,
}

impl WatcherHandle {
    /// Emit now, regardless of debounce. Used when the menu opens.
    pub fn refresh(&self) {
        let _ = self.cmd.send(Cmd::Refresh);
    }
    pub fn stop(self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

/// Start watching. Emits the current list immediately, then on every change
/// (debounced, trailing-edge) and at least every `poll` as a backstop — a session
/// whose process dies produces no filesystem event, so polling is what removes it.
pub fn spawn<F>(cfg: WatcherConfig, probe: Arc<dyn ProcessProbe>, on_sessions: F) -> WatcherHandle
where
    F: Fn(Vec<LiveSession>) + Send + 'static,
{
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    std::thread::spawn(move || run(cfg, probe, on_sessions, cmd_rx));
    WatcherHandle { cmd: cmd_tx }
}

fn run<F>(cfg: WatcherConfig, probe: Arc<dyn ProcessProbe>, on_sessions: F, cmd_rx: mpsc::Receiver<Cmd>)
where
    F: Fn(Vec<LiveSession>),
{
    let dir = cfg.sessions_dir.clone();
    let emit = |probe: &dyn ProcessProbe| on_sessions(live_sessions(&dir, probe));

    // Filesystem events and control commands share one channel so the loop has one wait.
    let (ev_tx, ev_rx) = mpsc::channel::<Event>();
    let fs_tx = ev_tx.clone();
    let mut watcher = match notify::recommended_watcher(move |_res| {
        let _ = fs_tx.send(Event::Fs);
    }) {
        Ok(w) => Some(w),
        Err(err) => {
            eprintln!("perch: watcher: failed to create filesystem watcher: {err}");
            None
        }
    };
    // Bridge commands into the same channel.
    std::thread::spawn(move || {
        for c in cmd_rx {
            let stop = matches!(c, Cmd::Stop);
            let _ = ev_tx.send(Event::Cmd(c));
            if stop {
                break;
            }
        }
    });

    let mut watching = try_watch(watcher.as_mut(), &dir, true);

    emit(&*probe);
    let mut last = Instant::now();
    let mut pending = false;

    loop {
        let wait = if !watching {
            cfg.poll
        } else if pending {
            cfg.debounce.saturating_sub(last.elapsed())
        } else {
            cfg.poll
        };
        match ev_rx.recv_timeout(wait) {
            Ok(Event::Cmd(Cmd::Stop)) => return,
            Ok(Event::Cmd(Cmd::Refresh)) => {
                emit(&*probe);
                last = Instant::now();
                pending = false;
            }
            Ok(Event::Fs) => {
                if last.elapsed() >= cfg.debounce {
                    emit(&*probe);
                    last = Instant::now();
                    pending = false;
                } else {
                    pending = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !watching {
                    watching = try_watch(watcher.as_mut(), &dir, false);
                }
                emit(&*probe);
                last = Instant::now();
                pending = false;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

enum Event {
    Fs,
    Cmd(Cmd),
}

/// Never creates the directory: if it is absent, `watch` fails and we poll instead.
fn try_watch(watcher: Option<&mut notify::RecommendedWatcher>, dir: &Path, log_failure: bool) -> bool {
    let Some(w) = watcher else { return false };
    match w.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => true,
        Err(err) => {
            if log_failure {
                eprintln!("perch: watcher: failed to watch {}: {err} — polling every {:?} instead", dir.display(), Duration::from_secs(5));
            }
            false
        }
    }
}
```

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core ui::watcher` → PASS, 3 tests (they involve real timing; each finishes in under 4 s).
Run: `source "$HOME/.cargo/env" && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Confirm `src-tauri` still builds: `source "$HOME/.cargo/env" && cargo build -p perch-app` (it keeps its own copy of the watcher for now; nothing in it changed).

```bash
git add crates/perch-core Cargo.toml Cargo.lock
git commit -m "feat(core): framework-free session watcher with refresh and stop"
```

---

## Task 5: `perch-ffi` — the UniFFI surface

**Files:**
- Create: `crates/perch-ffi/Cargo.toml`, `crates/perch-ffi/src/lib.rs`, `crates/perch-ffi/src/bin/uniffi-bindgen.rs`, `crates/perch-ffi/src/bin/uniffi-bindgen-swift.rs`
- Modify: root `Cargo.toml` (members + workspace deps)

**Interfaces:**
- Consumes: `perch_core::{config, db, pricing, index, live, platform::RealProcessProbe, ui::{model, watcher}}`.
- Produces (UniFFI-exported):

```rust
#[derive(uniffi::Enum)]   pub enum Status { Working, Idle, Waiting, Background }
#[derive(uniffi::Record)] pub struct Stats { window_tokens: String, week_tokens: String, day_tokens: String, day_cost: String, estimated: bool, has_data: bool }
#[derive(uniffi::Record)] pub struct SessionRow { id: String, pid: i32, name: String, project: String, kind: String, version: String, status: Status, status_label: String, elapsed: String, tokens: String, cost: String }
#[derive(uniffi::Record)] pub struct RecentRow { id: String, name: String, project: String, ended_ago: String, tokens: String }
#[derive(uniffi::Record)] pub struct PopoverModel { stats: Stats, live: Vec<SessionRow>, recent: Vec<RecentRow>, tray_title: String, error: Option<String> }
#[derive(uniffi::Error)]  pub enum PerchError { NoConfigDir { path: String }, Database { message: String }, Io { message: String } }
#[uniffi::export(with_foreign)] pub trait PerchListener: Send + Sync { fn on_model(&self, model: PopoverModel); }
#[derive(uniffi::Object)] pub struct Perch { … }
#[uniffi::export] impl Perch {
    #[uniffi::constructor] fn new(config_dir: Option<String>) -> Result<Arc<Self>, PerchError>;
    fn current(&self) -> PopoverModel;
    fn start(&self, listener: Arc<dyn PerchListener>);
    fn refresh(&self);
    fn stop(&self);
}
```

The FFI records mirror `perch_core::ui::model` one-to-one with `From` impls; they are separate types so `perch-core` stays free of `uniffi`.

- [ ] **Step 1: Crate manifest and bindgen binaries**

Root `Cargo.toml`: add `"crates/perch-ffi"` to `members`; add to `[workspace.dependencies]`:

```toml
uniffi = { version = "0.32", features = ["cli"] }
```

Create `crates/perch-ffi/Cargo.toml`:

```toml
[package]
name = "perch-ffi"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
name = "perch_ffi"
crate-type = ["staticlib", "cdylib", "lib"]

[dependencies]
perch-core = { path = "../perch-core" }
uniffi.workspace = true
anyhow.workspace = true
chrono.workspace = true

[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi-bindgen.rs"

[[bin]]
name = "uniffi-bindgen-swift"
path = "src/bin/uniffi-bindgen-swift.rs"

[dev-dependencies]
tempfile.workspace = true
```

Create `crates/perch-ffi/src/bin/uniffi-bindgen.rs`:

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

Create `crates/perch-ffi/src/bin/uniffi-bindgen-swift.rs`:

```rust
fn main() {
    uniffi::uniffi_bindgen_swift()
}
```

- [ ] **Step 2: Write the failing smoke test**

Create `crates/perch-ffi/src/lib.rs` with only the test module for now:

```rust
//! UniFFI surface over perch-core. Records mirror `perch_core::ui::model` exactly;
//! the shell renders them and nothing else.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct Capture(Mutex<Vec<PopoverModel>>);
    impl PerchListener for Capture {
        fn on_model(&self, model: PopoverModel) {
            self.0.lock().unwrap().push(model);
        }
    }

    #[test]
    fn constructs_against_an_empty_config_dir_and_emits_a_model() {
        let tmp = tempfile::tempdir().unwrap();
        // Perch's own DB must not land inside the (fake) config dir either.
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let snap = perch.current();
        assert!(!snap.stats.has_data);
        assert_eq!(snap.stats.window_tokens, "—");
        assert!(snap.live.is_empty());

        let cap = Arc::new(Capture(Mutex::new(Vec::new())));
        perch.start(cap.clone());
        std::thread::sleep(Duration::from_millis(500));
        perch.stop();
        assert!(!cap.0.lock().unwrap().is_empty(), "start must emit at least the initial model");
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none(), "nothing written into the config dir");
    }

    #[test]
    fn missing_config_dir_is_a_typed_error() {
        let err = Perch::new(Some("/definitely/not/here".into())).err().expect("must fail");
        assert!(matches!(err, PerchError::NoConfigDir { .. }));
    }
}
```

Run: `source "$HOME/.cargo/env" && cargo test -p perch-ffi`
Expected: FAIL — `cannot find type Perch`.

- [ ] **Step 3: Implement the surface**

Insert above the tests in `lib.rs`:

```rust
use perch_core::platform::RealProcessProbe;
use perch_core::ui::{model as core_model, watcher};
use perch_core::{config, db, index, live, pricing};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Status { Working, Idle, Waiting, Background }

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Stats {
    pub window_tokens: String,
    pub week_tokens: String,
    pub day_tokens: String,
    pub day_cost: String,
    pub estimated: bool,
    pub has_data: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionRow {
    pub id: String,
    pub pid: i32,
    pub name: String,
    pub project: String,
    pub kind: String,
    pub version: String,
    pub status: Status,
    pub status_label: String,
    pub elapsed: String,
    pub tokens: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RecentRow {
    pub id: String,
    pub name: String,
    pub project: String,
    pub ended_ago: String,
    pub tokens: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PopoverModel {
    pub stats: Stats,
    pub live: Vec<SessionRow>,
    pub recent: Vec<RecentRow>,
    pub tray_title: String,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PerchError {
    #[error("no Claude Code config directory at {path}")]
    NoConfigDir { path: String },
    #[error("database error: {message}")]
    Database { message: String },
    #[error("io error: {message}")]
    Io { message: String },
}

impl From<core_model::Status> for Status {
    fn from(s: core_model::Status) -> Self {
        match s {
            core_model::Status::Working => Status::Working,
            core_model::Status::Idle => Status::Idle,
            core_model::Status::Waiting => Status::Waiting,
            core_model::Status::Background => Status::Background,
        }
    }
}
impl From<core_model::Stats> for Stats {
    fn from(s: core_model::Stats) -> Self {
        Stats { window_tokens: s.window_tokens, week_tokens: s.week_tokens, day_tokens: s.day_tokens, day_cost: s.day_cost, estimated: s.estimated, has_data: s.has_data }
    }
}
impl From<core_model::SessionRow> for SessionRow {
    fn from(r: core_model::SessionRow) -> Self {
        SessionRow { id: r.id, pid: r.pid, name: r.name, project: r.project, kind: r.kind, version: r.version, status: r.status.into(), status_label: r.status_label, elapsed: r.elapsed, tokens: r.tokens, cost: r.cost }
    }
}
impl From<core_model::RecentRow> for RecentRow {
    fn from(r: core_model::RecentRow) -> Self {
        RecentRow { id: r.id, name: r.name, project: r.project, ended_ago: r.ended_ago, tokens: r.tokens }
    }
}
impl From<core_model::PopoverModel> for PopoverModel {
    fn from(m: core_model::PopoverModel) -> Self {
        PopoverModel {
            stats: m.stats.into(),
            live: m.live.into_iter().map(Into::into).collect(),
            recent: m.recent.into_iter().map(Into::into).collect(),
            tray_title: m.tray_title,
            error: m.error,
        }
    }
}

/// Implemented by the shell. Called on the watcher thread; the shell hops to its UI thread.
#[uniffi::export(with_foreign)]
pub trait PerchListener: Send + Sync {
    fn on_model(&self, model: PopoverModel);
}

#[derive(uniffi::Object)]
pub struct Perch {
    config_dir: PathBuf,
    db_path: PathBuf,
    handle: Mutex<Option<watcher::WatcherHandle>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Perch's own database. Never inside the Claude Code config dir.
fn app_data_db() -> Result<PathBuf, PerchError> {
    let home = std::env::var("HOME").map_err(|_| PerchError::Io { message: "HOME is not set".into() })?;
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(home).join("Library").join("Application Support").join("Perch");
    #[cfg(not(target_os = "macos"))]
    let base = PathBuf::from(home).join(".local").join("share").join("perch");
    Ok(base.join("index.db"))
}

#[uniffi::export]
impl Perch {
    /// `config_dir: None` resolves per spec §3 (CLAUDE_CONFIG_DIR → XDG → ~/.claude).
    #[uniffi::constructor]
    pub fn new(config_dir: Option<String>) -> Result<Arc<Self>, PerchError> {
        let dir = match config_dir {
            Some(p) => PathBuf::from(p),
            None => config::config_dir().ok_or(PerchError::NoConfigDir { path: "<unresolved>".into() })?,
        };
        if !dir.is_dir() {
            return Err(PerchError::NoConfigDir { path: dir.to_string_lossy().into_owned() });
        }
        Ok(Arc::new(Self { config_dir: dir, db_path: app_data_db()?, handle: Mutex::new(None) }))
    }

    /// Synchronous snapshot: sessions from disk, stats from the index if it opens.
    pub fn current(&self) -> PopoverModel {
        let sessions = live::live_sessions(&config::sessions_dir(&self.config_dir), &RealProcessProbe);
        self.model_for(sessions)
    }

    /// Re-index, emit, then keep emitting on every change and at least every 5 s.
    pub fn start(&self, listener: Arc<dyn PerchListener>) {
        let mut guard = self.handle.lock().unwrap();
        if guard.is_some() {
            return;
        }
        self.reindex();
        let me = ThisPerch { config_dir: self.config_dir.clone(), db_path: self.db_path.clone() };
        let cfg = watcher::WatcherConfig::for_dir(config::sessions_dir(&self.config_dir));
        let h = watcher::spawn(cfg, Arc::new(RealProcessProbe), move |sessions| {
            listener.on_model(me.model_for(sessions));
        });
        *guard = Some(h);
    }

    /// Re-index and emit now. Called when the menu opens.
    pub fn refresh(&self) {
        self.reindex();
        if let Some(h) = self.handle.lock().unwrap().as_ref() {
            h.refresh();
        }
    }

    pub fn stop(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            h.stop();
        }
    }
}

/// The parts of `Perch` the watcher closure needs, without holding an `Arc<Perch>`
/// (which would keep the object alive past the shell's last reference).
#[derive(Clone)]
struct ThisPerch {
    config_dir: PathBuf,
    db_path: PathBuf,
}

impl ThisPerch {
    fn model_for(&self, sessions: Vec<live::LiveSession>) -> PopoverModel {
        let db = db::open(&self.db_path).ok();
        core_model::build_model(db.as_ref(), &sessions, now_ms()).into()
    }
}

impl Perch {
    fn model_for(&self, sessions: Vec<live::LiveSession>) -> PopoverModel {
        ThisPerch { config_dir: self.config_dir.clone(), db_path: self.db_path.clone() }.model_for(sessions)
    }

    fn reindex(&self) {
        let Ok(database) = db::open(&self.db_path) else { return };
        let _ = pricing::seed_default_prices(&database);
        let _ = index::index_all(&database, &config::projects_dir(&self.config_dir));
    }
}
```

Add `thiserror.workspace = true` to `crates/perch-ffi/Cargo.toml` dependencies (it is already a workspace dependency).

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-ffi` → PASS, 2 tests.
Run: `source "$HOME/.cargo/env" && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Run: `source "$HOME/.cargo/env" && cargo build --release -p perch-ffi` → produces `target/release/libperch_ffi.a`.

Confirm the smoke test's config dir stayed untouched (the test asserts it). Confirm no network dependency was pulled in: `grep -E '^name = "(reqwest|hyper|ureq)"' Cargo.lock` must match only tauri's mobile-target entries that were already there.

```bash
git add crates/perch-ffi Cargo.toml Cargo.lock
git commit -m "feat(ffi): UniFFI surface — Perch object, listener, view-model records"
```

---

## Task 6: `scripts/build-xcframework.sh`

**Files:**
- Create: `scripts/build-xcframework.sh`
- Modify: `.gitignore` (add `apps/macos/Perch/Frameworks/` and `build/`)

**Interfaces:**
- Produces: `apps/macos/Perch/Frameworks/PerchCore.xcframework` (static, arm64 + x86_64 slices), and generated Swift sources at `apps/macos/Perch/Sources/PerchFFI/` (a SwiftPM target). The Swift module name is `PerchFFI`; the C module (modulemap) is `perch_ffiFFI`.

- [ ] **Step 1: Write the script**

Create `scripts/build-xcframework.sh`:

```bash
#!/usr/bin/env bash
# Build perch-ffi as a static library for macOS (arm64 + x86_64), generate the Swift
# bindings, and package an XCFramework the SwiftPM app links against.
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=release
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
OUT=apps/macos/Perch
FW="$OUT/Frameworks/PerchCore.xcframework"
GEN="$OUT/Sources/PerchFFI"
BUILD=build/xcframework
LIB=libperch_ffi.a

for t in "${TARGETS[@]}"; do rustup target add "$t" >/dev/null 2>&1 || true; done

for t in "${TARGETS[@]}"; do
  cargo build -p perch-ffi --lib --$PROFILE --target "$t"
done

rm -rf "$BUILD" "$FW" "$GEN"
mkdir -p "$BUILD/headers" "$GEN"

# Universal static library so one XCFramework slice serves both Mac architectures.
lipo -create \
  "target/aarch64-apple-darwin/$PROFILE/$LIB" \
  "target/x86_64-apple-darwin/$PROFILE/$LIB" \
  -output "$BUILD/$LIB"

# Bindings are generated FROM the compiled library, so they always match it.
cargo run -p perch-ffi --bin uniffi-bindgen-swift -- "$BUILD/$LIB" "$GEN" --swift-sources
cargo run -p perch-ffi --bin uniffi-bindgen-swift -- "$BUILD/$LIB" "$BUILD/headers" --headers
cargo run -p perch-ffi --bin uniffi-bindgen-swift -- "$BUILD/$LIB" "$BUILD/headers" \
  --xcframework --modulemap --modulemap-filename module.modulemap

xcodebuild -create-xcframework \
  -library "$BUILD/$LIB" -headers "$BUILD/headers" \
  -output "$FW"

echo "built $FW"
echo "swift sources in $GEN:"; ls "$GEN"
```

`chmod +x scripts/build-xcframework.sh`. Append to `.gitignore`:

```
build/
apps/macos/Perch/Frameworks/
apps/macos/Perch/Sources/PerchFFI/
apps/macos/Perch/.build/
```

The generated Swift sources are build output (regenerated from the library every time), so they are ignored rather than committed — a stale checked-in binding is a classic FFI bug.

- [ ] **Step 2: Run it**

Run: `source "$HOME/.cargo/env" && scripts/build-xcframework.sh`
Expected: ends with `built apps/macos/Perch/Frameworks/PerchCore.xcframework` and lists `perch_ffi.swift` in the sources dir. The x86_64 target compiles perch-core's `bundled` SQLite with the host clang — Xcode provides it. If `rustup target add x86_64-apple-darwin` needs a network download, that is expected once.

Verify the slice: `lipo -info build/xcframework/libperch_ffi.a` → `Architectures in the fat file: ... are: x86_64 arm64`.

- [ ] **Step 3: Commit**

```bash
git add scripts/build-xcframework.sh .gitignore
git commit -m "build: script to produce PerchCore.xcframework with Swift bindings"
```

---

## Task 7: SwiftPM app skeleton — status item, Quit, engine bridge

**Files:**
- Create: `apps/macos/Perch/Package.swift`, `apps/macos/Perch/Makefile`, `apps/macos/Perch/Resources/Info.plist`, `apps/macos/Perch/Sources/Perch/PerchApp.swift`, `apps/macos/Perch/Sources/Perch/Bridge/PerchEngine.swift`, `apps/macos/Perch/Sources/Perch/StatusItemController.swift`

**Interfaces:**
- Consumes: the `PerchFFI` Swift module (`Perch`, `PerchListener`, `PopoverModel`, `PerchError`).
- Produces: `PerchEngine` (`@MainActor final class`, `@Published var model: PopoverModel?`, `func start()`, `func refresh()`), `StatusItemController` with a menu containing only Quit for now, and a `make run` that launches the app with no Dock icon.

- [ ] **Step 1: Package manifest**

Create `apps/macos/Perch/Package.swift`:

```swift
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Perch",
    platforms: [.macOS(.v15)],
    targets: [
        // Static Rust core + C module map, produced by scripts/build-xcframework.sh.
        .binaryTarget(name: "PerchCore", path: "Frameworks/PerchCore.xcframework"),
        // Generated by uniffi-bindgen-swift; regenerated on every framework build.
        .target(
            name: "PerchFFI",
            dependencies: ["PerchCore"],
            path: "Sources/PerchFFI",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "Perch",
            dependencies: ["PerchFFI"],
            path: "Sources/Perch",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)
```

Generated UniFFI Swift is not yet strict-concurrency clean, hence language mode 5 for that target only.

- [ ] **Step 2: Info.plist and Makefile**

Create `apps/macos/Perch/Resources/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.perch.app</string>
  <key>CFBundleName</key><string>Perch</string>
  <key>CFBundleExecutable</key><string>Perch</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
```

`LSUIElement` is what keeps a bundled app out of the Dock; the code also sets the Accessory policy so `swift run` behaves the same.

Create `apps/macos/Perch/Makefile`:

```makefile
.PHONY: framework build run bundle clean

framework:
	../../../scripts/build-xcframework.sh

build: 
	swift build -c release

run: build
	./.build/release/Perch

# A minimal .app so LSUIElement and the bundle id apply.
bundle: build
	rm -rf build/Perch.app
	mkdir -p build/Perch.app/Contents/MacOS build/Perch.app/Contents/Resources
	cp .build/release/Perch build/Perch.app/Contents/MacOS/Perch
	cp Resources/Info.plist build/Perch.app/Contents/Info.plist
	@echo "open build/Perch.app"

clean:
	rm -rf .build build Frameworks Sources/PerchFFI
```

- [ ] **Step 3: The engine bridge**

Create `apps/macos/Perch/Sources/Perch/Bridge/PerchEngine.swift`:

```swift
import Foundation
import PerchFFI

/// Owns the Rust `Perch` object and republishes its model on the main actor.
/// Nothing in Swift reads Claude Code's files; this is the only doorway.
@MainActor
final class PerchEngine: ObservableObject {
    @Published private(set) var model: PopoverModel?
    @Published private(set) var startupError: String?

    private var perch: Perch?
    private var listener: Listener?

    func start() {
        do {
            let p = try Perch(configDir: nil)
            let l = Listener { [weak self] m in
                Task { @MainActor in self?.model = m }
            }
            perch = p
            listener = l
            model = p.current()
            p.start(listener: l)
        } catch {
            startupError = String(describing: error)
        }
    }

    func refresh() { perch?.refresh() }
    func stop() { perch?.stop() }
}

/// Rust calls this on its watcher thread; hop to the main actor before touching UI.
private final class Listener: PerchListener, @unchecked Sendable {
    private let deliver: @Sendable (PopoverModel) -> Void
    init(_ deliver: @escaping @Sendable (PopoverModel) -> Void) { self.deliver = deliver }
    func onModel(model: PopoverModel) { deliver(model) }
}
```

Generated names follow UniFFI's Swift conventions: `Perch(configDir:)`, `PerchListener.onModel(model:)`, records as structs with camelCase members. If a name differs in the generated `perch_ffi.swift`, use the generated name and record it in the report.

- [ ] **Step 4: Status item with Quit, and the app entry**

Create `apps/macos/Perch/Sources/Perch/StatusItemController.swift`:

```swift
import AppKit
import Combine

/// The tray item and its menu. A real NSMenu: while it is open, macOS is tracking a
/// menu, which is the only thing that keeps the menu bar visible over a fullscreen app.
@MainActor
final class StatusItemController: NSObject, NSMenuDelegate {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private let engine: PerchEngine
    private var cancellables = Set<AnyCancellable>()

    init(engine: PerchEngine) {
        self.engine = engine
        super.init()
        menu.delegate = self
        statusItem.menu = menu
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "bird", accessibilityDescription: "Perch")
            button.imagePosition = .imageLeading
        }
        engine.$model
            .receive(on: DispatchQueue.main)
            .sink { [weak self] in self?.render($0) }
            .store(in: &cancellables)
        render(nil)
    }

    func menuWillOpen(_ menu: NSMenu) {
        engine.refresh()
    }

    private func render(_ model: PopoverModel?) {
        statusItem.button?.title = model?.trayTitle ?? ""
        menu.removeAllItems()
        // Cards arrive in Task 8; for now the menu is just Quit.
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit Perch", action: #selector(quit), keyEquivalent: "q").target = self
    }

    @objc private func quit() { NSApp.terminate(nil) }
}
```

Create `apps/macos/Perch/Sources/Perch/PerchApp.swift`:

```swift
import AppKit

@main
enum PerchApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var engine: PerchEngine?
    private var status: StatusItemController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Menu-bar app: no Dock icon, no app switcher entry.
        NSApp.setActivationPolicy(.accessory)
        let engine = PerchEngine()
        self.engine = engine
        self.status = StatusItemController(engine: engine)
        engine.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        engine?.stop()
    }
}
```

- [ ] **Step 5: Build and hand-verify**

Run (own tool call): `cd apps/macos/Perch && swift build -c release 2>&1 | tail -5`
Expected: `Compiling Perch`, then `Build complete`. First build compiles the generated bindings; ~1 min.

Run: `cd apps/macos/Perch && make bundle && open build/Perch.app`
Verify and report: a bird icon appears in the menu bar with a session count as its title; no Dock icon; clicking opens a menu containing only "Quit Perch"; Quit exits. **With a fullscreen app frontmost, opening the menu keeps the menu bar visible** — that is the entire reason for the rewrite, and it should already hold with an empty menu.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Perch/Package.swift apps/macos/Perch/Makefile apps/macos/Perch/Resources apps/macos/Perch/Sources/Perch
git commit -m "feat(macos): SwiftPM menu-bar app skeleton over the Rust engine"
```

---

## Task 8: The cards — the richer popover

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/Cards/{StatsCard,SessionRow,RecentCard,EmptyCard}.swift`
- Modify: `apps/macos/Perch/Sources/Perch/StatusItemController.swift`

**Interfaces:**
- Consumes: `PopoverModel` and its records; `PerchEngine`.
- Produces: SwiftUI views hosted in menu items. Every string comes from the model; the views apply layout and colour only.

Visual spec: the approved dense mockup at `.superpowers/brainstorm/23622-1788104330/content/menubar.html` (option C), widened to ~360 pt and taller. Colours: ground uses the system menu material (no custom background — menus draw their own), text `.primary`/`.secondary`, status dots working `#30d158`, idle the same at 55 % opacity, waiting `#ffd60a`, background `#636366`; the `est` badge is a dashed amber capsule.

- [ ] **Step 1: The cards**

Create `apps/macos/Perch/Sources/Perch/Cards/StatsCard.swift`:

```swift
import SwiftUI
import PerchFFI

struct StatsCard: View {
    let stats: Stats
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            stat("Window", stats.windowTokens)
            stat("Week", stats.weekTokens)
            VStack(alignment: .leading, spacing: 2) {
                Text("24h").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
                Text(stats.dayCost).font(.title3.weight(.semibold)).monospacedDigit()
                if stats.hasData && stats.estimated {
                    Text("est")
                        .font(.caption2)
                        .foregroundStyle(Color(red: 1, green: 0.84, blue: 0.04))
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .overlay(Capsule().strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [3, 2]))
                            .foregroundStyle(Color(red: 1, green: 0.84, blue: 0.04)))
                }
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 10)
        .frame(width: 360, alignment: .leading)
    }
    private func stat(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            Text(value).font(.title3.weight(.semibold)).monospacedDigit()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
```

Create `apps/macos/Perch/Sources/Perch/Cards/SessionRow.swift`:

```swift
import SwiftUI
import PerchFFI

struct SessionRowView: View {
    let row: SessionRow
    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Circle().fill(dotColor).frame(width: 8, height: 8).padding(.top, 5)
            VStack(alignment: .leading, spacing: 1) {
                HStack {
                    Text(row.name).fontWeight(.medium).lineLimit(1)
                    Spacer()
                    Text(row.elapsed).font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
                Text(row.statusLabel)
                    .font(.caption)
                    .foregroundStyle(row.status == .waiting ? Color(red: 1, green: 0.84, blue: 0.04) : .secondary)
                Text([row.project, row.kind, row.version.isEmpty ? nil : "v\(row.version)", row.tokens == "—" ? nil : "\(row.tokens) · \(row.cost)"]
                        .compactMap { $0 }.joined(separator: " · "))
                    .font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .frame(width: 360, alignment: .leading)
    }
    private var dotColor: Color {
        switch row.status {
        case .waiting: Color(red: 1, green: 0.84, blue: 0.04)
        case .working: Color(red: 0.19, green: 0.82, blue: 0.35)
        case .idle: Color(red: 0.19, green: 0.82, blue: 0.35).opacity(0.55)
        case .background: Color(red: 0.39, green: 0.39, blue: 0.40)
        }
    }
}
```

Create `apps/macos/Perch/Sources/Perch/Cards/RecentCard.swift`:

```swift
import SwiftUI
import PerchFFI

struct RecentCard: View {
    let rows: [RecentRow]
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Recent").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            ForEach(rows, id: \.id) { r in
                HStack {
                    Circle().fill(Color(red: 0.39, green: 0.39, blue: 0.40)).frame(width: 8, height: 8)
                    Text(r.project).lineLimit(1)
                    Spacer()
                    Text(r.tokens == "—" ? "ended \(r.endedAgo) ago" : "\(r.tokens) · ended \(r.endedAgo) ago")
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .frame(width: 360, alignment: .leading)
    }
}
```

Create `apps/macos/Perch/Sources/Perch/Cards/EmptyCard.swift`:

```swift
import SwiftUI

struct EmptyCard: View {
    let title: String
    let detail: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).foregroundStyle(.secondary)
            if let detail { Text(detail).font(.caption2).foregroundStyle(.tertiary).lineLimit(2) }
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .frame(width: 360, alignment: .leading)
    }
}
```

- [ ] **Step 2: Host the cards in the menu**

Replace `render(_:)` in `StatusItemController.swift`:

```swift
    private func render(_ model: PopoverModel?) {
        statusItem.button?.title = model?.trayTitle ?? ""
        menu.removeAllItems()

        guard let model else {
            add(EmptyCard(title: engine.startupError ?? "Starting…", detail: nil))
            addQuit()
            return
        }

        add(StatsCard(stats: model.stats))
        if let err = model.error { add(EmptyCard(title: "Index unavailable", detail: err)) }
        menu.addItem(.separator())

        let waiting = model.live.filter { $0.status == .waiting }.count
        if waiting > 0 {
            add(EmptyCard(title: waiting == 1 ? "1 session is waiting on you" : "\(waiting) sessions are waiting on you", detail: nil))
        }
        if model.live.isEmpty {
            add(EmptyCard(title: "No sessions running", detail: nil))
        } else {
            for row in model.live { add(SessionRowView(row: row)) }
        }
        if !model.recent.isEmpty {
            menu.addItem(.separator())
            add(RecentCard(rows: model.recent))
        }
        menu.addItem(.separator())
        addQuit()
    }

    private func add<V: View>(_ view: V) {
        let item = NSMenuItem()
        let host = NSHostingView(rootView: view)
        host.frame.size = host.fittingSize
        item.view = host
        menu.addItem(item)
    }

    private func addQuit() {
        menu.addItem(withTitle: "Quit Perch", action: #selector(quit), keyEquivalent: "q").target = self
    }
```

Add `import SwiftUI` at the top of the file. The waiting banner is an `EmptyCard` restyled inline; if a distinct amber band is wanted, give `EmptyCard` an optional `tint` — but only after the hand-test, not speculatively.

- [ ] **Step 3: Build and hand-verify (the milestone gate)**

Run (own tool call): `cd apps/macos/Perch && swift build -c release 2>&1 | tail -3`
Run: `cd apps/macos/Perch && make bundle && open build/Perch.app`

Verify and report each:
1. **Menu bar stays visible with the menu open over a fullscreen app** — the defining check.
2. Stats card shows three values, third labelled `24h` with the dashed amber `est` badge; `—` if the index is empty.
3. Each live session row shows name · elapsed on line 1, status label on line 2, project · kind · version · tokens · cost on line 3; blocked sessions first with an amber label.
4. Idle sessions show a dim dot and "idle".
5. Recent section lists up to three ended sessions with "ended Xh ago".
6. Escape closes the menu; clicking away closes it; clicking the icon again closes it.
7. Live update: start or quit a `claude` session with the menu **closed**, reopen — the list reflects it. (An open `NSMenu` is modal; it re-renders on next open. That is standard menu behaviour and acceptable — record it as a known difference from the Tauri popover.)
8. No Dock icon.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Perch/Sources/Perch
git commit -m "feat(macos): stats, session, and recent cards hosted in the status menu"
```

---

## Task 9: CI — build the framework and the app; audit the new crate

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: a `native-app` job (macOS) that runs the xcframework script and `swift build`; the `no-network` job's globs extended to `crates/perch-ffi`.

- [ ] **Step 1: Extend the audits**

In the `no-network` job, every glob that lists `crates/*/Cargo.toml` already covers `crates/perch-ffi/Cargo.toml`, and `crates/*/src/**` covers its sources — **verify by reading the job**, and add `apps/macos/Perch/Sources/Perch/**/*.swift` to the source-audit grep with patterns `URLSession`, `NSURLConnection`, `Network.framework`, `NWConnection`, `https?://`. Add a `/tmp` positive-control self-test for the Swift pattern in the same style as the others. Exclude `Sources/PerchFFI/` (generated, ignored) from the glob.

- [ ] **Step 2: The build job**

Add:

```yaml
  native-app:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin, x86_64-apple-darwin
      - uses: Swatinem/rust-cache@v2
      - run: scripts/build-xcframework.sh
      - run: swift build -c release
        working-directory: apps/macos/Perch
```

- [ ] **Step 3: Verify locally**

Hand-run the `no-network` script blocks on the real tree with the macOS scope; confirm all pass and the new Swift self-test fires. Then `scripts/build-xcframework.sh && (cd apps/macos/Perch && swift build -c release)` once more from clean (`make clean` first) to prove the job's exact sequence works from nothing.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: build the native macOS app and audit its sources and the ffi crate"
```

---

## Task 10: README and backlog

**Files:**
- Modify: `README.md`, `docs/BACKLOG.md`

- [ ] **Step 1: README**

Add a "Native app (macOS 15+)" section above the Tauri section: how to build (`scripts/build-xcframework.sh`, then `cd apps/macos/Perch && make bundle`), that the engine is Rust linked via UniFFI, that the menu is a real `NSMenu` (which is why the menu bar stays visible over fullscreen apps), and that the Tauri app remains until the native app reaches parity. Keep the privacy section's claims true: the native app makes no requests; the "one outbound host" line describes a future release — say so.

- [ ] **Step 2: Backlog**

In `docs/BACKLOG.md`, move "Menu bar stays visible", "Richer session rows", "Recent section", "Taller popover", and "Re-index on show" from **Now** to **Shipped**, and add under **Next — settings & polish**: "Remove the Tauri app and React frontend once the native app has jump/resume parity."

- [ ] **Step 3: Commit**

```bash
git add README.md docs/BACKLOG.md
git commit -m "docs: document the native app; update the backlog"
```

---

## Self-Review

**Spec coverage.** §2 richer popover → Tasks 3 (model) and 8 (cards). §3 architecture — `ui` module → Tasks 1, 3, 4; `perch-ffi` → Task 5; xcframework script → Task 6; SwiftPM app → Tasks 7–8. §4 data flow (start → reindex → emit → watch; menu open → refresh) → Tasks 5, 7. §5 error handling (no config dir → typed error → EmptyCard; db failure → sessions-only with dashes) → Tasks 3, 5, 8. §6 testing → Rust tests in Tasks 1–5, FFI smoke test in Task 5, CI in Task 9, hand-test in Task 8. §7 cross-platform → the `ui` and `ffi` crates carry no `cfg(macos)` except the app-data path.

**Known deviation, recorded:** an open `NSMenu` is modal and does not live-update while open; it re-renders on the next open (Task 8 check 7). The Tauri popover updated in place. Spec §4 said "each change → callback → rebuild"; that still happens — the rebuild is visible on next open. If in-place updating of an open menu is ever required, `NSMenu` item views can be updated while the menu is tracking (they are ordinary views), which is a follow-on, not a blocker.

**Type consistency checked:** `PopoverModel`/`Stats`/`SessionRow`/`RecentRow`/`Status` field names are identical across `perch-core::ui::model` (Task 3), `perch-ffi` (Task 5), and the generated Swift (camelCased by UniFFI: `windowTokens`, `statusLabel`, `endedAgo`, `trayTitle`, `hasData`) as used in Task 8. `WatcherHandle::{refresh, stop}` (Task 4) are what `Perch::{refresh, stop}` call (Task 5). `query::{session_usage, recent_sessions}` (Task 2) are what `build_model` calls (Task 3).

**Placeholder scan:** none. Every step has its code or its exact command.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks.
2. **Inline Execution** — tasks executed in this session with batch checkpoints.
