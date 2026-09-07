# Perch Settings, Notifications, and Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Perch settings that matter, the waiting-on-you notification the project was started to solve, and a diagnostics view that answers "why isn't my session showing?" — with global defaults in a small settings window and per-project overrides on the project itself.

**Architecture:** `perch-core` gains `settings` (a versioned, watched, format-preserving `config.toml`), `notify` (the edge-triggered notification decision, pure and heavily tested), and two view-models. `perch-ffi` exposes them. Swift adds a settings window, a per-project override control in the detail pane, and a `Notifier` that does nothing but hand finished strings to `UNUserNotificationCenter`.

**Tech Stack:** Rust 1.98, `toml_edit` 0.25 (format-preserving; the crate Cargo uses), `notify` 8, rusqlite, `uniffi` 0.32, Swift 6.3 / SwiftPM, AppKit + SwiftUI, `SMAppService` and `UNUserNotificationCenter` (both in the SDK), macOS 15+.

**Spec:** `docs/superpowers/specs/2026-09-06-settings-and-notifications-design.md` — implements §2–§10. The 2026-08-30 spec's §9.5 and the two prior design docs remain binding.

## Global Constraints

- **Rust owns everything except drawing.** Settings validation, defaults, migration, the notification *decision*, and every displayed string live in `perch-core`. Swift lays out, draws, and delivers. A `String(format:)`, `NumberFormatter`, or comparison against a Rust sentinel in Swift is a defect.
- **Read-only** against the user's Claude Code directory. Perch writes only inside its own application-support directory.
- **No network, no telemetry.** The only new dependency permitted by this plan is `toml_edit`; **no other Cargo crate and no SPM package.** `SMAppService` and `UNUserNotificationCenter` ship in the SDK.
- `perch-core` and `perch-ffi` must keep compiling on Linux. Only the default config path may be OS-conditional, behind the same seam the app-data path already uses.
- Rust edition 2021; `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass. **164 tests pass before this plan starts.**
- The retained Tauri app (`src-tauri`, `src/`) stays untouched and must keep building (`cargo build -p perch-app`).
- Generated output (`apps/macos/Perch/Sources/PerchFFI/`, `Frameworks/`) is git-ignored and must never be committed. Re-run `scripts/build-xcframework.sh` after any FFI change.
- **Never read `apps/macos/Perch/Sources/PerchFFI/perch_ffi.swift`** — 49 KB of generated code that has stalled agents. The controller supplies the API doc.
- Run every command in the foreground; keep tool output small.
- **A setting that does not take effect is worse than no setting.** Task 10 exists to wire every one of them; do not consider a setting done because it round-trips to disk.

---

## File Structure

```
crates/perch-core/src/
├── settings/
│   ├── mod.rs         Settings + Defaults + validate + migrate      (Task 1)
│   └── store.rs       path resolution, load/save, watching          (Task 2)
├── notify.rs          NotificationDecision — edge-triggered, pure    (Task 4)
├── db.rs              + per-project notify override                 (Task 3)
└── ui/
    ├── settings.rs    SettingsModel                                 (Task 5)
    └── diagnostics.rs DiagnosticsModel                              (Task 5)

crates/perch-ffi/src/lib.rs                                          (Task 6)

apps/macos/Perch/Sources/Perch/
├── Settings/SettingsWindow.swift                                    (Task 7)
├── Settings/DiagnosticsView.swift                                   (Task 7)
├── MainWindow/ProjectDetail.swift  + override control               (Task 8)
├── Notifications/Notifier.swift                                     (Task 9)
└── (wiring across PerchApp / StatusItemController / Launcher)       (Task 10)

.github/workflows/ci.yml, README.md, docs/BACKLOG.md                 (Task 11)
```

---

## Task 1: `settings` — the typed struct, defaults, and validation

**Files:**
- Create: `crates/perch-core/src/settings/mod.rs`
- Modify: `crates/perch-core/src/lib.rs`, root `Cargo.toml`, `crates/perch-core/Cargo.toml`

**Interfaces:**
- Produces:

```rust
pub const SETTINGS_VERSION: i64 = 1;

pub enum MenuBarDisplay { Icon, Count, CountAndWaiting }

pub struct Settings {
    pub launch_at_login: bool,
    pub claude_config_dir: String,     // "" = auto-detect
    pub menu_bar_display: MenuBarDisplay,
    pub poll_seconds: u32,             // 1..=60
    pub waiting_enabled: bool,
    pub waiting_after_minutes: u32,    // 1..=240
    pub include_background: bool,
    pub preferred_terminal: String,    // "Terminal" | "iTerm2" | ...
}

impl Default for Settings { /* the spec §4 defaults */ }
impl Settings {
    /// Clamps every out-of-range value to its bound and reports what it changed,
    /// so a hand-edited file is never silently ignored *or* silently obeyed.
    pub fn validated(self) -> (Settings, Vec<String>);
}
```

`Settings` derives `Serialize`/`Deserialize` with `#[serde(default)]` on every field, so a file missing a key gets that key's default rather than failing to parse — a config file must tolerate being older than the binary.

- [ ] **Step 1: Add the dependency**

Root `Cargo.toml` `[workspace.dependencies]`: `toml_edit = { version = "0.25", features = ["serde"] }`.
`crates/perch-core/Cargo.toml` `[dependencies]`: `toml_edit.workspace = true`.

Then confirm the dependency is inert with respect to the network audit:
Run: `source "$HOME/.cargo/env" && cargo tree -p perch-core | grep -iE "reqwest|hyper|ureq|tokio" | head` — expect no output. Paste it into your report.

- [ ] **Step 2: Write the failing tests**

Create `crates/perch-core/src/settings/mod.rs`:

```rust
//! Perch's own settings. Every value here replaces something that used to be a
//! constant, and every one of them is validated in Rust so a hand-edited file
//! behaves the same way whichever shell is reading it.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_ones() {
        let s = Settings::default();
        assert!(!s.launch_at_login);
        assert_eq!(s.claude_config_dir, "");
        assert_eq!(s.menu_bar_display, MenuBarDisplay::Count);
        assert_eq!(s.poll_seconds, 5);
        assert!(!s.waiting_enabled, "notifications are off until asked for");
        assert_eq!(s.waiting_after_minutes, 10);
        assert!(!s.include_background, "a bg session is not waiting on you");
        assert_eq!(s.preferred_terminal, "Terminal");
    }

    #[test]
    fn out_of_range_values_are_clamped_and_reported() {
        let s = Settings { poll_seconds: 0, waiting_after_minutes: 9999, ..Default::default() };
        let (s, notes) = s.validated();
        assert_eq!(s.poll_seconds, 1, "clamped to the floor, not reset to the default");
        assert_eq!(s.waiting_after_minutes, 240);
        assert_eq!(notes.len(), 2, "each change is reported so the edit is not silently obeyed");
        assert!(notes.iter().any(|n| n.contains("poll_seconds")));
        assert!(notes.iter().any(|n| n.contains("waiting_after_minutes")));
    }

    #[test]
    fn in_range_values_are_left_alone_and_report_nothing() {
        let s = Settings { poll_seconds: 30, waiting_after_minutes: 60, ..Default::default() };
        let (s, notes) = s.validated();
        assert_eq!(s.poll_seconds, 30);
        assert_eq!(s.waiting_after_minutes, 60);
        assert!(notes.is_empty());
    }

    #[test]
    fn a_missing_key_takes_its_default_rather_than_failing() {
        // A file written by an older Perch will not have every key.
        let partial = r#"
            version = 1
            [sessions]
            poll_seconds = 9
        "#;
        let s: Settings = toml_edit::de::from_str(partial).expect("partial file must parse");
        assert_eq!(s.poll_seconds, 9, "the key that is present is honoured");
        assert_eq!(s.waiting_after_minutes, 10, "the ones that are absent take defaults");
    }

    #[test]
    fn menu_bar_display_round_trips_through_its_wire_names() {
        for (v, wire) in [
            (MenuBarDisplay::Icon, "icon"),
            (MenuBarDisplay::Count, "count"),
            (MenuBarDisplay::CountAndWaiting, "count-and-waiting"),
        ] {
            let doc = format!("[menu_bar]\ndisplay = \"{wire}\"\n");
            let s: Settings = toml_edit::de::from_str(&doc).unwrap();
            assert_eq!(s.menu_bar_display, v, "{wire} must map to {v:?}");
        }
    }

    #[test]
    fn an_unknown_display_value_falls_back_rather_than_failing_the_whole_file() {
        let s: Settings = toml_edit::de::from_str("[menu_bar]\ndisplay = \"nonsense\"\n")
            .expect("one bad enum value must not cost the user every other setting");
        assert_eq!(s.menu_bar_display, MenuBarDisplay::Count);
    }
}
```

Note the serde shape the tests imply: the struct is flattened across `[general]`, `[menu_bar]`, `[sessions]`, and `[notifications]` tables. Implement it with nested `#[serde(default)]` section structs internally if that is cleaner than field-level attributes — the public `Settings` shape above is what matters.

- [ ] **Step 3: Register and run to see them fail**

Add `pub mod settings;` to `crates/perch-core/src/lib.rs`.
Run: `source "$HOME/.cargo/env" && cargo test -p perch-core settings`
Expected: FAIL — `cannot find type Settings`.

- [ ] **Step 4: Implement**

Write `Settings`, `MenuBarDisplay`, `Default`, and `validated()`. `MenuBarDisplay` uses `#[serde(rename_all = "kebab-case")]` and a `#[serde(other)]`-style fallback so an unknown value degrades to `Count` rather than failing the file.

`validated()` clamps rather than rejects, and returns a human-readable note per change (`"poll_seconds was 0; clamped to 1"`). Clamping keeps the app running on a bad edit; reporting keeps the user informed. Both matter — silently obeying a nonsense value and silently discarding it are equally bad.

- [ ] **Step 5: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo build -p perch-app`

```bash
git add crates/perch-core/src/settings crates/perch-core/src/lib.rs Cargo.toml Cargo.lock crates/perch-core/Cargo.toml
git commit -m "feat(core): settings struct with defaults and clamping validation"
```

---

## Task 2: `settings::store` — path, load, save, migrate, watch

**Files:**
- Create: `crates/perch-core/src/settings/store.rs`
- Modify: `crates/perch-core/src/settings/mod.rs`

**Interfaces:**
- Consumes: `Settings`, `SETTINGS_VERSION`, `ui::watcher`'s pattern.
- Produces:

```rust
pub fn config_path() -> anyhow::Result<PathBuf>;     // PERCH_CONFIG, else data dir + config.toml
pub struct Loaded { pub settings: Settings, pub notes: Vec<String>, pub error: Option<String> }
pub fn load(path: &Path) -> Loaded;                   // never fails: defaults + error on trouble
pub fn save(path: &Path, s: &Settings) -> anyhow::Result<()>;  // format-preserving
pub fn watch<F: Fn() + Send + 'static>(path: PathBuf, on_change: F) -> WatchHandle;
```

**Format preservation is the point of `toml_edit`.** `save` parses the existing file into a `DocumentMut`, assigns only the keys Perch owns, and writes it back — so comments, key order, and unknown keys survive exactly. When the file does not exist, it is created from a template carrying the header comment and every key at its default, so the file documents itself.

**Migration mirrors the database's shape** (`db::migrate`): read `version`, run a one-shot upgrade when it is below `SETTINGS_VERSION`, and never destroy what the user owns. There is nothing to migrate at version 1 — the primitive exists so the first real change is not a special case. Say so in a comment, as `db::migrate` does.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_loads_defaults_without_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let l = load(&tmp.path().join("nope.toml"));
        assert_eq!(l.settings.poll_seconds, 5);
        assert!(l.error.is_none(), "absence is not a failure — it is a first run");
    }

    #[test]
    fn a_malformed_file_yields_defaults_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, "this is not = = toml").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.poll_seconds, 5, "the app still runs");
        let err = l.error.expect("a hand-edited file that did not parse must say so");
        assert!(err.contains("config.toml") || err.contains("parse"), "actionable: {err}");
    }

    #[test]
    fn out_of_range_values_load_clamped_with_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, "[sessions]\npoll_seconds = 0\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.poll_seconds, 1);
        assert_eq!(l.notes.len(), 1);
    }

    #[test]
    fn saving_preserves_comments_key_order_and_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, concat!(
            "# my own note about this file\n",
            "version = 1\n",
            "\n",
            "[sessions]\n",
            "# I like a slow poll\n",
            "poll_seconds = 30\n",
            "something_a_newer_perch_added = true\n",
        )).unwrap();

        let mut s = load(&p).settings;
        s.poll_seconds = 12;
        save(&p, &s).unwrap();

        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("# my own note about this file"), "comments survive");
        assert!(after.contains("# I like a slow poll"), "inline comments survive");
        assert!(after.contains("something_a_newer_perch_added = true"),
                "a key this binary does not know must not be eaten");
        assert!(after.contains("poll_seconds = 12"), "and the change lands");
    }

    #[test]
    fn a_created_file_documents_itself_and_reloads_identically() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        save(&p, &Settings::default()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("#"), "the file opens with a header comment");
        assert!(text.contains("poll_seconds"), "every key is present, not just non-defaults");
        assert_eq!(load(&p).settings, Settings::default(), "round trip is exact");
    }

    #[test]
    fn perch_config_overrides_the_default_path() {
        let tmp = tempfile::tempdir().unwrap();
        let want = tmp.path().join("elsewhere.toml");
        std::env::set_var("PERCH_CONFIG", &want);
        let got = config_path().unwrap();
        std::env::remove_var("PERCH_CONFIG");
        assert_eq!(got, want);
    }
}
```

The `PERCH_CONFIG` test mutates the process environment. `perch-ffi` already has an `ENV_LOCK` for exactly this; add the same guard here (a `static ENV_LOCK: Mutex<()>` in the test module) so it cannot race the other tests in this crate — Rust runs a crate's tests in parallel threads within one process.

- [ ] **Step 2: Run to see them fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core settings::store`
Expected: FAIL — `cannot find function load`.

- [ ] **Step 3: Implement**

`config_path()` checks `PERCH_CONFIG` first, then falls back to the app-data directory beside `index.db`. Reuse the existing app-data resolution rather than writing a second one; if it currently lives in `perch-ffi`, move it into `perch-core` and have the FFI call it — say in your report which you did.

`load` never returns `Err`: a missing file is defaults; a malformed one is defaults plus an error string naming the path; an out-of-range value is clamped with a note. The app must always start.

`save` uses `toml_edit::DocumentMut`, assigning each owned key in place.

`watch` reuses the debounce-plus-poll-backstop shape from `ui::watcher`. It watches a *file*, which on some platforms is replaced rather than modified by an editor — so watch the parent directory and filter, rather than watching the file inode, and note why in a comment.

- [ ] **Step 4: Verify and commit**

Run: `source "$HOME/.cargo/env" && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/settings
git commit -m "feat(core): settings store — path, format-preserving save, watching"
```

---

## Task 3: Per-project notification override

**Files:**
- Modify: `crates/perch-core/src/db.rs`

**Interfaces:**
- Produces:

```rust
pub enum NotifyOverride { Default, Off, Custom { after_minutes: u32 } }
// ProjectMeta gains: pub notify: NotifyOverride
pub fn set_notify_override(&self, project_id: i64, o: &NotifyOverride) -> Result<()>;
```

Stored as two columns on `projects`, **below the user-owned line** so re-indexing never touches
them: `notify_mode TEXT NOT NULL DEFAULT 'default'` and `notify_after_minutes INTEGER`.

**Migration to `SCHEMA_VERSION = 3`**, in the shape `db::migrate` already uses: guard each
`ALTER TABLE` with a `pragma_table_info` existence check so it is idempotent on a fresh
database. **Unlike the v2 migration, this one must not reset offsets or delete turns** — no
re-scan is needed, and the v2 comment already warns that the reset is only safe where it sits.

- [ ] **Step 1: Write the failing tests**

```rust
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
        db.set_notify_override(id, &NotifyOverride::Custom { after_minutes: 20 }).unwrap();
        db.set_note(id, "keep me").unwrap();

        assert_eq!(db.upsert_project("-a-b", "/a/b", false).unwrap(), id);

        let m = db.project_meta(id).unwrap();
        assert_eq!(m.notify, NotifyOverride::Custom { after_minutes: 20 });
        assert_eq!(m.note.as_deref(), Some("keep me"));
    }

    #[test]
    fn the_v3_migration_adds_the_columns_without_touching_indexed_data() {
        // A v2-shaped database with real turns: unlike v2, this migration must
        // not force a re-scan, so nothing indexed may be lost.
        let db = open_in_memory().unwrap();
        let pid = db.upsert_project("-a-b", "/a/b", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "s1".into(), project_id: pid, file_path: "/tmp/s1.jsonl".into(), file_size: 0,
            indexed_offset: 777, started_at: Some(1), last_activity_at: Some(2),
            cwd: None, git_branch: None, cc_version: None, message_count: 1, title: None,
        }).unwrap();
        db.insert_turns("s1", &[Turn { ts: 2, model: "m".into(), usage: TurnUsage::default() }]).unwrap();

        assert_eq!(db.turn_count().unwrap(), 1, "turns survive a v3 migration");
        assert_eq!(db.session_offset("s1").unwrap(), 777, "offsets are not reset");
        assert_eq!(db.project_meta(pid).unwrap().notify, NotifyOverride::Default);
    }
```

The last test constructs the database through the normal `open_in_memory` path, which already
runs every migration — it asserts the *absence* of the v2 reset behaviour at v3, which is the
regression that would matter. If you can build a genuinely v2-stamped database as the v2 test
does, prefer that; say which you did.

- [ ] **Step 2: Run to see them fail, then implement, then verify**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core db` → FAIL, then implement, then PASS.

`NotifyOverride` maps to the columns as: `Default → ('default', NULL)`, `Off → ('off', NULL)`,
`Custom{n} → ('custom', n)`. Reading tolerates an unknown mode string by falling back to
`Default` — a newer Perch's mode must not break an older one.

Run: `source "$HOME/.cargo/env" && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo build -p perch-app`

```bash
git add crates/perch-core/src/db.rs
git commit -m "feat(core): per-project notification override, v3 migration"
```

---

## Task 4: `notify` — the edge-triggered decision

**Files:**
- Create: `crates/perch-core/src/notify.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `live::{LiveSession, SessionStatus}`, `settings::Settings`, `db::NotifyOverride`.
- Produces:

```rust
/// One session's blocked stretch. A session that blocks, resumes, and blocks
/// again is a new episode — the key comes from data Claude Code owns, so it
/// survives a Perch restart without re-notifying.
pub struct Episode { pub session_id: String, pub waiting_since: i64 }

pub struct Notification { pub session_id: String, pub project: String,
                          pub title: String, pub body: String }

/// Pure: same inputs, same answer. `already_notified` is the caller's memory of
/// episodes it has fired for; the returned set replaces it.
pub fn decide(
    live: &[LiveSession],
    settings: &Settings,
    override_for: &dyn Fn(&str) -> NotifyOverride,   // by session cwd
    already_notified: &HashSet<Episode>,
    now_ms: i64,
) -> (Vec<Notification>, HashSet<Episode>);
```

**This is the most testable part of the milestone and gets the most tests.** The whole value of
the feature is that it fires exactly once at the right moment; every way that can go wrong is a
test below.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn waiting(id: &str, cwd: &str, since_ms: i64, kind: &str) -> LiveSession {
        LiveSession {
            pid: 1, session_id: id.into(), cwd: cwd.into(), name: id.into(), kind: kind.into(),
            status: SessionStatus::Waiting { reason: Some("dialog open".into()), since_ms },
            started_at: 0, status_updated_at: since_ms, cc_version: None, socket_path: None,
        }
    }
    fn working(id: &str, cwd: &str) -> LiveSession {
        LiveSession { status: SessionStatus::Working, ..waiting(id, cwd, 0, "interactive") }
    }
    fn on() -> Settings { Settings { waiting_enabled: true, ..Default::default() } }
    fn no_override() -> impl Fn(&str) -> NotifyOverride { |_| NotifyOverride::Default }

    #[test]
    fn nothing_fires_while_the_feature_is_off() {
        let s = Settings { waiting_enabled: false, ..Default::default() };
        let (n, _) = decide(&[waiting("a", "/p", 0, "interactive")], &s, &no_override(),
                            &HashSet::new(), 60 * MIN);
        assert!(n.is_empty());
    }

    #[test]
    fn nothing_fires_before_the_threshold() {
        let (n, seen) = decide(&[waiting("a", "/p", 0, "interactive")], &on(), &no_override(),
                               &HashSet::new(), 9 * MIN);
        assert!(n.is_empty(), "9 minutes is under the 10-minute default");
        assert!(seen.is_empty(), "and nothing is remembered yet");
    }

    #[test]
    fn it_fires_once_at_the_threshold_and_not_again_while_still_waiting() {
        let live = [waiting("a", "/p", 0, "interactive")];
        let (n, seen) = decide(&live, &on(), &no_override(), &HashSet::new(), 10 * MIN);
        assert_eq!(n.len(), 1, "fires when the threshold is crossed");
        assert!(n[0].body.contains("dialog open"), "the reason is in the message: {}", n[0].body);

        let (again, seen2) = decide(&live, &on(), &no_override(), &seen, 30 * MIN);
        assert!(again.is_empty(), "level-triggering here would notify every tick");
        assert_eq!(seen2, seen, "and the memory is unchanged");
    }

    #[test]
    fn a_session_that_resumes_and_blocks_again_is_a_new_episode() {
        let (_, seen) = decide(&[waiting("a", "/p", 0, "interactive")], &on(), &no_override(),
                               &HashSet::new(), 10 * MIN);
        // It goes back to work: the old episode is forgotten.
        let (_, seen) = decide(&[working("a", "/p")], &on(), &no_override(), &seen, 20 * MIN);
        assert!(seen.is_empty(), "an episode ends when the session works again");
        // Blocks again, later.
        let (n, _) = decide(&[waiting("a", "/p", 30 * MIN, "interactive")], &on(), &no_override(),
                            &seen, 41 * MIN);
        assert_eq!(n.len(), 1, "a genuinely new block deserves a new alert");
    }

    #[test]
    fn a_vanished_session_is_forgotten_so_the_memory_cannot_grow_forever() {
        let (_, seen) = decide(&[waiting("a", "/p", 0, "interactive")], &on(), &no_override(),
                               &HashSet::new(), 10 * MIN);
        let (_, seen) = decide(&[], &on(), &no_override(), &seen, 20 * MIN);
        assert!(seen.is_empty(), "the session is gone; its episode goes with it");
    }

    #[test]
    fn background_sessions_are_excluded_unless_asked_for() {
        let bg = [waiting("a", "/p", 0, "bg")];
        let (n, _) = decide(&bg, &on(), &no_override(), &HashSet::new(), 60 * MIN);
        assert!(n.is_empty(), "a bg session is not waiting on *you*");

        let s = Settings { include_background: true, ..on() };
        let (n, _) = decide(&bg, &s, &no_override(), &HashSet::new(), 60 * MIN);
        assert_eq!(n.len(), 1, "unless you say otherwise");
    }

    #[test]
    fn a_project_set_to_off_never_notifies() {
        let off = |cwd: &str| if cwd == "/quiet" { NotifyOverride::Off } else { NotifyOverride::Default };
        let live = [waiting("a", "/quiet", 0, "interactive"), waiting("b", "/loud", 0, "interactive")];
        let (n, _) = decide(&live, &on(), &off, &HashSet::new(), 60 * MIN);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].session_id, "b", "only the project that did not opt out");
    }

    #[test]
    fn a_project_with_a_custom_threshold_uses_its_own() {
        let custom = |cwd: &str| if cwd == "/slow" { NotifyOverride::Custom { after_minutes: 45 } }
                                 else { NotifyOverride::Default };
        let live = [waiting("a", "/slow", 0, "interactive")];
        let (n, _) = decide(&live, &on(), &custom, &HashSet::new(), 20 * MIN);
        assert!(n.is_empty(), "20 minutes is under this project's own 45");
        let (n, _) = decide(&live, &on(), &custom, &HashSet::new(), 46 * MIN);
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn the_message_is_finished_in_rust() {
        let (n, _) = decide(&[waiting("a", "/Users/x/my-proj", 0, "interactive")], &on(),
                            &no_override(), &HashSet::new(), 12 * MIN);
        let m = &n[0];
        assert_eq!(m.project, "my-proj", "the directory name, as everywhere else");
        assert!(!m.title.is_empty() && !m.body.is_empty());
        assert!(m.body.contains("12m") || m.body.contains("12"), "how long, already formatted: {}", m.body);
    }
}
```

- [ ] **Step 2: Register, run to see them fail, implement, verify**

Add `pub mod notify;` to `crates/perch-core/src/lib.rs`.

The implementation is a fold over the live sessions: skip when disabled, skip `bg` unless
included, resolve the effective threshold (project override beats global), compare
`now_ms - waiting_since` against it, and emit only for episodes not already in the incoming
set. The returned set contains exactly the episodes still live and waiting — which is what makes
a vanished or resumed session forget itself, and keeps the memory bounded.

Message strings are composed here (`human_elapsed` for the duration, the directory name for the
project), never in a shell.

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core notify && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/perch-core/src/notify.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): edge-triggered waiting-on-you notification decision"
```

---

## Task 5: `ui::settings` and `ui::diagnostics` view-models

**Files:**
- Create: `crates/perch-core/src/ui/settings.rs`, `crates/perch-core/src/ui/diagnostics.rs`
- Modify: `crates/perch-core/src/ui/mod.rs`

**Interfaces:**

```rust
// ui::settings
pub struct SettingsModel { pub settings: Settings, pub config_path: String,
                           pub notes: Vec<String>, pub error: Option<String> }
pub fn build_settings(path: &Path) -> SettingsModel;

// ui::diagnostics
pub enum RecordVerdict { Accepted, NoSuchProcess, NotClaude { actual: String }, Unparsable }
pub struct RecordRow { pub file: String, pub pid: i32, pub session_id: String,
                       pub verdict: RecordVerdict, pub verdict_label: String }
pub struct DiagnosticsModel {
    pub config_dir: String, pub config_dir_source: String,   // "CLAUDE_CONFIG_DIR" | "XDG" | "default" | "setting"
    pub sessions_dir: String, pub records: Vec<RecordRow>,
    pub settings_path: String, pub settings_loaded: bool,
    pub index_path: String, pub index_sessions: String, pub index_turns: String,
    pub last_indexed: String,
}
pub fn build_diagnostics(db: Option<&Db>, config_dir: &Path, settings_path: &Path,
                         probe: &dyn ProcessProbe, now_ms: i64) -> DiagnosticsModel;
```

**Diagnostics must explain a rejection, not merely report one.** `verdict_label` is the finished
sentence a user reads — "ignored: pid 4821 is `zsh`, not `claude`" — because the liveness rule
(record exists **and** the pid is alive **and** the executable is `claude`) is exactly what a
confused user needs spelled out. `live::live_sessions` currently applies that filter internally;
extract the per-record decision so both it and this share one implementation rather than drifting.

- [ ] **Step 1: Write the failing tests**

Cover: each verdict variant is produced for its own cause; a directory with no records yields an
empty list rather than an error; `config_dir_source` names the mechanism that actually resolved
it; and `build_settings` surfaces a parse error and the notes from a clamped file.

- [ ] **Step 2: Run to see them fail, implement, verify, commit**

```bash
git add crates/perch-core/src/ui
git commit -m "feat(core): settings and diagnostics view-models"
```

---

## Task 6: FFI surface

**Files:**
- Modify: `crates/perch-ffi/src/lib.rs`

**Interfaces** (on the existing `Perch` object):

```
SettingsModel    settings()
SettingsModel    save_settings(Settings s)                    throws PerchError
DiagnosticsModel diagnostics()
ProjectDetail    set_notify_override(i64 project_id, NotifyOverride o)  throws PerchError
```

plus a `PerchListener` addition — `fn on_notifications(&self, items: Vec<Notification>)` — so the
watcher's tick can push notification decisions to the shell the same way it pushes models.

Mirror every new record with a `From` impl that **destructures** the core value, as the existing
mirrors do, so a field added in core becomes a compile error here rather than silently never
reaching a shell. `set_notify_override` returns the refreshed `ProjectDetail`, matching the other
edit methods, so no shell re-fetches.

- [ ] Steps: tests first (each new method reachable; an unknown project id is a typed error), implement, `cargo test --workspace`, fmt, clippy, `cargo build --release -p perch-ffi`, `cargo build -p perch-app`, `scripts/build-xcframework.sh`, then **capture the exact generated Swift names into your report** — Tasks 7–10 are written against them.

```bash
git add crates/perch-ffi/src/lib.rs
git commit -m "feat(ffi): settings, diagnostics, notify override, notification push"
```

---

## Task 7: The settings window

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/Settings/SettingsWindow.swift`, `Settings/DiagnosticsView.swift`
- Modify: `StatusItemController.swift` (a **Settings…** item, `,` key equivalent), `PerchApp.swift`

A small `NSWindow` hosting SwiftUI, opened from the status menu, following `MainWindowController`'s
established pattern — including `isReleasedWhenClosed = false`, the `notification.object === window`
guard, and the activation-policy flip.

Three panes, chosen because eight settings do not need eleven: **General** (launch at login, Claude
Code directory, preferred terminal), **Sessions & Menu Bar** (poll interval, menu-bar display),
**Notifications** (the three notification settings). Diagnostics is a fourth pane, reachable
directly — not hidden behind a toggle, because it exists for someone already having trouble.

**Dependent controls are disabled, not hidden:** the two notification sub-options stay visible and
greyed when notifications are off.

Every displayed string comes from the model, including `notes` and `error`. Edits call
`save_settings` and adopt the returned model.

- [ ] Steps: implement, `swift build -c release`, `make bundle && open build/Perch.app`, verify what you can and **state plainly what you could not** (prior agents on this repo could not see the app's UI — black screenshots, empty Accessibility tree, one proved it with a bare control app), commit.

---

## Task 8: The per-project override, on the project

**Files:**
- Modify: `apps/macos/Perch/Sources/Perch/MainWindow/ProjectDetail.swift`

**This is the milestone's distinguishing decision — build it where the spec says.** A segmented
control — **Default / Custom / Off** — in the project detail pane beside pin, archive, and the
note, with a stepper for the custom threshold that is disabled unless Custom is selected. Not a
settings list; the point is that you set it where you are already looking.

When Default is selected, show the global value it inherits so the user knows what "default"
currently means — text from the model, not assembled in Swift.

Calls `set_notify_override` and adopts the returned `ProjectDetail`, then calls the existing
`onChanged` so the sidebar refreshes.

- [ ] Steps: implement, build, run, verify what you can, commit.

---

## Task 9: Delivering notifications

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/Notifications/Notifier.swift`
- Modify: `Bridge/PerchEngine.swift`, `PerchApp.swift`

`Notifier` does exactly one thing: hand finished strings to `UNUserNotificationCenter`. No
decision, no formatting, no threshold logic — all of that is in `notify.rs`.

**Authorization is requested when the user enables the setting, not at launch.** A permission
prompt before the app has shown its worth is the fastest way to be denied. If authorization is
refused, the settings toggle must reflect that and say where to change it, rather than appearing
on while delivering nothing.

Clicking a notification opens the main window with that session's project selected —
`UNUserNotificationCenterDelegate`, with the project id in the request's `userInfo`.

- [ ] Steps: implement, build, run. **Verify a real notification is delivered** — enable the setting and trigger one however you can (a genuinely waiting session, or by temporarily lowering the threshold). If your environment cannot show notifications, say so plainly rather than claiming it works. Commit.

---

## Task 10: Making every setting take effect

**Files:**
- Modify: `PerchApp.swift`, `StatusItemController.swift`, `MainWindow/Launcher.swift`, `Bridge/PerchEngine.swift`, and `perch-ffi` where the watcher is configured

**A setting that does not change behaviour is worse than no setting** — it is a lie in a window.
This task wires all eight, and its review will check each one end to end:

| Setting | What must actually change |
|---|---|
| Launch at login | `SMAppService.mainApp.register()` / `.unregister()` on toggle, with the real current state reflected on open |
| Claude Code directory | Overrides the auto-detected path; the engine restarts against it, and Diagnostics shows the new source |
| Menu-bar display | The status item shows an icon only, a count, or a count with the waiting hourglass |
| Poll interval | The watcher's poll backstop uses it (`WatcherConfig`), not the hardcoded 5 s |
| Notifications enabled / threshold / background | Passed into `notify::decide` on every tick |
| Preferred terminal | `Launcher` opens that terminal rather than always `Terminal.app` |

The config file is watched, so a hand-edit must take effect without restarting Perch. Prove that:
edit `config.toml` on disk while the app runs and confirm the change lands.

- [ ] Steps: implement, build, run, verify each row of the table and report what you observed per row, commit.

---

## Task 11: CI and docs

- [ ] **Guards.** Run each `no-network` guard against the real tree and confirm it passes and its self-test fires. `toml_edit` is the one new crate — confirm the forbidden-crate graph check still passes on both matrix scopes and that no HTTP client entered the graph. The Swift write guard added last milestone permits writes only in `Launcher.swift`; `Notifier.swift` writes nothing, so it must still pass unchanged — verify rather than assume.
- [ ] **README:** document the settings window, the config file (with its path and the `PERCH_CONFIG` override), the waiting-on-you notification, and diagnostics. Keep every privacy claim exactly as true as it is: notifications are local, the config file is Perch's own, and nothing here makes a network request.
- [ ] **Backlog:** move the settings window and the waiting-on-you notification into **Shipped**, rewritten to describe what shipped. Add: hooks on session events (with the trust caveat), the approaching-limit notification (blocked on the usage endpoint), settings search and per-section reset (deferred until the surface is bigger).
- [ ] Verify and commit.

---

## Self-Review

**Spec coverage.** §3 architecture — every file has a task. §4 config file → Tasks 1–2. §5 the eight settings → Task 1 (shape), Task 7 (window), Task 10 (effect). §6 notifications → Task 4 (decision), Task 9 (delivery). §7 diagnostics → Task 5 (model), Task 7 (pane). §8 error handling → Task 2 (`load` never fails), Task 7 (surfacing), Task 9 (authorization denied). §9 testing → Tasks 1–5 carry it. §10 cross-platform — only `SMAppService`, `UNUserNotificationCenter`, and the default path are OS-bound.

**The inversion is preserved:** per-project overrides are Task 8, in the project detail pane. No task builds a settings list of projects.

**Type consistency checked:** `Settings` fields are identical across Task 1 (core), Task 6 (FFI mirror), and Tasks 7/10 (Swift, camelCased). `NotifyOverride` is produced in Task 3, consumed by Task 4's `override_for`, mirrored in Task 6, set in Task 8. `Episode`/`Notification` are produced in Task 4 and delivered in Task 9.

**Placeholder scan:** none. Tasks 5, 7, 8, 9 give interfaces and requirements rather than full literal code, which is deliberate for the view and window layers — their exact shape depends on the generated Swift names captured in Task 6, and inventing them here would be guessing.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks.
2. **Inline Execution** — tasks executed in this session with batch checkpoints.
