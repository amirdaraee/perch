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
- **No network requests, today.** Neither the data layer, the Tauri app, nor the native macOS
  app makes any HTTP call, and CI fails the build if an HTTP client or telemetry dependency
  appears in the manifests or the resolved dependency graph. A planned future release adds
  exactly one outbound host, `anthropic.com`, to read your own rate-limit status from
  Anthropic's OAuth usage endpoint (see the backlog) — this section will be updated when
  that lands.

## Native app (macOS 15+)

A native rewrite of the menu-bar app: `NSStatusItem` → a real, tracked `NSMenu` → SwiftUI
cards hosted in `NSMenuItem`s. It exists because only a real, tracked `NSMenu` keeps the
menu bar visible while it's open over a fullscreen app — an `NSPopover` or a detached
`NSPanel` does not, which two spikes confirmed before this rewrite started. That said: the
design is structurally correct for this, but no one has yet confirmed it on a real display
against a real fullscreen app — treat it as guaranteed by construction, not as verified.

The engine is the same Rust view-model as everywhere else in this repo (`perch-core`'s `ui`
module), exposed to Swift as a static library via [UniFFI](https://github.com/mozilla/uniffi-rs)
0.32. Every string the menu shows — formatting, the em dash for absent data, the pluralised
"N sessions are waiting on you" banner — is produced in Rust, so a later Linux or Windows
shell can reuse it instead of reimplementing it.

Requires macOS 15+ and Swift 6. Build and run from a clean checkout:

```bash
scripts/build-xcframework.sh        # builds perch-ffi, emits PerchCore.xcframework + Swift bindings
cd apps/macos/Perch && make bundle  # swift build -c release, then bundles build/Perch.app
open build/Perch.app
```

`apps/macos/Perch` is a SwiftPM executable — there's no Xcode project. The `.xcframework`
and the generated Swift bindings are build output and git-ignored; always regenerate them
with the script above rather than trusting a checked-in copy, which is a classic source of
FFI bugs.

The Tauri app below stays in the repo, unchanged, until the native app reaches parity — it
still lacks jump-to-session and resume.

## The app

Perch is a **menu-bar app** (macOS) built with [Tauri 2](https://tauri.app): a tray icon
showing live-session count, and a popover with usage stats and the session list.

Prerequisites: [Rust](https://rustup.rs), [pnpm](https://pnpm.io), and the Tauri CLI
(`cargo install tauri-cli --version '^2'`, giving `cargo tauri`).

```bash
pnpm install
cargo tauri build                  # .app + .dmg under target/release/bundle
# or, for just the binary:
pnpm build && cargo build --release -p perch-app --features custom-protocol
```

The `custom-protocol` feature is what embeds the built frontend in the binary. `cargo tauri
build` turns it on for you; a plain `cargo build` does not, and produces a binary that expects
a dev server instead.

The popover is converted to an `NSPanel` using [`tauri-nspanel`](https://github.com/ahkohd/tauri-nspanel),
because a plain window cannot appear over fullscreen apps. It is AppKit-only, it is the only
dependency in this project that does not come from crates.io — a git dependency on a personal
repository, pinned by commit in `src-tauri/Cargo.toml` — and it is compiled only on macOS.

**Known issue: the app and `perch-cli` can disagree about where your data is.** An app launched
from Finder does not inherit environment variables set in a shell rc, so `CLAUDE_CONFIG_DIR`
and `XDG_CONFIG_HOME` are invisible to it: the app falls back to `~/.claude` while `perch-cli`,
run from your shell, honours them. This resolves when Perch grows a settings file to record the
directory explicitly.

## Try the data layer

Requires [Rust](https://rustup.rs).

> **Reported totals are a floor, not a total.** Perch currently indexes only the top-level
> `*.jsonl` transcripts in each project directory. Subagent transcripts, which live under
> `<session-id>/subagents/`, are not yet indexed even though that work is billed separately —
> so your real usage is higher than anything the commands below report, sometimes
> substantially.

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
