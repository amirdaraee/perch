# Perch Data Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the headless data layer for Perch — discover Claude Code projects, incrementally index session transcripts into SQLite, and report token/cost totals through a debug CLI.

**Architecture:** A pure-Rust library crate `perch-core` with no Tauri dependency, so every rule is unit-testable with plain `cargo test`. A thin binary crate `perch-cli` exercises the library end-to-end against real data. Transcripts are append-only, so the indexer stores a byte offset per file and re-parses only new bytes. Path resolution reads the `cwd` field inside transcripts rather than decoding directory slugs, which are lossy.

**Tech Stack:** Rust 2021 (rustup, stable), `rusqlite` (bundled SQLite), `serde_json`, `chrono`, `anyhow`, `thiserror`, `clap`, `tempfile` (dev). Node 24 / pnpm 10 exist on the machine but are unused in this plan — the UI arrives in the next plan.

**Spec:** `docs/superpowers/specs/2026-08-30-perch-design.md`

## Global Constraints

- **Scope of this plan:** spec Milestone 1 only (spec §12). Milestones 2–6 (tray, windows, usage API, notifications, release) get their own plans once this layer is proven against real data.
- **Read-only:** nothing in this plan ever writes to, moves, or deletes anything under the Claude Code config directory. Only `perch`'s own app-data directory is written.
- **No network:** this plan makes zero outbound requests. No HTTP client dependency may be added.
- **No telemetry, no analytics, no crash reporting** (spec §10).
- **Rust edition 2021**, stable toolchain. All code must pass `cargo clippy -- -D warnings` and `cargo fmt --check`.
- **Token classes stay separate** — `input`, `output`, `cache_read`, `cache_write_5m`, `cache_write_1h`, `thinking` are never summed into a single number at the storage layer (spec §6).
- **Real paths come from `cwd`, never from decoding a directory slug** (spec §3.1).
- **License:** MIT. Every new file needs no header, but `Cargo.toml` carries `license = "MIT"`.
- **Platform:** macOS arm64 is the development target; nothing in this plan may use a macOS-only API — the whole data layer must compile and test on Linux too.

---

## File Structure

```
perch/
├── Cargo.toml                        # workspace root
├── rust-toolchain.toml               # pins stable channel
├── crates/
│   ├── perch-core/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs                # public API surface, re-exports
│   │   │   ├── config.rs             # config-dir resolution (pure fns)
│   │   │   ├── model.rs              # shared types: TurnUsage, Turn, SessionMeta, Project
│   │   │   ├── transcript.rs         # single-line JSONL parsing
│   │   │   ├── scan.rs               # incremental byte-offset file scanning
│   │   │   ├── discovery.rs          # project discovery + real-path + worktree parents
│   │   │   ├── db.rs                 # schema, migrations, upserts
│   │   │   ├── pricing.rs            # price table + cost math
│   │   │   ├── index.rs              # orchestration: discover → scan → persist
│   │   │   └── query.rs              # read-side aggregation queries
│   │   └── tests/
│   │       ├── fixtures/             # synthetic .jsonl files
│   │       └── incremental.rs        # integration: incremental equivalence
└── crates/
    └── perch-cli/
        ├── Cargo.toml
        └── src/main.rs               # debug commands: projects, sessions, index
```

**Responsibilities.** `transcript.rs` knows JSON shapes and nothing about files. `scan.rs` knows files and offsets and nothing about JSON semantics beyond calling the parser. `db.rs` knows SQL and nothing about parsing. `index.rs` is the only module that composes them. This keeps every rule testable in isolation.

---

## Task 1: Workspace scaffold and toolchain

**Files:**
- Create: `rust-toolchain.toml`
- Create: `Cargo.toml`
- Create: `crates/perch-core/Cargo.toml`
- Create: `crates/perch-core/src/lib.rs`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: nothing.
- Produces: a `perch-core` library crate that compiles and runs tests via `cargo test -p perch-core`.

- [ ] **Step 1: Install the Rust toolchain**

Rust is not installed on this machine. Run:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile default
source "$HOME/.cargo/env"
rustc --version && cargo --version
```

Expected: prints a version of 1.80 or later for both. If `rustc` is still not found in later steps, run `source "$HOME/.cargo/env"` again in that shell.

- [ ] **Step 2: Pin the toolchain**

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Create the workspace manifest**

Create `Cargo.toml` at the repo root:

```toml
[workspace]
resolver = "2"
members = ["crates/perch-core", "crates/perch-cli"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
repository = "https://github.com/amirdaraee/perch"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
rusqlite = { version = "0.32", features = ["bundled"] }
clap = { version = "4", features = ["derive"] }
tempfile = "3"
```

- [ ] **Step 4: Create the core crate manifest**

Create `crates/perch-core/Cargo.toml`:

```toml
[package]
name = "perch-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
anyhow.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
rusqlite.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 5: Create the library entry point**

Create `crates/perch-core/src/lib.rs`:

```rust
//! Perch core: reads Claude Code's on-disk data. Never writes to it.

pub mod config;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
```

Create a placeholder `crates/perch-core/src/config.rs` containing only a doc comment for now:

```rust
//! Resolution of the Claude Code configuration directory.
```

- [ ] **Step 6: Create a stub CLI crate so the workspace resolves**

Create `crates/perch-cli/Cargo.toml`:

```toml
[package]
name = "perch-cli"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
perch-core = { path = "../perch-core" }
anyhow.workspace = true
clap.workspace = true
```

Create `crates/perch-cli/src/main.rs`:

```rust
fn main() {
    println!("perch-cli");
}
```

- [ ] **Step 7: Update .gitignore**

Append to `.gitignore` (it already contains `.superpowers/`, `target/`, `node_modules/`, `dist/`, `*.db`):

```
Cargo.lock.orig
.DS_Store
```

Note: `Cargo.lock` at a workspace root with a binary **should** be committed. Do not ignore it.

- [ ] **Step 8: Verify the workspace builds and tests**

Run: `cargo test --workspace`
Expected: PASS, one test (`workspace_builds`).

Run: `cargo fmt --check && cargo clippy --workspace -- -D warnings`
Expected: no output, exit 0.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml crates .gitignore
git commit -m "chore: scaffold cargo workspace with perch-core and perch-cli"
```

---

## Task 2: Config directory resolution

**Files:**
- Modify: `crates/perch-core/src/config.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn resolve_config_dir(claude_config_dir: Option<&str>, xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf>`
  - `pub fn config_dir() -> Option<PathBuf>` — reads the real environment.
  - `pub fn projects_dir(config: &Path) -> PathBuf`
  - `pub fn sessions_dir(config: &Path) -> PathBuf`
  - `pub fn history_file(config: &Path) -> PathBuf`

Precedence per spec §3: `$CLAUDE_CONFIG_DIR` → `$XDG_CONFIG_HOME/claude` → `$HOME/.claude`.

- [ ] **Step 1: Write the failing tests**

Replace `crates/perch-core/src/config.rs` with:

```rust
//! Resolution of the Claude Code configuration directory.

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_claude_config_dir() {
        let got = resolve_config_dir(Some("/custom/claude"), Some("/xdg"), Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/custom/claude")));
    }

    #[test]
    fn falls_back_to_xdg_config_home() {
        let got = resolve_config_dir(None, Some("/xdg"), Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/xdg/claude")));
    }

    #[test]
    fn falls_back_to_home_dot_claude() {
        let got = resolve_config_dir(None, None, Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/home/u/.claude")));
    }

    #[test]
    fn returns_none_when_nothing_is_set() {
        assert_eq!(resolve_config_dir(None, None, None), None);
    }

    #[test]
    fn ignores_empty_env_values() {
        let got = resolve_config_dir(Some(""), Some(""), Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/home/u/.claude")));
    }

    #[test]
    fn derives_subpaths() {
        let c = Path::new("/home/u/.claude");
        assert_eq!(projects_dir(c), PathBuf::from("/home/u/.claude/projects"));
        assert_eq!(sessions_dir(c), PathBuf::from("/home/u/.claude/sessions"));
        assert_eq!(history_file(c), PathBuf::from("/home/u/.claude/history.jsonl"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core config`
Expected: FAIL — `cannot find function resolve_config_dir in this scope`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/config.rs`:

```rust
fn non_empty(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.trim().is_empty())
}

/// Pure resolution so precedence is testable without touching the process environment.
pub fn resolve_config_dir(
    claude_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(d) = non_empty(claude_config_dir) {
        return Some(PathBuf::from(d));
    }
    if let Some(d) = non_empty(xdg_config_home) {
        return Some(Path::new(d).join("claude"));
    }
    non_empty(home).map(|h| Path::new(h).join(".claude"))
}

/// Reads the real process environment.
pub fn config_dir() -> Option<PathBuf> {
    let claude = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    resolve_config_dir(claude.as_deref(), xdg.as_deref(), home.as_deref())
}

pub fn projects_dir(config: &Path) -> PathBuf {
    config.join("projects")
}

pub fn sessions_dir(config: &Path) -> PathBuf {
    config.join("sessions")
}

pub fn history_file(config: &Path) -> PathBuf {
    config.join("history.jsonl")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p perch-core config`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/perch-core/src/config.rs
git commit -m "feat(core): resolve Claude Code config dir honoring CLAUDE_CONFIG_DIR and XDG"
```

---

## Task 3: Shared types

**Files:**
- Create: `crates/perch-core/src/model.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `TurnUsage`, `Turn`, `SessionMeta`, `SessionRecord` — used by every later task. (No project struct here: the read side defines `ProjectSummary` in Task 10, and an unused duplicate would be dead code.)

- [ ] **Step 1: Write the failing test**

Create `crates/perch-core/src/model.rs`:

```rust
//! Shared value types.

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_componentwise() {
        let a = TurnUsage { input: 1, output: 2, cache_read: 3, cache_write_5m: 4, cache_write_1h: 5, thinking: 6 };
        let b = TurnUsage { input: 10, output: 20, cache_read: 30, cache_write_5m: 40, cache_write_1h: 50, thinking: 60 };
        let sum = a.plus(&b);
        assert_eq!(sum.input, 11);
        assert_eq!(sum.output, 22);
        assert_eq!(sum.cache_read, 33);
        assert_eq!(sum.cache_write_5m, 44);
        assert_eq!(sum.cache_write_1h, 55);
        assert_eq!(sum.thinking, 66);
    }

    #[test]
    fn total_billable_excludes_thinking() {
        // thinking tokens are a subset of output and must not be double counted
        let u = TurnUsage { input: 1, output: 100, cache_read: 10, cache_write_5m: 5, cache_write_1h: 5, thinking: 40 };
        assert_eq!(u.total_tokens(), 121);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p perch-core model`
Expected: FAIL — `failed to resolve: use of undeclared type TurnUsage` (or "unresolved module" until Step 4 registers it).

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/model.rs`:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    /// Subset of `output`. Reported for insight, never added to totals.
    pub thinking: u64,
}

impl TurnUsage {
    pub fn plus(&self, other: &TurnUsage) -> TurnUsage {
        TurnUsage {
            input: self.input + other.input,
            output: self.output + other.output,
            cache_read: self.cache_read + other.cache_read,
            cache_write_5m: self.cache_write_5m + other.cache_write_5m,
            cache_write_1h: self.cache_write_1h + other.cache_write_1h,
            thinking: self.thinking + other.thinking,
        }
    }

    /// Sum of the four billable classes. Excludes `thinking`, which is part of `output`.
    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }

    pub fn cache_write_total(&self) -> u64 {
        self.cache_write_5m + self.cache_write_1h
    }
}

/// One assistant message with usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub ts: i64,
    pub model: String,
    pub usage: TurnUsage,
}

/// Facts about a session gathered while scanning its transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub message_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub project_id: i64,
    pub file_path: String,
    pub file_size: u64,
    pub indexed_offset: u64,
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub message_count: u64,
}
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add below `pub mod config;`:

```rust
pub mod model;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core model`
Expected: PASS, 2 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/model.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): add shared token and session types"
```

---

## Task 4: Transcript line parsing

**Files:**
- Create: `crates/perch-core/src/transcript.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `model::{TurnUsage}`.
- Produces: `pub struct ParsedLine { pub kind: String, pub cwd: Option<String>, pub git_branch: Option<String>, pub cc_version: Option<String>, pub ts: Option<i64>, pub model: Option<String>, pub usage: Option<TurnUsage> }` and `pub fn parse_line(line: &str) -> Option<ParsedLine>`.

Parsing is lenient: unknown fields are ignored, missing fields become `None`, malformed JSON yields `None` rather than an error. The observed shape is:

```json
{"type":"assistant","sessionId":"…","cwd":"/abs/path","gitBranch":"HEAD","version":"2.1.234",
 "timestamp":"2026-08-18T12:16:08.619Z",
 "message":{"model":"claude-fable-5","usage":{"input_tokens":2,"cache_creation_input_tokens":23848,
   "cache_read_input_tokens":24221,"output_tokens":231,
   "output_tokens_details":{"thinking_tokens":79},
   "cache_creation":{"ephemeral_1h_input_tokens":23848,"ephemeral_5m_input_tokens":0}}}}
```

Note the redundancy: `cache_creation_input_tokens` equals the sum of the `cache_creation` breakdown. Prefer the breakdown; fall back to the total assigned to the 5m bucket when the breakdown is absent.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/transcript.rs`:

```rust
//! Parsing of individual transcript JSONL lines. Lenient by design.

use crate::model::TurnUsage;
use serde_json::Value;

#[cfg(test)]
mod tests {
    use super::*;

    const ASSISTANT: &str = r#"{"type":"assistant","sessionId":"s1","cwd":"/Users/a/proj","gitBranch":"main","version":"2.1.234","timestamp":"2026-08-18T12:16:08.619Z","message":{"model":"claude-fable-5","usage":{"input_tokens":2,"cache_creation_input_tokens":23848,"cache_read_input_tokens":24221,"output_tokens":231,"output_tokens_details":{"thinking_tokens":79},"cache_creation":{"ephemeral_1h_input_tokens":23848,"ephemeral_5m_input_tokens":0}}}}"#;

    #[test]
    fn parses_assistant_usage_into_separate_classes() {
        let p = parse_line(ASSISTANT).expect("should parse");
        let u = p.usage.expect("should have usage");
        assert_eq!(u.input, 2);
        assert_eq!(u.output, 231);
        assert_eq!(u.cache_read, 24221);
        assert_eq!(u.cache_write_1h, 23848);
        assert_eq!(u.cache_write_5m, 0);
        assert_eq!(u.thinking, 79);
        assert_eq!(p.model.as_deref(), Some("claude-fable-5"));
    }

    #[test]
    fn extracts_session_metadata() {
        let p = parse_line(ASSISTANT).unwrap();
        assert_eq!(p.cwd.as_deref(), Some("/Users/a/proj"));
        assert_eq!(p.git_branch.as_deref(), Some("main"));
        assert_eq!(p.cc_version.as_deref(), Some("2.1.234"));
        assert!(p.ts.is_some());
    }

    #[test]
    fn parses_timestamp_to_unix_millis() {
        let p = parse_line(ASSISTANT).unwrap();
        // 2026-08-18T12:16:08.619Z
        assert_eq!(p.ts, Some(1787055368619));
    }

    #[test]
    fn falls_back_to_total_cache_creation_when_breakdown_absent() {
        let line = r#"{"type":"assistant","timestamp":"2026-08-18T12:16:08.619Z","message":{"model":"m","usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":40}}}"#;
        let u = parse_line(line).unwrap().usage.unwrap();
        assert_eq!(u.cache_write_5m, 40);
        assert_eq!(u.cache_write_1h, 0);
    }

    #[test]
    fn user_line_has_metadata_but_no_usage() {
        let line = r#"{"type":"user","cwd":"/Users/a/proj","timestamp":"2026-08-18T12:00:00.000Z","message":{"role":"user","content":"hi"}}"#;
        let p = parse_line(line).unwrap();
        assert!(p.usage.is_none());
        assert_eq!(p.kind, "user");
        assert_eq!(p.cwd.as_deref(), Some("/Users/a/proj"));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let line = r#"{"type":"assistant","brandNewField":{"x":1},"timestamp":"2026-08-18T12:16:08.619Z","message":{"model":"m","usage":{"input_tokens":5,"output_tokens":6,"somethingNew":true}}}"#;
        let u = parse_line(line).unwrap().usage.unwrap();
        assert_eq!(u.input, 5);
        assert_eq!(u.output, 6);
    }

    #[test]
    fn returns_none_for_malformed_json() {
        assert!(parse_line("{not json").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
    }

    #[test]
    fn returns_none_for_json_that_is_not_an_object() {
        assert!(parse_line("[1,2,3]").is_none());
        assert!(parse_line("\"a string\"").is_none());
    }

    #[test]
    fn handles_meta_lines_without_type() {
        let line = r#"{"leafUuid":"x","sessionId":"s1"}"#;
        let p = parse_line(line).unwrap();
        assert_eq!(p.kind, "");
        assert!(p.usage.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core transcript`
Expected: FAIL — `cannot find function parse_line`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/transcript.rs`:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedLine {
    pub kind: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub ts: Option<i64>,
    pub model: Option<String>,
    pub usage: Option<TurnUsage>,
}

fn u64_at(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn string_at(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
    }

fn parse_ts(v: &Value) -> Option<i64> {
    let raw = v.get("timestamp")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn parse_usage(message: &Value) -> Option<TurnUsage> {
    let usage = message.get("usage")?;
    if !usage.is_object() {
        return None;
    }

    let (w5, w1h) = match usage.get("cache_creation") {
        Some(c) if c.is_object() => (
            u64_at(c, "ephemeral_5m_input_tokens"),
            u64_at(c, "ephemeral_1h_input_tokens"),
        ),
        // No breakdown available: attribute the total to the 5m bucket.
        _ => (u64_at(usage, "cache_creation_input_tokens"), 0),
    };

    let thinking = usage
        .get("output_tokens_details")
        .map(|d| u64_at(d, "thinking_tokens"))
        .unwrap_or(0);

    Some(TurnUsage {
        input: u64_at(usage, "input_tokens"),
        output: u64_at(usage, "output_tokens"),
        cache_read: u64_at(usage, "cache_read_input_tokens"),
        cache_write_5m: w5,
        cache_write_1h: w1h,
        thinking,
    })
}

/// Parse one transcript line. Returns `None` for blank or malformed lines,
/// which callers count and skip rather than treating as fatal.
pub fn parse_line(line: &str) -> Option<ParsedLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    if !v.is_object() {
        return None;
    }

    let message = v.get("message");
    Some(ParsedLine {
        kind: v.get("type").and_then(Value::as_str).unwrap_or("").to_owned(),
        cwd: string_at(&v, "cwd"),
        git_branch: string_at(&v, "gitBranch"),
        cc_version: string_at(&v, "version"),
        ts: parse_ts(&v),
        model: message.and_then(|m| string_at(m, "model")),
        usage: message.and_then(parse_usage),
    })
}
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod transcript;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core transcript`
Expected: PASS, 9 tests.

The constant `1787055368619` is the verified epoch-millis value for `2026-08-18T12:16:08.619Z`. If that test fails, the parser is wrong — do not change the constant.

- [ ] **Step 6: Verify against a real transcript**

Run:

```bash
cargo test -p perch-core transcript -- --nocapture
head -c 2000 "$(ls -S ~/.claude/projects/*/*.jsonl | head -1)" | head -1 > /tmp/perch-real-line.json
wc -c /tmp/perch-real-line.json
```

This confirms real lines exist and are large; no assertion here, it is a sanity check before the integration task.

- [ ] **Step 7: Commit**

```bash
git add crates/perch-core/src/transcript.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): parse transcript lines with four separate token classes"
```

---

## Task 5: Incremental byte-offset scanning

**Files:**
- Create: `crates/perch-core/src/scan.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `transcript::parse_line`, `model::{Turn, SessionMeta}`.
- Produces: `pub struct ScanOutcome { pub turns: Vec<Turn>, pub meta: SessionMeta, pub new_offset: u64, pub lines_parsed: u64, pub lines_skipped: u64 }` and `pub fn scan_from(path: &Path, offset: u64) -> std::io::Result<ScanOutcome>`.

This is the load-bearing task of the whole plan. Two rules:

1. **Never advance past the last complete line.** A live session is being appended to while we read; the final line may be truncated. Consume only up to and including the last `\n` and set `new_offset` to exactly that point.
2. **Truncation resets.** If the file is now smaller than the stored offset, the file was replaced — rescan from 0.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/scan.rs`:

```rust
//! Incremental scanning of append-only transcript files.

use crate::model::{SessionMeta, Turn};
use crate::transcript::parse_line;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

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
        write_lines(&p, &[
            assistant("2026-08-18T10:00:00.000Z", 1, 2),
            assistant("2026-08-18T10:01:00.000Z", 3, 4),
        ]);

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
        write_lines(&p, &[
            assistant("2026-08-18T10:00:00.000Z", 1, 2),
            assistant("2026-08-18T10:01:00.000Z", 3, 4),
        ]);

        let first = scan_from(&p, 0).unwrap();

        append_lines(&p, &[
            assistant("2026-08-18T10:02:00.000Z", 5, 6),
            assistant("2026-08-18T10:03:00.000Z", 7, 8),
        ]);

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
        assert_eq!(out.new_offset, complete_len, "offset must stop at the last newline");
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
        write_lines(&p, &[
            assistant("2026-08-18T10:00:00.000Z", 1, 2),
            "{ not json".to_string(),
            assistant("2026-08-18T10:02:00.000Z", 5, 6),
        ]);

        let out = scan_from(&p, 0).unwrap();
        assert_eq!(out.turns.len(), 2);
        assert_eq!(out.lines_skipped, 1);
        assert_eq!(out.lines_parsed, 2);
    }

    #[test]
    fn rescans_from_zero_when_file_shrank() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(&p, &[
            assistant("2026-08-18T10:00:00.000Z", 1, 2),
            assistant("2026-08-18T10:01:00.000Z", 3, 4),
        ]);
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
        write_lines(&p, &[
            assistant("2026-08-18T10:00:00.000Z", 1, 2),
            assistant("2026-08-18T12:00:00.000Z", 3, 4),
        ]);
        let out = scan_from(&p, 0).unwrap();
        assert!(out.meta.first_ts.unwrap() < out.meta.last_ts.unwrap());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core scan`
Expected: FAIL — `cannot find function scan_from`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/scan.rs`:

```rust
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
        return Ok(ScanOutcome { new_offset: start, ..Default::default() });
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
            return Ok(ScanOutcome { new_offset: start, ..Default::default() });
        }
    };
    let complete = &buf[..=last_newline];
    let new_offset = start + complete.len() as u64;

    let mut out = ScanOutcome { new_offset, ..Default::default() };

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
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod scan;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core scan`
Expected: PASS, 8 tests. `incremental_scan_equals_full_scan` and `does_not_consume_a_partial_final_line` are the two that matter most — if either fails, do not proceed.

- [ ] **Step 6: Benchmark against the largest real transcript**

Run:

```bash
cargo test -p perch-core --release scan
ls -S ~/.claude/projects/*/*.jsonl | head -1
```

Then add this temporary check to confirm real-world performance, run it, and delete it:

```bash
cat > /tmp/perch_bench.rs <<'EOF'
// throwaway
EOF
echo "manual: note the largest file size; full-scan cost is measured in Task 9"
```

No assertion. This step exists to make the file size concrete before integration.

- [ ] **Step 7: Commit**

```bash
git add crates/perch-core/src/scan.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): incremental transcript scanning with partial-line safety"
```

---

## Task 6: Project discovery and real-path resolution

**Files:**
- Create: `crates/perch-core/src/discovery.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `scan::scan_from` (indirectly via reading a first line), `config::projects_dir`.
- Produces:
  - `pub struct DiscoveredProject { pub slug: String, pub dir: PathBuf, pub real_path: String, pub path_is_guess: bool, pub session_files: Vec<PathBuf> }`
  - `pub fn discover(projects_root: &Path) -> std::io::Result<Vec<DiscoveredProject>>`
  - `pub fn real_path_for(project_dir: &Path) -> Option<String>` — reads `cwd` from the first parseable line of any session file.
  - `pub fn slug_to_guess(slug: &str) -> String` — last resort only.
  - `pub fn parent_path_of(real_path: &str) -> Option<String>` — truncates at `/.claude/worktrees/`.

Per spec §3.1, slug decoding is **never** used when a `cwd` is available.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/discovery.rs`:

```rust
//! Project discovery. Real paths come from transcript `cwd`, never from slugs.

use crate::transcript::parse_line;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_project(root: &Path, slug: &str, cwd: Option<&str>) -> PathBuf {
        let dir = root.join(slug);
        fs::create_dir_all(&dir).unwrap();
        if let Some(cwd) = cwd {
            let f = dir.join("11111111-1111-1111-1111-111111111111.jsonl");
            let mut fh = fs::File::create(f).unwrap();
            // First line is a meta line with no cwd, to prove we keep looking.
            writeln!(fh, r#"{{"type":"last-prompt","leafUuid":"x"}}"#).unwrap();
            writeln!(
                fh,
                r#"{{"type":"user","cwd":"{cwd}","timestamp":"2026-08-18T10:00:00.000Z"}}"#
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn reads_real_path_from_cwd_not_from_slug() {
        let tmp = tempfile::tempdir().unwrap();
        // This slug is genuinely ambiguous: the true path contains a dot.
        let dir = make_project(
            tmp.path(),
            "-Users-a-00-projects-amirdaraee-github-io",
            Some("/Users/a/00/projects/amirdaraee.github.io"),
        );
        assert_eq!(
            real_path_for(&dir).as_deref(),
            Some("/Users/a/00/projects/amirdaraee.github.io")
        );
    }

    #[test]
    fn slug_guess_is_only_a_fallback_and_is_marked() {
        let tmp = tempfile::tempdir().unwrap();
        make_project(tmp.path(), "-Users-a-00-projects-empty-one", None);
        let found = discover(tmp.path()).unwrap();
        let p = found.iter().find(|p| p.slug.ends_with("empty-one")).unwrap();
        assert!(p.path_is_guess, "a project with no readable cwd must be flagged");
        assert_eq!(p.real_path, "/Users/a/00/projects/empty/one");
    }

    #[test]
    fn slug_guess_replaces_leading_and_inner_dashes_with_slashes() {
        assert_eq!(slug_to_guess("-Users-a-proj"), "/Users/a/proj");
        assert_eq!(slug_to_guess("Users-a-proj"), "Users/a/proj");
    }

    #[test]
    fn discovers_projects_and_their_session_files() {
        let tmp = tempfile::tempdir().unwrap();
        make_project(tmp.path(), "-Users-a-one", Some("/Users/a/one"));
        make_project(tmp.path(), "-Users-a-two", Some("/Users/a/two"));

        let mut found = discover(tmp.path()).unwrap();
        found.sort_by(|a, b| a.slug.cmp(&b.slug));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].real_path, "/Users/a/one");
        assert_eq!(found[0].session_files.len(), 1);
        assert!(!found[0].path_is_guess);
    }

    #[test]
    fn ignores_non_directories_and_non_jsonl_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_project(tmp.path(), "-Users-a-one", Some("/Users/a/one"));
        fs::write(dir.join("notes.txt"), "hello").unwrap();
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(tmp.path().join("stray.json"), "{}").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_files.len(), 1);
    }

    #[test]
    fn detects_worktree_parent_path() {
        assert_eq!(
            parent_path_of("/Users/a/00/projects/personal-dashboard/.claude/worktrees/finance-section").as_deref(),
            Some("/Users/a/00/projects/personal-dashboard")
        );
        assert_eq!(parent_path_of("/Users/a/00/projects/personal-dashboard"), None);
    }

    #[test]
    fn missing_root_yields_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let found = discover(&tmp.path().join("does-not-exist")).unwrap();
        assert!(found.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core discovery`
Expected: FAIL — `cannot find function discover`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/discovery.rs`:

```rust
pub const WORKTREE_MARKER: &str = "/.claude/worktrees/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredProject {
    pub slug: String,
    pub dir: PathBuf,
    pub real_path: String,
    /// True when `real_path` was guessed from the slug because no session
    /// file carried a `cwd`. The UI must mark these as uncertain.
    pub path_is_guess: bool,
    pub session_files: Vec<PathBuf>,
}

/// Last-resort decoding. Lossy: the encoder flattens both `/` and `.` to `-`,
/// so this can only ever be a guess. Never call it when a `cwd` is available.
pub fn slug_to_guess(slug: &str) -> String {
    slug.replace('-', "/")
}

/// A worktree lives at `<parent>/.claude/worktrees/<name>`.
pub fn parent_path_of(real_path: &str) -> Option<String> {
    real_path
        .find(WORKTREE_MARKER)
        .map(|i| real_path[..i].to_string())
}

/// Read the first `cwd` found in any session file in this directory.
pub fn real_path_for(project_dir: &Path) -> Option<String> {
    for file in session_files_in(project_dir) {
        let Ok(fh) = fs::File::open(&file) else { continue };
        // Transcript lines can be very large; cap how many we inspect.
        for line in BufReader::new(fh).lines().take(50) {
            let Ok(line) = line else { break };
            if let Some(parsed) = parse_line(&line) {
                if let Some(cwd) = parsed.cwd {
                    return Some(cwd);
                }
            }
        }
    }
    None
}

fn session_files_in(project_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(project_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    files
}

pub fn discover(projects_root: &Path) -> std::io::Result<Vec<DiscoveredProject>> {
    let entries = match fs::read_dir(projects_root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(slug) = dir.file_name().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };

        let session_files = session_files_in(&dir);
        let (real_path, path_is_guess) = match real_path_for(&dir) {
            Some(cwd) => (cwd, false),
            None => (slug_to_guess(&slug), true),
        };

        out.push(DiscoveredProject { slug, dir, real_path, path_is_guess, session_files });
    }
    Ok(out)
}
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod discovery;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core discovery`
Expected: PASS, 7 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/discovery.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): discover projects using transcript cwd instead of lossy slugs"
```

---

## Task 7: Database schema and persistence

**Files:**
- Create: `crates/perch-core/src/db.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `model::{Turn, TurnUsage}`.
- Produces:
  - `pub struct Db { conn: rusqlite::Connection }`
  - `pub fn open(path: &Path) -> anyhow::Result<Db>` and `pub fn open_in_memory() -> anyhow::Result<Db>`
  - `pub fn upsert_project(&self, slug: &str, real_path: &str, path_is_guess: bool) -> anyhow::Result<i64>`
  - `pub fn set_parent(&self, child_id: i64, parent_id: Option<i64>) -> anyhow::Result<()>`
  - `pub fn set_note(&self, project_id: i64, note: &str) -> anyhow::Result<()>`
  - `pub fn upsert_session(&self, s: &SessionRecord) -> anyhow::Result<()>`
  - `pub fn session_offset(&self, session_id: &str) -> anyhow::Result<u64>`
  - `pub fn insert_turns(&self, session_id: &str, turns: &[Turn]) -> anyhow::Result<()>`
  - `pub fn rebuild_derived(&self) -> anyhow::Result<()>` — drops `sessions` and `turns` rows only.

The critical property, from spec §6: **re-indexing must never destroy user-owned columns.**

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/db.rs`:

```rust
//! SQLite persistence. User-owned columns survive re-indexing.

use crate::model::{SessionRecord, Turn};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 1;

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
        let b = db.upsert_project("slug", "/Users/a/proj-moved", false).unwrap();
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

        assert_eq!(db.note(id).unwrap().as_deref(), Some("left off on the CSV parser"));
        assert!(db.pinned(id).unwrap());
        assert_eq!(db.display_name(id).unwrap().as_deref(), Some("Personal Dashboard"));
    }

    #[test]
    fn rebuild_derived_clears_sessions_and_turns_but_not_projects() {
        let db = open_in_memory().unwrap();
        let id = db.upsert_project("slug", "/Users/a/proj", false).unwrap();
        db.set_note(id, "keep me").unwrap();
        db.upsert_session(&session("s1", id, 10)).unwrap();
        db.insert_turns("s1", &[Turn { ts: 1, model: "m".into(), usage: TurnUsage { input: 1, ..Default::default() } }]).unwrap();

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
        db.insert_turns("s1", &[Turn {
            ts: 1_700_000_000_000,
            model: "claude-fable-5".into(),
            usage: TurnUsage { input: 2, output: 231, cache_read: 24221, cache_write_5m: 0, cache_write_1h: 23848, thinking: 79 },
        }]).unwrap();

        let (i, o, r, w5, w1) = db.turn_totals().unwrap();
        assert_eq!((i, o, r, w5, w1), (2, 231, 24221, 0, 23848));
    }

    #[test]
    fn parent_project_can_be_set() {
        let db = open_in_memory().unwrap();
        let parent = db.upsert_project("p", "/Users/a/proj", false).unwrap();
        let child = db.upsert_project("c", "/Users/a/proj/.claude/worktrees/x", false).unwrap();
        db.set_parent(child, Some(parent)).unwrap();
        assert_eq!(db.parent_of(child).unwrap(), Some(parent));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core db`
Expected: FAIL — `cannot find function open_in_memory`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/db.rs`:

```rust
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
        Ok(self
            .conn
            .query_row("SELECT id FROM projects WHERE slug = ?1", params![slug], |r| r.get(0))?)
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
        Ok(self
            .conn
            .query_row("SELECT note FROM projects WHERE id = ?1", params![project_id], |r| r.get(0))?)
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
                s.id, s.project_id, s.file_path, s.file_size as i64, s.indexed_offset as i64,
                s.started_at, s.last_activity_at, s.cwd, s.git_branch, s.cc_version,
                s.message_count as i64
            ],
        )?;
        Ok(())
    }

    pub fn session_offset(&self, session_id: &str) -> Result<u64> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT indexed_offset FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok();
        Ok(v.unwrap_or(0).max(0) as u64)
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
                    session_id, t.ts, t.model,
                    t.usage.input as i64, t.usage.output as i64, t.usage.cache_read as i64,
                    t.usage.cache_write_5m as i64, t.usage.cache_write_1h as i64,
                    t.usage.thinking as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Drop derived rows only. Never touches `projects`.
    pub fn rebuild_derived(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM turns; DELETE FROM sessions;")?;
        Ok(())
    }

    pub fn project_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?)
    }

    pub fn session_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?)
    }

    pub fn turn_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))?)
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
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod db;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core db`
Expected: PASS, 9 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/db.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): SQLite schema that preserves user-owned columns across reindexing"
```

---

## Task 8: Pricing and cost estimation

**Files:**
- Create: `crates/perch-core/src/pricing.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `model::TurnUsage`, `db::Db`.
- Produces:
  - `pub struct ModelPrice { pub input_per_mtok: f64, pub output_per_mtok: f64, pub cache_read_per_mtok: f64, pub cache_write_per_mtok: f64 }`
  - `pub fn seed_default_prices(db: &Db) -> anyhow::Result<()>`
  - `pub fn price_for(db: &Db, model: &str) -> anyhow::Result<Option<ModelPrice>>`
  - `pub fn cost_usd(price: &ModelPrice, usage: &TurnUsage) -> f64`

Per spec §8: unknown models are **counted with zero price**, never dropped. Prices live in a table so a change is a row update, not a release.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/pricing.rs`:

```rust
//! Estimated dollar cost. Tokens are truth; dollars are an estimate.

use crate::db::Db;
use crate::model::TurnUsage;
use anyhow::Result;
use rusqlite::params;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn cost_prices_each_class_separately() {
        let p = ModelPrice {
            input_per_mtok: 10.0,
            output_per_mtok: 50.0,
            cache_read_per_mtok: 1.0,
            cache_write_per_mtok: 12.5,
        };
        let u = TurnUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write_5m: 500_000,
            cache_write_1h: 500_000,
            thinking: 900_000,
        };
        // 10 + 50 + 1 + 12.5 = 73.5 ; thinking must not add anything
        assert!((cost_usd(&p, &u) - 73.5).abs() < 1e-9);
    }

    #[test]
    fn zero_usage_costs_nothing() {
        let p = ModelPrice { input_per_mtok: 10.0, output_per_mtok: 50.0, cache_read_per_mtok: 1.0, cache_write_per_mtok: 12.5 };
        assert_eq!(cost_usd(&p, &TurnUsage::default()), 0.0);
    }

    #[test]
    fn seeded_prices_are_readable() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let p = price_for(&db, "claude-fable-5").unwrap();
        assert!(p.is_some(), "the default model must be seeded");
    }

    #[test]
    fn unknown_model_has_no_price_and_is_not_an_error() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        assert!(price_for(&db, "some-future-model").unwrap().is_none());
    }

    #[test]
    fn seeding_twice_does_not_duplicate_or_fail() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        seed_default_prices(&db).unwrap();
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM prices WHERE model = 'claude-fable-5'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn a_user_edited_price_survives_reseeding() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        db.conn().execute("UPDATE prices SET input_per_mtok = 99.0 WHERE model = 'claude-fable-5'", []).unwrap();
        seed_default_prices(&db).unwrap();
        assert_eq!(price_for(&db, "claude-fable-5").unwrap().unwrap().input_per_mtok, 99.0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core pricing`
Expected: FAIL — `cannot find struct ModelPrice`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/pricing.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

/// USD per million tokens. These are estimates for display only and are
/// user-editable at runtime; seeding never overwrites an existing row.
const DEFAULTS: &[(&str, ModelPrice)] = &[
    ("claude-fable-5", ModelPrice { input_per_mtok: 15.0, output_per_mtok: 75.0, cache_read_per_mtok: 1.5, cache_write_per_mtok: 18.75 }),
    ("claude-opus-5", ModelPrice { input_per_mtok: 15.0, output_per_mtok: 75.0, cache_read_per_mtok: 1.5, cache_write_per_mtok: 18.75 }),
    ("claude-sonnet-5", ModelPrice { input_per_mtok: 3.0, output_per_mtok: 15.0, cache_read_per_mtok: 0.3, cache_write_per_mtok: 3.75 }),
    ("claude-haiku-4-5-20251001", ModelPrice { input_per_mtok: 1.0, output_per_mtok: 5.0, cache_read_per_mtok: 0.1, cache_write_per_mtok: 1.25 }),
];

pub fn seed_default_prices(db: &Db) -> Result<()> {
    for (model, p) in DEFAULTS {
        db.conn().execute(
            "INSERT INTO prices (model, input_per_mtok, output_per_mtok,
                 cache_read_per_mtok, cache_write_per_mtok)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(model) DO NOTHING",
            params![model, p.input_per_mtok, p.output_per_mtok, p.cache_read_per_mtok, p.cache_write_per_mtok],
        )?;
    }
    Ok(())
}

pub fn price_for(db: &Db, model: &str) -> Result<Option<ModelPrice>> {
    let found = db.conn().query_row(
        "SELECT input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok
         FROM prices WHERE model = ?1",
        params![model],
        |r| {
            Ok(ModelPrice {
                input_per_mtok: r.get(0)?,
                output_per_mtok: r.get(1)?,
                cache_read_per_mtok: r.get(2)?,
                cache_write_per_mtok: r.get(3)?,
            })
        },
    );
    match found {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

const MTOK: f64 = 1_000_000.0;

/// `thinking` is a subset of `output` and is deliberately not priced again.
pub fn cost_usd(price: &ModelPrice, usage: &TurnUsage) -> f64 {
    (usage.input as f64 / MTOK) * price.input_per_mtok
        + (usage.output as f64 / MTOK) * price.output_per_mtok
        + (usage.cache_read as f64 / MTOK) * price.cache_read_per_mtok
        + (usage.cache_write_total() as f64 / MTOK) * price.cache_write_per_mtok
}
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod pricing;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core pricing`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/pricing.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): editable price table with per-class cost estimation"
```

---

## Task 9: Index orchestration

**Files:**
- Create: `crates/perch-core/src/index.rs`
- Create: `crates/perch-core/tests/incremental.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `discovery::discover`, `scan::scan_from`, `db::Db`, `config`.
- Produces:
  - `pub struct IndexStats { pub projects: usize, pub sessions: usize, pub new_turns: usize, pub bytes_read: u64, pub lines_skipped: u64 }`
  - `pub fn index_all(db: &Db, projects_root: &Path) -> anyhow::Result<IndexStats>`

Session id is the transcript filename stem. Worktree parents are linked in a second pass, after every project has an id.

- [ ] **Step 1: Write the failing integration test**

Create `crates/perch-core/tests/incremental.rs`:

```rust
use perch_core::db::open_in_memory;
use perch_core::index::index_all;
use std::fs;
use std::io::Write;
use std::path::Path;

fn assistant(ts: &str, cwd: &str, input: u64, output: u64) -> String {
    format!(
        r#"{{"type":"assistant","cwd":"{cwd}","gitBranch":"main","version":"2.1.1","timestamp":"{ts}","message":{{"model":"claude-fable-5","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
    )
}

fn write_session(root: &Path, slug: &str, id: &str, lines: &[String]) {
    let dir = root.join(slug);
    fs::create_dir_all(&dir).unwrap();
    let mut f = fs::File::create(dir.join(format!("{id}.jsonl"))).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

fn append_session(root: &Path, slug: &str, id: &str, lines: &[String]) {
    let p = root.join(slug).join(format!("{id}.jsonl"));
    let mut f = fs::File::options().append(true).open(p).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

#[test]
fn indexes_projects_sessions_and_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
        assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4),
    ]);

    let db = open_in_memory().unwrap();
    let stats = index_all(&db, root).unwrap();

    assert_eq!(stats.projects, 1);
    assert_eq!(stats.sessions, 1);
    assert_eq!(stats.new_turns, 2);
    assert_eq!(db.turn_count().unwrap(), 2);
}

/// The property the whole performance strategy rests on.
#[test]
fn reindexing_after_append_adds_only_new_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
    ]);

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 1);

    append_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4),
    ]);

    let stats = index_all(&db, root).unwrap();
    assert_eq!(stats.new_turns, 1, "must index only the appended turn");
    assert_eq!(db.turn_count().unwrap(), 2, "must not duplicate the first turn");
}

#[test]
fn reindexing_unchanged_data_is_a_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
    ]);

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    let stats = index_all(&db, root).unwrap();

    assert_eq!(stats.new_turns, 0);
    assert_eq!(stats.bytes_read, 0);
    assert_eq!(db.turn_count().unwrap(), 1);
}

#[test]
fn links_worktrees_to_their_parent_project() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(root, "-Users-a-proj", "aaaa", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/proj", 1, 2),
    ]);
    write_session(root, "-Users-a-proj--claude-worktrees-feature", "bbbb", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/proj/.claude/worktrees/feature", 1, 2),
    ]);

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();

    let parent_id: i64 = db.conn()
        .query_row("SELECT id FROM projects WHERE real_path = '/Users/a/proj'", [], |r| r.get(0))
        .unwrap();
    let child_parent: Option<i64> = db.conn()
        .query_row(
            "SELECT parent_project_id FROM projects WHERE real_path LIKE '%worktrees/feature'",
            [], |r| r.get(0),
        )
        .unwrap();
    assert_eq!(child_parent, Some(parent_id));
}

#[test]
fn a_truncated_transcript_is_rescanned_without_duplicating_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
        assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4),
        assistant("2026-08-18T10:02:00.000Z", "/Users/a/one", 5, 6),
    ]);

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 3);

    // The file is replaced by a shorter one — the stored offset is now meaningless.
    write_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T11:00:00.000Z", "/Users/a/one", 9, 9),
    ]);

    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 1, "stale turns must be dropped, not duplicated");

    let message_count: i64 = db.conn()
        .query_row("SELECT message_count FROM sessions WHERE id = 'aaaa'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(message_count, 1, "message count must not accumulate across a rescan");
}

#[test]
fn user_notes_survive_a_reindex() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(root, "-Users-a-one", "aaaa", &[
        assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
    ]);

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    let id: i64 = db.conn().query_row("SELECT id FROM projects LIMIT 1", [], |r| r.get(0)).unwrap();
    db.set_note(id, "where I left off").unwrap();

    index_all(&db, root).unwrap();
    assert_eq!(db.note(id).unwrap().as_deref(), Some("where I left off"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p perch-core --test incremental`
Expected: FAIL — `unresolved import perch_core::index`.

- [ ] **Step 3: Write the implementation**

Create `crates/perch-core/src/index.rs`:

```rust
//! Composes discovery, scanning, and persistence into one indexing pass.

use crate::db::Db;
use crate::discovery::{discover, parent_path_of};
use crate::model::SessionRecord;
use crate::scan::scan_from;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub projects: usize,
    pub sessions: usize,
    pub new_turns: usize,
    pub bytes_read: u64,
    pub lines_skipped: u64,
}

pub fn index_all(db: &Db, projects_root: &Path) -> Result<IndexStats> {
    let projects = discover(projects_root)?;
    let mut stats = IndexStats { projects: projects.len(), ..Default::default() };

    // Pass 1: projects, sessions, turns.
    let mut ids_by_path: HashMap<String, i64> = HashMap::new();
    let mut worktrees: Vec<(i64, String)> = Vec::new();

    for project in &projects {
        let project_id = db.upsert_project(&project.slug, &project.real_path, project.path_is_guess)?;
        ids_by_path.insert(project.real_path.clone(), project_id);

        if let Some(parent) = parent_path_of(&project.real_path) {
            worktrees.push((project_id, parent));
        }

        for file in &project.session_files {
            let Some(session_id) = file.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let offset = db.session_offset(session_id)?;
            let file_size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);

            // `scan_from` restarts at 0 when the file shrank. The already-stored
            // turns for this session are then stale duplicates and must be dropped,
            // and the message count must not accumulate on top of them.
            let restarted = file_size < offset;
            if restarted {
                db.delete_turns_for_session(session_id)?;
            }

            let outcome = match scan_from(file, offset) {
                Ok(o) => o,
                Err(_) => continue, // unreadable file: skip, never fatal
            };

            stats.sessions += 1;
            stats.new_turns += outcome.turns.len();
            stats.bytes_read += outcome
                .new_offset
                .saturating_sub(if restarted { 0 } else { offset });
            stats.lines_skipped += outcome.lines_skipped;

            let existing_messages = if offset == 0 || restarted {
                0
            } else {
                db.session_message_count(session_id)?
            };

            db.upsert_session(&SessionRecord {
                id: session_id.to_string(),
                project_id,
                file_path: file.to_string_lossy().into_owned(),
                file_size,
                indexed_offset: outcome.new_offset,
                started_at: outcome.meta.first_ts,
                last_activity_at: outcome.meta.last_ts,
                cwd: outcome.meta.cwd.clone(),
                git_branch: outcome.meta.git_branch.clone(),
                cc_version: outcome.meta.cc_version.clone(),
                message_count: existing_messages + outcome.meta.message_count,
            })?;

            db.insert_turns(session_id, &outcome.turns)?;
        }
    }

    // Pass 2: link worktrees, now that every project has an id.
    for (child_id, parent_path) in worktrees {
        let parent_id = ids_by_path.get(&parent_path).copied();
        db.set_parent(child_id, parent_id)?;
    }

    Ok(stats)
}
```

- [ ] **Step 4: Add the two helpers the orchestrator needs**

In `crates/perch-core/src/db.rs`, add inside `impl Db`:

```rust
    pub fn session_message_count(&self, session_id: &str) -> Result<u64> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT message_count FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok();
        Ok(v.unwrap_or(0).max(0) as u64)
    }

    /// Used when a transcript was truncated or replaced and must be rescanned
    /// from zero: the previously stored turns are stale duplicates.
    pub fn delete_turns_for_session(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM turns WHERE session_id = ?1", params![session_id])?;
        Ok(())
    }
```

- [ ] **Step 5: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod index;
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p perch-core --test incremental`
Expected: PASS, 6 tests.

Run: `cargo test --workspace`
Expected: PASS, all tests.

- [ ] **Step 7: Commit**

```bash
git add crates/perch-core/src/index.rs crates/perch-core/src/db.rs crates/perch-core/src/lib.rs crates/perch-core/tests/incremental.rs
git commit -m "feat(core): index orchestration with incremental turn ingestion"
```

---

## Task 10: Read-side aggregation queries

**Files:**
- Create: `crates/perch-core/src/query.rs`
- Modify: `crates/perch-core/src/lib.rs`

**Interfaces:**
- Consumes: `db::Db`, `pricing`.
- Produces:
  - `pub struct ProjectSummary { pub id: i64, pub slug: String, pub real_path: String, pub display_name: Option<String>, pub parent_project_id: Option<i64>, pub path_is_guess: bool, pub sessions: i64, pub usage: TurnUsage, pub cost_usd: f64, pub last_activity_at: Option<i64> }`
  - `pub fn project_summaries(db: &Db) -> anyhow::Result<Vec<ProjectSummary>>`
  - `pub fn usage_since(db: &Db, since_ms: i64) -> anyhow::Result<(TurnUsage, f64)>`
  - `pub fn usage_by_model(db: &Db) -> anyhow::Result<Vec<(String, TurnUsage, f64)>>`

`usage_since` is what serves both clocks from spec §6 — a trailing 5-hour window and a trailing week are the same query with different arguments.

- [ ] **Step 1: Write the failing tests**

Create `crates/perch-core/src/query.rs`:

```rust
//! Read-side aggregation.

use crate::db::Db;
use crate::model::TurnUsage;
use crate::pricing::{cost_usd, price_for};
use anyhow::Result;
use rusqlite::params;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn};
    use crate::pricing::seed_default_prices;

    fn setup() -> Db {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-Users-a-one", "/Users/a/one", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "s1".into(), project_id: pid, file_path: "/tmp/s1.jsonl".into(),
            file_size: 0, indexed_offset: 0, started_at: Some(1_000), last_activity_at: Some(2_000),
            cwd: None, git_branch: None, cc_version: None, message_count: 2,
        }).unwrap();
        db.insert_turns("s1", &[
            Turn { ts: 1_000, model: "claude-fable-5".into(), usage: TurnUsage { input: 1_000_000, ..Default::default() } },
            Turn { ts: 5_000, model: "claude-sonnet-5".into(), usage: TurnUsage { output: 1_000_000, ..Default::default() } },
        ]).unwrap();
        db
    }

    #[test]
    fn summarizes_a_project_with_tokens_and_cost() {
        let db = setup();
        let rows = project_summaries(&db).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.sessions, 1);
        assert_eq!(r.usage.input, 1_000_000);
        assert_eq!(r.usage.output, 1_000_000);
        // fable input 15.0 + sonnet output 15.0
        assert!((r.cost_usd - 30.0).abs() < 1e-9);
        assert_eq!(r.last_activity_at, Some(5_000));
    }

    #[test]
    fn usage_since_filters_by_timestamp() {
        let db = setup();
        let (usage, cost) = usage_since(&db, 4_000).unwrap();
        assert_eq!(usage.input, 0);
        assert_eq!(usage.output, 1_000_000);
        assert!((cost - 15.0).abs() < 1e-9);
    }

    #[test]
    fn usage_since_with_zero_returns_everything() {
        let db = setup();
        let (usage, _) = usage_since(&db, 0).unwrap();
        assert_eq!(usage.input, 1_000_000);
        assert_eq!(usage.output, 1_000_000);
    }

    #[test]
    fn breaks_usage_down_by_model() {
        let db = setup();
        let mut rows = usage_by_model(&db).unwrap();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "claude-fable-5");
        assert_eq!(rows[0].1.input, 1_000_000);
    }

    #[test]
    fn unknown_model_is_counted_with_zero_cost() {
        let db = setup();
        db.insert_turns("s1", &[Turn {
            ts: 6_000, model: "model-from-the-future".into(),
            usage: TurnUsage { input: 2_000_000, ..Default::default() },
        }]).unwrap();

        let rows = project_summaries(&db).unwrap();
        assert_eq!(rows[0].usage.input, 3_000_000, "tokens must still be counted");
        assert!((rows[0].cost_usd - 30.0).abs() < 1e-9, "unpriced model adds no cost");
    }

    #[test]
    fn a_project_with_no_sessions_still_appears() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        db.upsert_project("-Users-a-empty", "/Users/a/empty", true).unwrap();
        let rows = project_summaries(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sessions, 0);
        assert_eq!(rows[0].usage.total_tokens(), 0);
        assert!(rows[0].path_is_guess);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p perch-core query`
Expected: FAIL — `cannot find function project_summaries`.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/perch-core/src/query.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ProjectSummary {
    pub id: i64,
    pub slug: String,
    pub real_path: String,
    pub display_name: Option<String>,
    pub parent_project_id: Option<i64>,
    pub path_is_guess: bool,
    pub sessions: i64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub last_activity_at: Option<i64>,
}

/// Sum usage per (scope, model) so each model's rate applies to its own tokens.
fn usage_rows(db: &Db, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<(String, TurnUsage)>> {
    let mut stmt = db.conn().prepare(sql)?;
    let rows = stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            TurnUsage {
                input: r.get::<_, i64>(1)? as u64,
                output: r.get::<_, i64>(2)? as u64,
                cache_read: r.get::<_, i64>(3)? as u64,
                cache_write_5m: r.get::<_, i64>(4)? as u64,
                cache_write_1h: r.get::<_, i64>(5)? as u64,
                thinking: r.get::<_, i64>(6)? as u64,
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

const SUMS: &str = "COALESCE(SUM(input),0), COALESCE(SUM(output),0), COALESCE(SUM(cache_read),0), \
                    COALESCE(SUM(cache_write_5m),0), COALESCE(SUM(cache_write_1h),0), \
                    COALESCE(SUM(thinking),0)";

/// Unknown models contribute tokens but no cost, per spec §8.
fn cost_of(db: &Db, model: &str, usage: &TurnUsage) -> Result<f64> {
    Ok(match price_for(db, model)? {
        Some(p) => cost_usd(&p, usage),
        None => 0.0,
    })
}

pub fn project_summaries(db: &Db) -> Result<Vec<ProjectSummary>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, slug, real_path, display_name, parent_project_id, path_is_guess
         FROM projects ORDER BY real_path",
    )?;
    let bases = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (id, slug, real_path, display_name, parent_project_id, path_is_guess) in bases {
        let sessions: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM sessions WHERE project_id = ?1",
            params![id],
            |r| r.get(0),
        )?;

        let per_model = usage_rows(
            db,
            &format!(
                "SELECT model, {SUMS} FROM turns
                 WHERE session_id IN (SELECT id FROM sessions WHERE project_id = ?1)
                 GROUP BY model"
            ),
            &[&id],
        )?;

        let mut usage = TurnUsage::default();
        let mut cost = 0.0;
        for (model, u) in &per_model {
            usage = usage.plus(u);
            cost += cost_of(db, model, u)?;
        }

        let last_activity_at: Option<i64> = db.conn().query_row(
            "SELECT MAX(ts) FROM turns
             WHERE session_id IN (SELECT id FROM sessions WHERE project_id = ?1)",
            params![id],
            |r| r.get(0),
        )?;

        out.push(ProjectSummary {
            id, slug, real_path, display_name, parent_project_id, path_is_guess,
            sessions, usage, cost_usd: cost, last_activity_at,
        });
    }
    Ok(out)
}

/// Serves both clocks: a trailing 5-hour window and a trailing week differ only
/// in `since_ms`. Pass 0 for all time.
pub fn usage_since(db: &Db, since_ms: i64) -> Result<(TurnUsage, f64)> {
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE ts >= ?1 GROUP BY model"),
        &[&since_ms],
    )?;

    let mut usage = TurnUsage::default();
    let mut cost = 0.0;
    for (model, u) in &per_model {
        usage = usage.plus(u);
        cost += cost_of(db, model, u)?;
    }
    Ok((usage, cost))
}

pub fn usage_by_model(db: &Db) -> Result<Vec<(String, TurnUsage, f64)>> {
    let per_model = usage_rows(db, &format!("SELECT model, {SUMS} FROM turns GROUP BY model"), &[])?;
    let mut out = Vec::new();
    for (model, u) in per_model {
        let c = cost_of(db, &model, &u)?;
        out.push((model, u, c));
    }
    Ok(out)
}
```

- [ ] **Step 4: Register the module**

In `crates/perch-core/src/lib.rs`, add:

```rust
pub mod query;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-core query`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/query.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): project, window, and per-model aggregation queries"
```

---

## Task 11: Debug CLI — Milestone 1 deliverable

**Files:**
- Modify: `crates/perch-cli/Cargo.toml`
- Modify: `crates/perch-cli/src/main.rs`

**Interfaces:**
- Consumes: everything in `perch-core`.
- Produces: a `perch-cli` binary with `index`, `projects`, `usage`, and `models` subcommands.

This is where the milestone is proven: run it against the real `~/.claude` and confirm the numbers are right.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/perch-cli/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_large_token_counts_compactly() {
        assert_eq!(human_tokens(0), "0");
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(1_500), "1.5k");
        assert_eq!(human_tokens(24_221), "24.2k");
        assert_eq!(human_tokens(4_100_000), "4.1M");
    }

    #[test]
    fn formats_cost_with_two_decimals() {
        assert_eq!(human_cost(0.0), "$0.00");
        assert_eq!(human_cost(8.204), "$8.20");
        assert_eq!(human_cost(38.0), "$38.00");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p perch-cli`
Expected: FAIL — `cannot find function human_tokens`.

- [ ] **Step 3: Add the dependency the CLI needs**

In `crates/perch-cli/Cargo.toml`, add to `[dependencies]`:

```toml
chrono.workspace = true
```

- [ ] **Step 4: Write the implementation**

Replace the top of `crates/perch-cli/src/main.rs` (keep the test module at the bottom):

```rust
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use perch_core::{config, db, index, pricing, query};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "perch", about = "Debug CLI for the Perch data layer")]
struct Cli {
    /// Override the Claude Code config directory.
    #[arg(long)]
    config_dir: Option<PathBuf>,

    /// Path to the index database. Defaults to a file beside the config dir.
    #[arg(long)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan transcripts and update the index.
    Index,
    /// List projects with token totals.
    Projects,
    /// Show usage over trailing windows.
    Usage,
    /// Break usage down by model.
    Models,
}

fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn human_cost(c: f64) -> String {
    format!("${c:.2}")
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config_dir = cli
        .config_dir
        .or_else(config::config_dir)
        .context("could not locate a Claude Code config directory; set CLAUDE_CONFIG_DIR")?;

    let db_path = cli.db.unwrap_or_else(|| PathBuf::from("perch-index.db"));
    let database = db::open(&db_path)?;
    pricing::seed_default_prices(&database)?;

    match cli.command {
        Command::Index => {
            let root = config::projects_dir(&config_dir);
            let started = std::time::Instant::now();
            let stats = index::index_all(&database, &root)?;
            println!(
                "indexed {} projects, {} sessions, {} new turns, {} new bytes, {} lines skipped in {:.2}s",
                stats.projects,
                stats.sessions,
                stats.new_turns,
                stats.bytes_read,
                stats.lines_skipped,
                started.elapsed().as_secs_f64()
            );
        }
        Command::Projects => {
            let mut rows = query::project_summaries(&database)?;
            rows.sort_by(|a, b| b.usage.total_tokens().cmp(&a.usage.total_tokens()));
            println!("{:<52} {:>4} {:>9} {:>9} {:>10}", "PROJECT", "SESS", "TOKENS", "CACHE-RD", "EST COST");
            for r in rows {
                let name = r.display_name.clone().unwrap_or_else(|| r.real_path.clone());
                let flag = if r.path_is_guess { " (guessed path)" } else { "" };
                let child = if r.parent_project_id.is_some() { "  ↳ " } else { "" };
                println!(
                    "{child}{:<52} {:>4} {:>9} {:>9} {:>10}{flag}",
                    truncate(&name, 50),
                    r.sessions,
                    human_tokens(r.usage.total_tokens()),
                    human_tokens(r.usage.cache_read),
                    human_cost(r.cost_usd),
                );
            }
        }
        Command::Usage => {
            let now = chrono::Utc::now().timestamp_millis();
            let five_hours = now - 5 * 60 * 60 * 1000;
            let week = now - 7 * 24 * 60 * 60 * 1000;

            for (label, since) in [("last 5 hours", five_hours), ("last 7 days", week), ("all time", 0)] {
                let (u, cost) = query::usage_since(&database, since)?;
                println!(
                    "{label:<14} total {:>9}  in {:>8}  out {:>8}  cache-rd {:>9}  cache-wr {:>8}  {:>9}",
                    human_tokens(u.total_tokens()),
                    human_tokens(u.input),
                    human_tokens(u.output),
                    human_tokens(u.cache_read),
                    human_tokens(u.cache_write_total()),
                    human_cost(cost),
                );
            }
        }
        Command::Models => {
            let mut rows = query::usage_by_model(&database)?;
            rows.sort_by(|a, b| b.1.total_tokens().cmp(&a.1.total_tokens()));
            println!("{:<34} {:>9} {:>10}", "MODEL", "TOKENS", "EST COST");
            for (model, u, cost) in rows {
                println!("{:<34} {:>9} {:>10}", truncate(&model, 32), human_tokens(u.total_tokens()), human_cost(cost));
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
    format!("…{tail}")
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p perch-cli`
Expected: PASS, 2 tests.

- [ ] **Step 6: Run against real data — the milestone gate**

```bash
cargo run --release -p perch-cli -- --db /tmp/perch-index.db index
cargo run --release -p perch-cli -- --db /tmp/perch-index.db projects
cargo run --release -p perch-cli -- --db /tmp/perch-index.db usage
cargo run --release -p perch-cli -- --db /tmp/perch-index.db models
```

Verify all of the following before considering the milestone done:

1. `index` reports roughly 23 projects and 231 sessions on the reference machine.
2. `projects` lists real filesystem paths — in particular `amirdaraee.github.io` appears with a dot, not as a guessed path.
3. Worktree rows appear indented under their parent project.
4. `cache-rd` is far larger than `in` for most projects. If cache-read is near zero, the parser is wrong.
5. Running `index` a second time reports **0 new turns** and **0 new bytes**, and completes in well under a second. This is the incremental property working on real data.
6. `models` lists the models actually present in the transcripts.

- [ ] **Step 7: Confirm no writes touched the Claude Code directory**

```bash
find ~/.claude -newermt '-10 minutes' -type f -not -path '*/projects/*' -not -path '*/sessions/*' -not -name '*.log' | head
```

Expected: no files that Perch could have created. Perch must only have written `/tmp/perch-index.db`. Claude Code itself may have modified its own files; anything named `perch*` under `~/.claude` is a bug.

- [ ] **Step 8: Lint and format**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0.

- [ ] **Step 9: Commit**

```bash
git add crates/perch-cli Cargo.lock
git commit -m "feat(cli): debug commands for index, projects, usage, and models"
```

---

## Task 12: CI and README for the data layer

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `README.md`
- Create: `LICENSE`

**Interfaces:**
- Consumes: the workspace test commands.
- Produces: CI that runs fmt, clippy, and tests on macOS and Linux, plus the outbound-host audit required by spec §10.

- [ ] **Step 1: Write the CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, ubuntu-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace

  no-network:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Assert no HTTP client dependencies in the data layer
        run: |
          set -euo pipefail
          if grep -rEn '^(reqwest|hyper|ureq|curl|isahc|surf)\s*=' crates/*/Cargo.toml; then
            echo "::error::An HTTP client was added. The data layer must make no outbound requests."
            exit 1
          fi
          echo "no HTTP client dependencies found"
      - name: Assert no telemetry or analytics dependencies
        run: |
          set -euo pipefail
          if grep -rEni '(sentry|posthog|segment|mixpanel|datadog|analytics)' crates/*/Cargo.toml; then
            echo "::error::Telemetry dependency detected. Perch ships no telemetry."
            exit 1
          fi
          echo "no telemetry dependencies found"
```

- [ ] **Step 2: Write the LICENSE**

Create `LICENSE` with the standard MIT text, `Copyright (c) 2026 Amir Daraee`:

```
MIT License

Copyright (c) 2026 Amir Daraee

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 3: Write the README**

Create `README.md`:

```markdown
# Perch

A dashboard for Claude Code — track your projects, live sessions, and token usage.

> **Status:** early development. The data layer works; the app UI is in progress.

Not affiliated with or endorsed by Anthropic.

## What it does

Perch reads the data Claude Code already writes to disk and answers three questions:

- **Which session is stuck waiting on me, and which terminal owns it?**
- **What was each project doing, and where did I leave off?**
- **How much of my rate-limit window is left, and where did the tokens go?**

## Privacy

Your transcripts contain source code, pasted secrets, and client names. So:

- **No telemetry, no analytics, no crash reporting.** Ever.
- **Read-only.** Perch never writes to, moves, or deletes anything in your Claude Code directory.
- **One outbound host,** and only in the app itself: `anthropic.com`, to read your own rate-limit
  status. The data layer in this repository makes no network requests at all, and CI fails the
  build if an HTTP client is added to it.

## Try the data layer

Requires [Rust](https://rustup.rs).

```bash
cargo run --release -p perch-cli -- index
cargo run --release -p perch-cli -- projects
cargo run --release -p perch-cli -- usage
cargo run --release -p perch-cli -- models
```

Perch finds your Claude Code data at `$CLAUDE_CONFIG_DIR`, then `$XDG_CONFIG_HOME/claude`,
then `~/.claude`. Override it with `--config-dir`.

## How it works

Transcripts are append-only, so Perch stores a byte offset per file and re-parses only new
bytes on each pass — a full re-index of 120 MB happens once, and every pass after that costs
microseconds per active session.

Project paths come from the `cwd` field inside transcripts, never from decoding the directory
name. That encoding flattens both `/` and `.` into `-`, so `-Users-me-projects-foo-github-io`
is ambiguous and decoding it picks the wrong directory.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## License

MIT
```

- [ ] **Step 4: Verify CI passes locally**

Run the same commands CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
grep -rEn '^(reqwest|hyper|ureq|curl|isahc|surf)\s*=' crates/*/Cargo.toml || echo "no http clients: OK"
```

Expected: all pass; the grep prints the OK message.

- [ ] **Step 5: Commit**

```bash
git add .github README.md LICENSE
git commit -m "chore: add CI, README, and MIT license"
```

---

## Self-Review

**Spec coverage.** Spec §3 (data sources) → Tasks 2, 4, 6. §3.1 (no slug decoding) → Task 6, with the ambiguous-slug case as a named regression test. §3.2 (volume) → Task 5's incremental design, verified on real data in Task 11 Step 6. §4 (architecture) → Task 1's crate split; the `live` and `limits` modules are deliberately out of this plan's scope and belong to the next one. §5 (indexing) → Tasks 5 and 9. §6 (data model) → Task 7, including the user-owned-column property. §8 (cost) → Task 8, including the unknown-model rule. §10 (privacy, license, CI) → Task 12. §11 (testing) → the incremental-equivalence, path-resolution, and cost rows are covered here; liveness and usage-client rows belong to the next plan alongside the code they test. §12 Milestone 1 → Task 11's gate.

**Not covered here, by design:** spec §7 (live detection), §8 tier-1 API client, §9 (all UI), §10 demo mode and release packaging. These are Milestones 2–6 and need their own plans.

**Known follow-ups deferred to the next plan:** the `prices` seed values in Task 8 are placeholders for display purposes and should be checked against published pricing before any release; `parent_path_of` assumes the `/.claude/worktrees/` layout observed on the reference machine and should gain a fallback if another layout appears.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — tasks executed in this session with batch checkpoints.
