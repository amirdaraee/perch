# Perch Tray & Live Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the working data layer into a macOS menu-bar app that shows which Claude Code sessions are running, which are blocked waiting on you, and roughly how much you have burned — updating live.

**Architecture:** A Tauri 2 app whose Rust side adds two modules to `perch-core` (`live` for session detection, `platform` for the OS-bound traits) plus a thin `src-tauri` shell exposing Tauri commands and pushing events. The frontend is a React + TypeScript single page rendering the dense popover. The app is tray-resident: closing the window does not quit it, and it shows no Dock icon.

**Tech Stack:** Rust 1.98 (workspace already established), Tauri 2, `notify` for file watching, `libc` for `kill(pid, 0)`; React 19 + TypeScript + Vite via pnpm 10.

**Spec:** `docs/superpowers/specs/2026-08-30-perch-design.md` — this plan implements Milestone 2 (spec §12), covering §7 (live detection), §9.1 (menu bar), and the parts of §9.4/§9.5 that the popover needs.

## Global Constraints

- **Scope:** spec Milestone 2 only. The main window (§9.2), the usage/limits API client (§8 tier 1), the full usage view (§9.3), and release packaging get their own plans.
- **Read-only:** nothing may write to, move, or delete anything under the Claude Code config directory. Only Perch's own app-data directory is written.
- **No network:** this plan adds no HTTP client. The popover's usage figures come from tier-2 local estimation (`query::usage_since`), which already exists, and must be visually marked as estimates per spec §8. CI fails the build if an HTTP client appears.
- **No telemetry, no analytics, no crash reporting** (spec §10).
- **No transcript contents** may be rendered, logged, or emitted — only paths, counts, totals, model names, session names, and status strings.
- Rust edition 2021; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` must pass.
- **`perch-core` must keep compiling and testing on Linux.** OS-bound behaviour goes behind traits (spec §4.1); only `src-tauri` and the trait implementations may be macOS-specific.
- **Liveness requires three confirmations** (spec §7): record exists, `kill(pid, 0)` succeeds, and the process command line contains the matching `--session-id`. Two are not enough — pids are recycled.
- Status comes from the `status`/`waitingFor` fields Claude Code publishes. **Never infer status from file mtimes.**
- License MIT; `.superpowers/` stays git-ignored.

---

## Decision recorded here: the frontend framework

The spec does not name one. **React 19 + TypeScript + Vite.** Reasoning: this repository is going public, and React has by far the largest contributor pool; the usage view in a later plan needs charts and tables, where React's ecosystem is strongest; and Tauri's official templates support it first-class. Svelte would produce a smaller bundle and less ceremony, which matters little for a tray app whose weight is dominated by the webview itself. This is cheap to change now and expensive later — if you prefer Svelte, say so before Task 2.

---

## File Structure

```
crates/perch-core/src/
├── live.rs            # NEW — session record parsing, liveness, status model
└── platform/
    ├── mod.rs         # NEW — ProcessProbe + AppLauncher traits (OS-agnostic)
    ├── macos.rs       # NEW — macOS implementations
    └── fallback.rs    # NEW — non-macOS stubs so Linux still builds and tests

src-tauri/
├── Cargo.toml
├── tauri.conf.json
├── build.rs
├── icons/
└── src/
    ├── main.rs        # thin: calls lib::run()
    ├── lib.rs         # Tauri builder, tray, activation policy, window wiring
    ├── commands.rs    # #[tauri::command] surface over perch-core
    └── watcher.rs     # file-watch loop -> emits events to the frontend

src/                   # frontend
├── main.tsx
├── App.tsx            # the dense popover
├── api.ts             # typed wrappers over invoke() + event listeners
├── types.ts           # mirrors the Rust payload types
├── format.ts          # token/cost/duration formatting
└── styles.css
```

**Responsibilities.** `live.rs` knows session records and nothing about Tauri. `platform/` is the only place OS-specific code lives. `src-tauri/commands.rs` is a translation layer with no logic of its own. `watcher.rs` owns the debounce and emits; it does not decide status.

---

## Task 1: Live session records — parsing and the status model

**Files:**
- Create: `crates/perch-core/src/live.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `config::sessions_dir`.
- Produces:
  - `pub enum SessionStatus { Working, Waiting { reason: Option<String>, since_ms: i64 }, Background, Ended }`
  - `pub struct LiveSession { pub pid: i32, pub session_id: String, pub cwd: String, pub name: String, pub kind: String, pub status: SessionStatus, pub started_at: i64, pub status_updated_at: i64, pub cc_version: Option<String>, pub socket_path: Option<String> }`
  - `pub fn parse_session_record(json: &str) -> Option<LiveSession>`
  - `pub fn record_files(sessions_dir: &Path) -> Vec<PathBuf>`

Real records look like this (fields beyond these exist and must be ignored):

```json
{"pid":11107,"sessionId":"25687be6-…","cwd":"/Users/a/proj","startedAt":1788097229808,
 "version":"2.1.251","kind":"interactive","messagingSocketPath":"/tmp/cc-socks/11107.sock",
 "name":"first-week-university-b6","status":"busy","statusUpdatedAt":1788099012751}
```
A waiting record additionally carries `"waitingFor":"dialog open"`. `kind` is `"interactive"` or `"bg"`.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/live.rs`:

```rust
//! Live Claude Code session records. Status is read, never inferred.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: &str = r#"{"pid":11107,"sessionId":"s-busy","cwd":"/Users/a/proj","startedAt":1788097229808,"version":"2.1.251","kind":"interactive","messagingSocketPath":"/tmp/cc-socks/11107.sock","name":"first-week-university-b6","status":"busy","statusUpdatedAt":1788099012751}"#;

    const WAITING: &str = r#"{"pid":30778,"sessionId":"s-wait","cwd":"/Users/a/dash","startedAt":1787988672885,"version":"2.1.250","kind":"bg","name":"Personal dashboard finance section","status":"waiting","waitingFor":"dialog open","statusUpdatedAt":1787988673228}"#;

    #[test]
    fn parses_a_busy_interactive_session() {
        let s = parse_session_record(BUSY).expect("should parse");
        assert_eq!(s.pid, 11107);
        assert_eq!(s.session_id, "s-busy");
        assert_eq!(s.name, "first-week-university-b6");
        assert_eq!(s.kind, "interactive");
        assert_eq!(s.cwd, "/Users/a/proj");
        assert_eq!(s.status, SessionStatus::Working);
        assert_eq!(s.socket_path.as_deref(), Some("/tmp/cc-socks/11107.sock"));
    }

    #[test]
    fn waiting_carries_its_reason_and_timestamp() {
        let s = parse_session_record(WAITING).unwrap();
        match &s.status {
            SessionStatus::Waiting { reason, since_ms } => {
                assert_eq!(reason.as_deref(), Some("dialog open"));
                assert_eq!(*since_ms, 1787988673228);
            }
            other => panic!("expected Waiting, got {other:?}"),
        }
    }

    #[test]
    fn background_kind_is_preserved_alongside_status() {
        // `kind` and `status` are independent: a bg session can be busy or waiting.
        let s = parse_session_record(WAITING).unwrap();
        assert_eq!(s.kind, "bg");
        assert!(matches!(s.status, SessionStatus::Waiting { .. }));
    }

    #[test]
    fn unknown_status_string_is_not_fatal() {
        let line = r#"{"pid":1,"sessionId":"x","cwd":"/tmp","name":"n","kind":"interactive","status":"some-future-state","startedAt":1,"statusUpdatedAt":2}"#;
        let s = parse_session_record(line).unwrap();
        assert_eq!(s.status, SessionStatus::Working, "unknown status falls back to Working");
    }

    #[test]
    fn missing_optional_fields_are_tolerated() {
        let line = r#"{"pid":2,"sessionId":"y","cwd":"/tmp","name":"n","kind":"interactive","status":"busy","startedAt":1,"statusUpdatedAt":2}"#;
        let s = parse_session_record(line).unwrap();
        assert!(s.socket_path.is_none());
        assert!(s.cc_version.is_none());
    }

    #[test]
    fn a_record_without_a_pid_or_session_id_is_rejected() {
        assert!(parse_session_record(r#"{"sessionId":"z","cwd":"/tmp"}"#).is_none());
        assert!(parse_session_record(r#"{"pid":3,"cwd":"/tmp"}"#).is_none());
    }

    #[test]
    fn malformed_json_is_rejected_not_panicked() {
        assert!(parse_session_record("{nope").is_none());
        assert!(parse_session_record("").is_none());
        assert!(parse_session_record("[1,2]").is_none());
    }

    #[test]
    fn record_files_lists_only_json_and_tolerates_a_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("111.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("222.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("333.key"), "x").unwrap();
        let mut found = record_files(tmp.path());
        found.sort();
        assert_eq!(found.len(), 2);

        assert!(record_files(&tmp.path().join("nope")).is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core live`
Expected: FAIL — `cannot find function parse_session_record`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// `rename_all` renames the VARIANTS; `rename_all_fields` is also required, or
// `since_ms` serializes as "since_ms" while the frontend expects "sinceMs".
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SessionStatus {
    Working,
    Waiting { reason: Option<String>, since_ms: i64 },
    Background,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSession {
    pub pid: i32,
    pub session_id: String,
    pub cwd: String,
    pub name: String,
    /// "interactive" or "bg". Independent of `status`.
    pub kind: String,
    pub status: SessionStatus,
    pub started_at: i64,
    pub status_updated_at: i64,
    pub cc_version: Option<String>,
    pub socket_path: Option<String>,
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn i64_at(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// Parse one `<pid>.json` session record. Lenient: unknown fields are ignored and a
/// malformed record yields `None` rather than an error, because Claude Code owns this
/// format and may extend it at any time.
pub fn parse_session_record(json: &str) -> Option<LiveSession> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    if !v.is_object() {
        return None;
    }

    let pid = v.get("pid").and_then(Value::as_i64)? as i32;
    let session_id = str_at(&v, "sessionId")?;
    let status_updated_at = i64_at(&v, "statusUpdatedAt");

    let status = match v.get("status").and_then(Value::as_str) {
        Some("waiting") => SessionStatus::Waiting {
            reason: str_at(&v, "waitingFor"),
            since_ms: status_updated_at,
        },
        // "busy" and anything unrecognised: treat as working rather than
        // inventing a state. A future status string must not break the UI.
        _ => SessionStatus::Working,
    };

    Some(LiveSession {
        pid,
        session_id,
        cwd: str_at(&v, "cwd").unwrap_or_default(),
        name: str_at(&v, "name").unwrap_or_default(),
        kind: str_at(&v, "kind").unwrap_or_else(|| "interactive".to_string()),
        status,
        started_at: i64_at(&v, "startedAt"),
        status_updated_at,
        cc_version: str_at(&v, "version"),
        socket_path: str_at(&v, "messagingSocketPath"),
    })
}

/// List `<pid>.json` files in the sessions directory. A missing directory is normal.
pub fn record_files(sessions_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
        .collect()
}
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add `pub mod live;` in alphabetical position.

- [ ] **Step 5: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core live`
Expected: PASS, 8 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/live.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): parse live session records with published status"
```

---

## Task 2: The platform traits and liveness

**Files:**
- Create: `crates/perch-core/src/platform/mod.rs`
- Create: `crates/perch-core/src/platform/macos.rs`
- Create: `crates/perch-core/src/platform/fallback.rs`
- Modify: `crates/perch-core/src/live.rs` (add `live_sessions`)
- Modify: `crates/perch-core/src/lib.rs`
- Modify: `crates/perch-core/Cargo.toml`

**Interfaces:**
- Consumes: `live::{LiveSession, parse_session_record, record_files}`.
- Produces:
  - `pub trait ProcessProbe: Send + Sync { fn is_alive(&self, pid: i32) -> bool; fn cmdline_contains(&self, pid: i32, needle: &str) -> bool; }`
  - `pub struct RealProcessProbe;` implementing it
  - `pub fn live_sessions(sessions_dir: &Path, probe: &dyn ProcessProbe) -> Vec<LiveSession>`

Spec §7 requires **three** confirmations. `is_alive` covers `kill(pid, 0)`; `cmdline_contains` covers the `--session-id` match; the record's existence is the third. Two alone are insufficient because pids are recycled.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `crates/perch-core/src/live.rs`:

```rust
    use crate::platform::ProcessProbe;
    use std::collections::HashSet;

    /// Fake probe so liveness rules are unit-testable without real processes.
    struct FakeProbe {
        alive: HashSet<i32>,
        cmdlines: std::collections::HashMap<i32, String>,
    }

    impl FakeProbe {
        fn new(alive: &[i32]) -> Self {
            Self { alive: alive.iter().copied().collect(), cmdlines: Default::default() }
        }
        fn with_cmdline(mut self, pid: i32, line: &str) -> Self {
            self.cmdlines.insert(pid, line.to_string());
            self
        }
    }

    impl ProcessProbe for FakeProbe {
        fn is_alive(&self, pid: i32) -> bool {
            self.alive.contains(&pid)
        }
        fn cmdline_contains(&self, pid: i32, needle: &str) -> bool {
            self.cmdlines.get(&pid).is_some_and(|l| l.contains(needle))
        }
    }

    fn write_record(dir: &Path, pid: i32, session_id: &str, status: &str) {
        let json = format!(
            r#"{{"pid":{pid},"sessionId":"{session_id}","cwd":"/tmp/p","name":"n","kind":"interactive","status":"{status}","startedAt":1,"statusUpdatedAt":2}}"#
        );
        std::fs::write(dir.join(format!("{pid}.json")), json).unwrap();
    }

    #[test]
    fn a_session_needs_all_three_confirmations() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 100, "alive-and-matching", "busy");

        let probe = FakeProbe::new(&[100]).with_cmdline(100, "claude --session-id alive-and-matching");
        assert_eq!(live_sessions(tmp.path(), &probe).len(), 1);
    }

    #[test]
    fn a_dead_pid_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 101, "dead", "busy");
        let probe = FakeProbe::new(&[]).with_cmdline(101, "claude --session-id dead");
        assert!(live_sessions(tmp.path(), &probe).is_empty(), "stale record must not resurrect");
    }

    #[test]
    fn a_recycled_pid_running_something_else_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 102, "ours", "busy");
        // pid is alive, but it is now some unrelated process
        let probe = FakeProbe::new(&[102]).with_cmdline(102, "/usr/bin/python3 unrelated.py");
        assert!(
            live_sessions(tmp.path(), &probe).is_empty(),
            "pid reuse must not be reported as a live session"
        );
    }

    #[test]
    fn live_sessions_are_sorted_waiting_first_then_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 200, "b-working", "busy");
        write_record(tmp.path(), 201, "a-waiting", "waiting");

        let probe = FakeProbe::new(&[200, 201])
            .with_cmdline(200, "claude --session-id b-working")
            .with_cmdline(201, "claude --session-id a-waiting");

        let out = live_sessions(tmp.path(), &probe);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].session_id, "a-waiting", "blocked sessions surface first");
    }

    #[test]
    fn an_unreadable_record_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bad.json"), "{ not json").unwrap();
        write_record(tmp.path(), 300, "good", "busy");
        let probe = FakeProbe::new(&[300]).with_cmdline(300, "claude --session-id good");
        assert_eq!(live_sessions(tmp.path(), &probe).len(), 1);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core live`
Expected: FAIL — `unresolved import crate::platform`.

- [ ] **Step 3: Add the `libc` dependency**

In the workspace root `Cargo.toml` under `[workspace.dependencies]`, add:

```toml
libc = "0.2"
```

In `crates/perch-core/Cargo.toml`, add a new target-scoped section (NOT a line inside the existing `[dependencies]` table — it is its own table and must come after it):

```toml
[target.'cfg(unix)'.dependencies]
libc = { workspace = true }
```

- [ ] **Step 4: Write the platform module**

Create `crates/perch-core/src/platform/mod.rs`:

```rust
//! The only OS-bound seams in the data layer. Everything else is portable.

/// Confirms a process is alive AND is the process we think it is.
/// Both are required: pids are recycled, so liveness alone would let a stale
/// session record resurrect as a false "running session" (spec §7).
pub trait ProcessProbe: Send + Sync {
    fn is_alive(&self, pid: i32) -> bool;
    fn cmdline_contains(&self, pid: i32, needle: &str) -> bool;
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::RealProcessProbe;

#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(not(target_os = "macos"))]
pub use fallback::RealProcessProbe;
```

Create `crates/perch-core/src/platform/macos.rs`:

```rust
use super::ProcessProbe;

#[derive(Debug, Default, Clone, Copy)]
pub struct RealProcessProbe;

impl ProcessProbe for RealProcessProbe {
    /// `kill(pid, 0)` sends no signal; it only checks the process exists and
    /// is signalable by this user.
    fn is_alive(&self, pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn cmdline_contains(&self, pid: i32, needle: &str) -> bool {
        let Ok(out) = std::process::Command::new("/bin/ps")
            .args(["-o", "command=", "-p", &pid.to_string()])
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).contains(needle)
    }
}
```

Create `crates/perch-core/src/platform/fallback.rs`:

```rust
use super::ProcessProbe;

/// Non-macOS builds exist so `perch-core` keeps compiling and testing on Linux.
/// These are honest stubs, not silent lies: they report nothing is alive, so a
/// non-macOS build shows no live sessions rather than wrong ones.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealProcessProbe;

impl ProcessProbe for RealProcessProbe {
    fn is_alive(&self, _pid: i32) -> bool {
        false
    }
    fn cmdline_contains(&self, _pid: i32, _needle: &str) -> bool {
        false
    }
}
```

- [ ] **Step 5: Add `live_sessions` to `live.rs`**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/live.rs`:

```rust
use crate::platform::ProcessProbe;

/// Read every session record and keep only those passing all three confirmations
/// from spec §7. Blocked sessions sort first — they are the ones needing you.
pub fn live_sessions(sessions_dir: &Path, probe: &dyn ProcessProbe) -> Vec<LiveSession> {
    let mut out: Vec<LiveSession> = record_files(sessions_dir)
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(&p).ok())
        .filter_map(|s| parse_session_record(&s))
        .filter(|s| probe.is_alive(s.pid) && probe.cmdline_contains(s.pid, &s.session_id))
        .collect();

    out.sort_by(|a, b| {
        let rank = |s: &LiveSession| match s.status {
            SessionStatus::Waiting { .. } => 0,
            _ => 1,
        };
        rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
    });
    out
}
```

- [ ] **Step 6: Register the module**

In `crates/perch-core/src/lib.rs`, add `pub mod platform;` in alphabetical position.

- [ ] **Step 7: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core live`
Expected: PASS, 16 tests (11 from Task 1 — which grew from 8 in its fix round — plus 5 new).

- [ ] **Step 8: Verify against the real machine**

Run:

```bash
source "$HOME/.cargo/env" && cargo test -p perch-core live && ls ~/.claude/sessions/*.json | wc -l
```

This confirms real records exist. Do not modify anything in that directory.

- [ ] **Step 9: Commit**

```bash
git add crates/perch-core/src/live.rs crates/perch-core/src/platform crates/perch-core/src/lib.rs crates/perch-core/Cargo.toml Cargo.toml Cargo.lock
git commit -m "feat(core): liveness with three confirmations behind a ProcessProbe trait"
```

---

## Task 3: A CLI `sessions` command to prove Tasks 1–2 on real data

**Files:**
- Modify: `crates/perch-cli/src/main.rs`

**Interfaces:**
- Consumes: `live::live_sessions`, `platform::RealProcessProbe`, `config::sessions_dir`.
- Produces: a `sessions` subcommand.

This exists so live detection is provable before any GUI code is written — the same discipline that made Milestone 1's headless gate valuable.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block at the bottom of `crates/perch-cli/src/main.rs`:

```rust
    #[test]
    fn formats_elapsed_durations_compactly() {
        assert_eq!(human_elapsed(0), "0s");
        assert_eq!(human_elapsed(45_000), "45s");
        assert_eq!(human_elapsed(90_000), "1m");
        assert_eq!(human_elapsed(3_600_000), "1h");
        assert_eq!(human_elapsed(115_200_000), "32h");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-cli`
Expected: FAIL — `cannot find function human_elapsed`.

- [ ] **Step 3: Implement**

Add to `crates/perch-cli/src/main.rs`:

```rust
fn human_elapsed(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}
```

Add `Sessions` to the `Command` enum:

```rust
    /// List live Claude Code sessions.
    Sessions,
```

And its arm in `main`:

```rust
        Command::Sessions => {
            use perch_core::live::{live_sessions, SessionStatus};
            use perch_core::platform::RealProcessProbe;

            let probe = RealProcessProbe;
            let sessions = live_sessions(&config::sessions_dir(&config_dir), &probe);
            let now = chrono::Utc::now().timestamp_millis();

            if sessions.is_empty() {
                println!("no live sessions");
            }
            println!("{:<40} {:<10} {:<24} {:>6}", "SESSION", "KIND", "STATUS", "FOR");
            for s in sessions {
                let (label, since) = match &s.status {
                    SessionStatus::Waiting { reason, since_ms } => (
                        format!("waiting · {}", reason.as_deref().unwrap_or("unknown")),
                        *since_ms,
                    ),
                    _ => ("working".to_string(), s.status_updated_at),
                };
                println!(
                    "{:<40} {:<10} {:<24} {:>6}",
                    truncate(&s.name, 38),
                    s.kind,
                    truncate(&label, 22),
                    human_elapsed(now - since),
                );
            }
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-cli`
Expected: PASS, 3 tests.

- [ ] **Step 5: Prove it on real data — the gate for Tasks 1–3**

```bash
source "$HOME/.cargo/env" && cargo run --release -p perch-cli -- sessions
```

Verify and report each:
1. The listed sessions match what is actually running (cross-check with `ls ~/.claude/sessions/*.json | wc -l` — the CLI may legitimately show fewer if some records are stale).
2. Any session in `waiting` shows its reason and a plausible elapsed time.
3. Blocked sessions sort above working ones.
4. No session appears whose pid is dead.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-cli/src/main.rs
git commit -m "feat(cli): sessions command listing live sessions with status"
```

---

## Task 4: Tauri scaffold

**Files:**
- Create: `src-tauri/Cargo.toml`, `src-tauri/build.rs`, `src-tauri/tauri.conf.json`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/icons/`
- Create: `package.json`, `pnpm-workspace.yaml` (if needed), `vite.config.ts`, `tsconfig.json`, `index.html`, `src/main.tsx`, `src/App.tsx`, `src/styles.css`
- Modify: root `Cargo.toml` (add `src-tauri` to workspace members)
- Modify: `.gitignore`

**Interfaces:**
- Produces: an app that builds and launches, showing an empty window. No tray yet.

- [ ] **Step 1: Install the Tauri CLI**

```bash
source "$HOME/.cargo/env" && cargo install tauri-cli --version "^2" --locked
cargo tauri --version
```

Expected: prints a 2.x version. This takes several minutes; run it as its own step.

- [ ] **Step 2: Create the frontend**

```bash
pnpm init
pnpm add -D vite @vitejs/plugin-react typescript @types/react @types/react-dom
pnpm add react react-dom @tauri-apps/api
```

Create `vite.config.ts`:

```ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: 'dist', emptyOutDir: true },
})
```

Create `tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "skipLibCheck": true,
    "isolatedModules": true,
    "noEmit": true
  },
  "include": ["src"]
}
```

Create `index.html`:

```html
<!doctype html>
<html lang="en">
  <head><meta charset="UTF-8" /><title>Perch</title></head>
  <body><div id="root"></div><script type="module" src="/src/main.tsx"></script></body>
</html>
```

Create `src/main.tsx`:

```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import './styles.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
```

Create `src/App.tsx`:

```tsx
export default function App() {
  return <div className="popover">Perch</div>
}
```

Create `src/styles.css`:

```css
:root { color-scheme: dark; }
body { margin: 0; font: 13px -apple-system, "SF Pro Text", system-ui, sans-serif; }
.popover { padding: 12px; }
```

- [ ] **Step 3: Create the Tauri crate**

Create `src-tauri/Cargo.toml`:

```toml
[package]
name = "perch-app"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
name = "perch_app_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
perch-core = { path = "../crates/perch-core" }
tauri = { version = "2", features = ["tray-icon"] }
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
chrono.workspace = true
notify = "8"
```

Create `src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

Create `src-tauri/src/main.rs`:

```rust
// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    perch_app_lib::run()
}
```

Create `src-tauri/src/lib.rs`:

```rust
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Perch");
}
```

Create `src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Perch",
  "version": "0.1.0",
  "identifier": "dev.perch.app",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "label": "popover",
        "title": "Perch",
        "width": 360,
        "height": 460,
        "resizable": false,
        "decorations": false,
        "transparent": false,
        "alwaysOnTop": true,
        "skipTaskbar": true,
        "visible": false
      }
    ],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": ["app", "dmg"],
    "icon": ["icons/icon.png"]
  }
}
```

Add `"dev": "vite"`, `"build": "vite build"` to `package.json` scripts.

- [ ] **Step 4: Generate icons**

```bash
source "$HOME/.cargo/env" && cargo tauri icon --help
```

If no source image exists, create a minimal 1024×1024 PNG placeholder and run `cargo tauri icon <path>`. A placeholder is acceptable for this plan; a designed icon is a later concern. Record in your report which you used.

- [ ] **Step 5: Add to the workspace and ignore build output**

In the root `Cargo.toml`, change members to:

```toml
members = ["crates/perch-core", "crates/perch-cli", "src-tauri"]
```

Append to `.gitignore`:

```
dist/
.vite/
```

- [ ] **Step 6: Verify it builds and the existing suite still passes**

```bash
source "$HOME/.cargo/env" && cargo build --workspace
source "$HOME/.cargo/env" && cargo test --workspace
pnpm build
```

Expected: all succeed; the 66 existing tests still pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: scaffold Tauri 2 app with React frontend"
```

---

## Task 5: Tray, activation policy, and popover behaviour

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: a tray-resident app with no Dock icon, whose tray click toggles the popover.

- [ ] **Step 1: Implement the tray and window behaviour**

Replace `src-tauri/src/lib.rs`:

```rust
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

fn toggle_popover(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Menu-bar app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItem::with_id(app, "quit", "Quit Perch", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            TrayIconBuilder::with_id("perch-tray")
                // `?`, not `unwrap()`: a missing icon must surface as a setup
                // error, not a panic on launch.
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .ok_or("no default window icon configured")?,
                )
                .icon_as_template(true)
                .title("Perch")
                .menu(&menu)
                // Left click toggles the popover; the menu is right-click only.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id().as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the popover must not quit the app — Perch lives in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Perch");
}
```

- [ ] **Step 2: Verify the tray behaviour by hand**

```bash
source "$HOME/.cargo/env" && cargo tauri dev
```

Verify and report each:
1. A tray item appears in the menu bar.
2. **No Dock icon appears** and Perch does not show in ⌘-Tab.
3. Left-clicking the tray shows the popover; clicking again hides it.
4. Right-clicking shows a menu with Quit, and Quit exits.
5. Closing the popover window (⌘W) hides it rather than quitting.

If `set_activation_policy` does not resolve, check the current Tauri 2 API name and report what you used instead of guessing silently.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(app): tray-resident menu bar app with toggling popover"
```

---

## Task 6: Tauri commands over `perch-core`

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces:
  - `#[tauri::command] fn live_sessions() -> Result<Vec<LiveSession>, String>`
  - `#[tauri::command] fn usage_summary() -> Result<UsageSummary, String>`
  - `pub struct UsageSummary { window: UsageSlice, week: UsageSlice, today: UsageSlice, source: String }` where `UsageSlice { tokens: u64, cost_usd: f64 }`

`source` is always `"estimated"` in this plan — the tier-1 API client is a later milestone, and spec §8 requires estimates be marked as such.

- [ ] **Step 1: Write the command layer**

Create `src-tauri/src/commands.rs`:

```rust
use perch_core::{config, db, live, platform::RealProcessProbe, pricing, query};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSlice {
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub window: UsageSlice,
    pub week: UsageSlice,
    pub today: UsageSlice,
    /// "estimated" until the tier-1 usage endpoint lands (spec §8).
    pub source: String,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    config::config_dir().ok_or_else(|| "could not locate a Claude Code config directory".to_string())
}

/// Perch's own database — never inside the Claude Code directory.
fn db_path() -> Result<std::path::PathBuf, String> {
    let base = dirs_next_app_data()?;
    Ok(base.join("index.db"))
}

fn dirs_next_app_data() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    Ok(std::path::PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Perch"))
}

#[tauri::command]
pub fn live_sessions() -> Result<Vec<live::LiveSession>, String> {
    let cfg = config_dir()?;
    Ok(live::live_sessions(
        &config::sessions_dir(&cfg),
        &RealProcessProbe,
    ))
}

#[tauri::command]
pub fn usage_summary() -> Result<UsageSummary, String> {
    let database = db::open(&db_path()?).map_err(|e| e.to_string())?;
    pricing::seed_default_prices(&database).map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp_millis();
    let slice = |since: i64| -> Result<UsageSlice, String> {
        let (u, cost) = query::usage_since(&database, since).map_err(|e| e.to_string())?;
        Ok(UsageSlice { tokens: u.total_tokens(), cost_usd: cost })
    };

    Ok(UsageSummary {
        window: slice(now - 5 * 60 * 60 * 1000)?,
        week: slice(now - 7 * 24 * 60 * 60 * 1000)?,
        today: slice(now - 24 * 60 * 60 * 1000)?,
        source: "estimated".to_string(),
    })
}

#[tauri::command]
pub fn reindex() -> Result<perch_core::index::IndexStats, String> {
    let cfg = config_dir()?;
    let database = db::open(&db_path()?).map_err(|e| e.to_string())?;
    pricing::seed_default_prices(&database).map_err(|e| e.to_string())?;
    perch_core::index::index_all(&database, &config::projects_dir(&cfg)).map_err(|e| e.to_string())
}
```

`IndexStats` must derive `Serialize` for this to compile. If it does not, add `serde::Serialize` to its derive list in `crates/perch-core/src/index.rs` and note it in your report.

- [ ] **Step 2: Register the commands**

In `src-tauri/src/lib.rs`, add `mod commands;` and add to the builder before `.run(...)`:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::live_sessions,
            commands::usage_summary,
            commands::reindex
        ])
```

- [ ] **Step 3: Verify it compiles and the app still runs**

```bash
source "$HOME/.cargo/env" && cargo build --workspace
source "$HOME/.cargo/env" && cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src
git commit -m "feat(app): Tauri commands for live sessions, usage, and reindex"
```

---

## Task 7: The file watcher and live events

**Files:**
- Create: `src-tauri/src/watcher.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `pub fn spawn(app: tauri::AppHandle)` — watches the sessions directory and emits `sessions-changed` with the current `Vec<LiveSession>`.

Debounce is required: a status flip rewrites the record file, and several events can arrive for one logical change.

- [ ] **Step 1: Write the watcher**

Create `src-tauri/src/watcher.rs`:

```rust
use notify::{RecursiveMode, Watcher};
use perch_core::{config, live, platform::RealProcessProbe};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Coalesce bursts: one status change rewrites the record file and can produce
/// several filesystem events.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Re-poll even without an event, so a session whose process dies (leaving a
/// stale record and no filesystem change) still disappears from the UI.
const POLL: Duration = Duration::from_secs(5);

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let Some(cfg) = config::config_dir() else {
            return;
        };
        let dir = config::sessions_dir(&cfg);

        let (tx, rx) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }

        emit_now(&app, &dir);
        let mut last = Instant::now();

        loop {
            match rx.recv_timeout(POLL) {
                Ok(_) => {
                    if last.elapsed() >= DEBOUNCE {
                        emit_now(&app, &dir);
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    emit_now(&app, &dir);
                    last = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn emit_now(app: &AppHandle, dir: &std::path::Path) {
    let sessions = live::live_sessions(dir, &RealProcessProbe);
    let _ = app.emit("sessions-changed", sessions);
}
```

The watcher never creates the sessions directory. If it does not exist, `watcher.watch` fails and the thread exits — the correct behaviour, and the read-only constraint forbids creating it.

- [ ] **Step 2: Start the watcher in setup**

In `src-tauri/src/lib.rs`, add `mod watcher;` and inside `.setup(|app| { … })`, after the tray is built:

```rust
            watcher::spawn(app.handle().clone());
```

- [ ] **Step 3: Verify no write occurs to the Claude Code directory**

```bash
source "$HOME/.cargo/env" && grep -rn "create_dir_all\|fs::write\|File::create\|remove_file" src-tauri/src/
```

Expected: no hit that targets a path under the Claude Code config directory. The only legitimate creation is Perch's own app-data directory in `commands.rs`.

- [ ] **Step 4: Build and commit**

```bash
source "$HOME/.cargo/env" && cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings
git add src-tauri/src
git commit -m "feat(app): watch session records and emit live updates"
```

---

## Task 8: The dense popover UI

**Files:**
- Create: `src/types.ts`, `src/api.ts`, `src/format.ts`
- Modify: `src/App.tsx`, `src/styles.css`

**Interfaces:**
- Produces: the dense popover from spec §9.1 — three stats across the top, then session rows.

- [ ] **Step 1: Types mirroring the Rust payloads**

Create `src/types.ts`:

```ts
export type SessionStatus =
  | { kind: 'working' }
  | { kind: 'waiting'; reason: string | null; sinceMs: number }
  | { kind: 'background' }
  | { kind: 'ended' }

export interface LiveSession {
  pid: number
  sessionId: string
  cwd: string
  name: string
  kind: string
  status: SessionStatus
  startedAt: number
  statusUpdatedAt: number
  ccVersion: string | null
  socketPath: string | null
}

export interface UsageSlice { tokens: number; costUsd: number }

export interface UsageSummary {
  window: UsageSlice
  week: UsageSlice
  today: UsageSlice
  source: 'estimated' | 'api'
}
```

- [ ] **Step 2: The API wrapper**

Create `src/api.ts`:

```ts
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { LiveSession, UsageSummary } from './types'

export const getLiveSessions = () => invoke<LiveSession[]>('live_sessions')
export const getUsageSummary = () => invoke<UsageSummary>('usage_summary')
export const reindex = () => invoke('reindex')

export const onSessionsChanged = (cb: (s: LiveSession[]) => void) =>
  listen<LiveSession[]>('sessions-changed', (e) => cb(e.payload))
```

- [ ] **Step 3: Formatting helpers**

Create `src/format.ts`:

```ts
export function tokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

export function cost(usd: number): string {
  return `$${usd.toFixed(2)}`
}

export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000))
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  return `${Math.floor(s / 3600)}h`
}
```

- [ ] **Step 4: The popover**

Replace `src/App.tsx`:

```tsx
import { useEffect, useState } from 'react'
import { getLiveSessions, getUsageSummary, onSessionsChanged } from './api'
import type { LiveSession, UsageSummary } from './types'
import { cost, elapsed, tokens } from './format'

function StatusDot({ s }: { s: LiveSession }) {
  const cls = s.status.kind === 'waiting' ? 'dot y' : s.kind === 'bg' ? 'dot w' : 'dot g'
  return <span className={cls} />
}

function SessionRow({ s, now }: { s: LiveSession; now: number }) {
  const waiting = s.status.kind === 'waiting' ? s.status : null
  const since = waiting ? waiting.sinceMs : s.statusUpdatedAt
  return (
    <div className="srow">
      <StatusDot s={s} />
      <span className="sname">
        <b>{s.name || s.sessionId.slice(0, 8)}</b>
        <i>{waiting ? `waiting · ${waiting.reason ?? 'unknown'}` : 'working'}</i>
      </span>
      <span className="stime">{elapsed(now - since)}</span>
    </div>
  )
}

export default function App() {
  const [sessions, setSessions] = useState<LiveSession[]>([])
  const [usage, setUsage] = useState<UsageSummary | null>(null)
  const [now, setNow] = useState(Date.now())

  useEffect(() => {
    getLiveSessions().then(setSessions).catch(() => setSessions([]))
    getUsageSummary().then(setUsage).catch(() => setUsage(null))
    const un = onSessionsChanged(setSessions)
    const tick = setInterval(() => setNow(Date.now()), 1000)
    return () => {
      un.then((f) => f())
      clearInterval(tick)
    }
  }, [])

  const waiting = sessions.filter((s) => s.status.kind === 'waiting')

  return (
    <div className="popover">
      <div className="stats">
        <div className="stat">
          <div className="lbl">Window</div>
          <div className="num">{usage ? tokens(usage.window.tokens) : '—'}</div>
        </div>
        <div className="stat">
          <div className="lbl">Week</div>
          <div className="num">{usage ? tokens(usage.week.tokens) : '—'}</div>
        </div>
        <div className="stat">
          <div className="lbl">Today</div>
          <div className="num">{usage ? cost(usage.today.costUsd) : '—'}</div>
          {usage && <div className="est">est</div>}
        </div>
      </div>

      {waiting.length > 0 && (
        <div className="banner">
          {waiting.length === 1
            ? `1 session is waiting on you`
            : `${waiting.length} sessions are waiting on you`}
        </div>
      )}

      <div className="lbl section">Live · {sessions.length}</div>
      {sessions.length === 0 && <div className="empty">No sessions running</div>}
      {sessions.map((s) => (
        <SessionRow key={s.sessionId} s={s} now={now} />
      ))}
    </div>
  )
}
```

- [ ] **Step 5: Styles**

Replace `src/styles.css` with a dark popover matching the approved mockup — 340px wide, `#1c1c1e` background, `#f2f2f7` text, three stats in a row, session rows with a coloured dot, an amber banner, and a dashed amber `est` badge. Status colours: working `#30d158`, waiting `#ffd60a`, background `#636366`. Keep it under 100 lines; no framework.

- [ ] **Step 6: Verify by hand**

```bash
source "$HOME/.cargo/env" && cargo tauri dev
```

Verify and report each:
1. The popover shows your real live sessions with correct names.
2. A session in `waiting` shows amber with its reason and an elapsed counter that ticks.
3. The banner appears only when something is waiting.
4. The `est` badge is visible on the spend figure.
5. Starting or stopping a `claude` session updates the popover **without restarting Perch** — this is the live-update property, the point of Task 7.

- [ ] **Step 7: Commit**

```bash
git add src package.json pnpm-lock.yaml
git commit -m "feat(ui): dense popover with live sessions and estimated usage"
```

---

## Task 9: Menu-bar title and CI for the app

**Files:**
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/watcher.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: the tray title showing live/waiting counts, and CI that builds the app.

Spec §9.1 says the menu-bar item shows the window utilization percentage. That percentage requires the tier-1 API (a later milestone), so this plan shows **what it can honestly show**: a count of live sessions, with an hourglass when something is blocked. Do not display a fabricated percentage.

- [ ] **Step 1: Update the tray title from the watcher**

In `src-tauri/src/watcher.rs`, in `emit_now`, after emitting, also set the tray title:

```rust
    let waiting = sessions
        .iter()
        .filter(|s| matches!(s.status, live::SessionStatus::Waiting { .. }))
        .count();
    let title = if waiting > 0 {
        format!("{} ⏳", waiting)
    } else if sessions.is_empty() {
        String::new()
    } else {
        format!("{}", sessions.len())
    };
    if let Some(tray) = app.tray_by_id("perch-tray") {
        let _ = tray.set_title(Some(title));
    }
```

`tray_by_id` requires `use tauri::Manager;`.

- [ ] **Step 2: Extend CI**

Add a job to `.github/workflows/ci.yml` that builds the app on macOS:

```yaml
  app:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: pnpm/action-setup@v4
        with: { version: 10 }
      - uses: actions/setup-node@v4
        with: { node-version: 24, cache: pnpm }
      - run: pnpm install --frozen-lockfile
      - run: pnpm build
      - run: cargo build -p perch-app
```

Keep the existing `test` and `no-network` jobs unchanged. **Verify the `no-network` job's file globs still cover the new `src-tauri` crate** — its guard scans `Cargo.toml crates/*/Cargo.toml`, which does not include `src-tauri/Cargo.toml`. Extend those globs to include it, and extend the source-audit globs to cover `src-tauri/src/**` as well. Add a positive-control self-test for the new coverage, matching the existing style.

This matters: `src-tauri` is exactly where an HTTP client would plausibly be added first.

- [ ] **Step 3: Verify locally**

```bash
source "$HOME/.cargo/env" && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
pnpm build
```

Then run the `no-network` job's script by hand and confirm it still passes and that its self-tests fire.

- [ ] **Step 4: Commit**

```bash
git add src-tauri .github
git commit -m "feat(app): tray title with live and waiting counts; extend CI to the app"
```

---

## Self-Review

**Spec coverage.** §7 (live detection, three confirmations, status model) → Tasks 1–2, proven on real data in Task 3. §4.1 (platform traits) → Task 2. §9.1 (menu bar, dense popover) → Tasks 5, 8, 9. §8's requirement that estimates be marked → Task 6's `source` field and Task 8's `est` badge. §10 (privacy) → Task 7 Step 3 and Task 9's CI extension.

**Deliberately not covered:** §8 tier-1 API client, §9.2 main window, §9.3 usage view, §9.4 jump/resume actions, §9.5 notifications, §10 demo mode and release packaging. Each needs its own plan.

**Known deviations from the spec, recorded rather than hidden:**
- The menu-bar item shows a session count, not the window utilization percentage §9.1 asks for. The percentage requires the tier-1 endpoint. Showing an estimated percentage would violate §8's rule that tier-2 must not fabricate a percentage against an unknown ceiling.
- The popover's three stats show absolute tokens and spend rather than percentages, for the same reason.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — tasks executed in this session with batch checkpoints.
