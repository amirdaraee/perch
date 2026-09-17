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
- **No network requests, today.** Neither the data layer nor the native macOS app makes any
  HTTP call, and CI fails the build if an HTTP client or telemetry dependency appears in the
  manifests or the resolved dependency graph. A planned future release adds exactly one
  outbound host, `anthropic.com`, to read your own rate-limit status from Anthropic's OAuth
  usage endpoint — this section will be updated when that lands.

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

**Open Perch**, an item in the status-item menu, opens the app's main window: a sidebar
listing every project — grouped Pinned, Active, Recent, and Archived, each with its token
and spend totals — and, for the selected project, a detail pane with your own note,
aggregate stats, a 14-day sparkline, and its full session history. A separate Usage tab adds
hero stats, 14 days of token-class bars, a top-projects ranking, and a per-model breakdown.
While the window is open the app gets a Dock icon; it goes back to menu-bar-only when the
window closes.

From the project detail pane you can pin, archive, rename, resume an ended session
(`claude --resume <id>`), or start a fresh one (`claude`) — both in the project's own
directory. Both actions hand the command to your preferred terminal — Settings lists the ones
it finds installed (Terminal, iTerm2, Warp, Ghostty, Alacritty, Kitty, WezTerm) — by writing a one-shot
script into Perch's *own* `~/Library/Application Support/Perch/commands/` directory (swept of
anything older than a minute) and asking `NSWorkspace` to open it there, which is what avoids
the Automation permission prompt an AppleScript-driven approach would need. The command line
itself is composed in Rust (`perch-core::actions`), which POSIX-single-quotes every value it
embeds before it ever reaches a shell. None of this touches your Claude Code directory or
makes a network request — both promises in the [Privacy](#privacy) section above hold for
the main window exactly as they do for the menu.

### Settings, notifications, and diagnostics

**Settings…** (⌘,) in the status-item menu opens a sidebar window of nine panes — General,
Menu Bar, Popover, Projects, Usage, Prices, Notifications, Diagnostics, and Advanced —
covering twenty-seven settings. Among them: launch at login; the Claude Code directory
(auto-detect, or an explicit override for when auto-detection picks the wrong one); the menu
bar's display mode, its glyph, and whether it dims once the index has gone unread for longer
than N minutes; which popover sections are drawn, how many recent sessions they list, the row
density, and whether a row carries its project folder and its usage; how recently a project
counts as active and whether archived ones are listed; the chart range, the top-projects
ranking, the burn-rate unit, and whether costs are shown at all; the three notification
settings below; the poll interval; the preferred terminal; and whether Resume launches
Claude Code with `--dangerously-skip-permissions`.

A search field at the top of the sidebar narrows every pane to the rows that match, so a
setting can be found by what it does rather than by guessing which pane it lives in. A pane
that has drifted from the shipped defaults offers to reset just itself; Advanced resets
everything, and also carries the resolved paths, the index's counts, and a reindex button.
Each pane shows what its current settings actually produce in your own data — the menu-bar
title as it will read, the burn-rate line the chosen unit gives you, how many projects clear
the active threshold — so a choice can be judged against the real thing rather than a
description of it. Prices is an editable model-price table; the shipped numbers are
placeholders, and a model priced by hand is remembered until you reset it. Every setting takes
effect immediately; nothing needs a relaunch.

Settings live in Perch's *own* `config.toml` — `~/Library/Application Support/Perch/config.toml`,
never inside your Claude Code directory — as seven TOML tables plus a version key. Set
`PERCH_CONFIG` to point it somewhere else entirely. The file is watched, so a hand edit while
Perch is running takes effect without a restart; an out-of-range value is clamped rather than
rejected, and a file that fails to parse falls back to defaults rather than refusing to start —
either way, the settings window says so.

The schema itself — every pane, group, row, control, bound, help sentence and search term —
is built in Rust (`perch-core::settings::schema`) and the window renders it. Swift picks one
SwiftUI control per control variant and one SF Symbol per semantic icon, and composes no
caption of its own: "every 1 minute" against "every 2 minutes" is Rust's sentence, not a
number the window pluralises. CI fails the build if a Swift `Text` ever formats a number.

**The waiting-on-you notification** is the problem Perch was written to solve: a session
blocked for 32 hours with nobody noticing. Turn it on in the Notifications pane and Perch tells you,
once per episode, when a session has been waiting on you longer than N minutes (default 10;
background sessions excluded by default). macOS asks for notification permission at that
moment, not at launch, and a later denial is reflected honestly in the toggle rather than the
setting silently doing nothing. The decision of whether and when to fire is made in Rust —
pure, and edge-triggered so it fires exactly once per blocked stretch — and Swift's only job is
handing the finished title and body to `UNUserNotificationCenter`. Clicking a notification
opens the main window with that session's project selected.

Any project can override the global threshold from its own detail pane in the main window,
next to pin, archive, and the note: Default (follow the setting above), a custom number of
minutes, or Off. Global defaults live in Settings; per-project exceptions live on the project.

**Diagnostics**, a pane in the Settings window, answers one question a confused person actually
asks: why isn't my session showing up? For every session record Perch found, it names whether
the record was accepted or the specific reason it wasn't — no such process, that pid isn't
`claude`, or the record couldn't be parsed — plus the resolved Claude Code directory and how it
was resolved, the config file's path and whether it loaded, and the index's path, session and
turn counts, and last successful run.

None of this touches the promises in [Privacy](#privacy) above: the config file is Perch's own,
never anything inside your Claude Code directory, and every notification is delivered locally
by `UNUserNotificationCenter` — nothing described in this section makes a network request.

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
