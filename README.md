# Perch

A macOS menu-bar dashboard for Claude Code: which sessions are working, which
are waiting on you, and what they cost.

> **Status:** early. It works and I use it daily, but it is pre-1.0 and the
> numbers below come with caveats I've tried to state plainly rather than bury.

Not affiliated with or endorsed by Anthropic.

## Why

The problem Perch was written to solve: a Claude Code session sat blocked on a
permission prompt for 32 hours and nobody noticed. If you run more than one
session, they scroll off, get buried behind a fullscreen window, and quietly
wait.

Perch reads the data Claude Code already writes to disk and answers three
questions:

- **Which session is stuck waiting on me, and which terminal owns it?**
- **What was each project doing, and where did I leave off?**
- **Where did the tokens go?**

## Privacy

Your transcripts contain source code, pasted secrets, and client names. So:

- **No telemetry, no analytics, no crash reporting.** Ever.
- **Read-only.** Perch never writes to, moves, or deletes anything in your
  Claude Code directory. It writes only to its own folder in
  `~/Library/Application Support/Perch/`.
- **No network requests.** There is no HTTP client in the dependency graph and
  no socket in the source. CI fails the build if either changes, if a telemetry
  crate becomes reachable in the resolved dependency graph, or if any Rust or
  Swift source names an absolute `http(s)://` URL.
- **No transcript contents leave the parser.** Perch reads transcripts to count
  tokens and detect state. No message text reaches a window, a notification, a
  log line, or the MCP server.

A planned future release would add exactly one outbound host, `anthropic.com`,
to read your own rate-limit status from Anthropic's OAuth usage endpoint. It is
not in the code today, and this section will change before it is.

## Install

Requires **macOS 15 or later**.

Download the latest `.zip` from
[Releases](https://github.com/amirdaraee/perch/releases/latest), unzip it, and
drag `Perch.app` to `/Applications`.

**The first launch will be blocked**, and you need to know this in advance or it
looks like the app is broken. Perch is signed, but with an ad-hoc signature
rather than a paid Apple Developer ID, so macOS treats it as from an
unidentified developer. To open it:

> **System Settings → Privacy & Security**, scroll to the bottom, and click
> **"Open Anyway"** next to the message about Perch.

On macOS 15 this is the only way — the old right-click → Open trick no longer
works for unsigned apps. Notarization, which removes this step, needs a paid
Apple Developer account and is not something this project has yet.

If you'd rather not run an unidentified binary at all, that's a reasonable
instinct for an app that reads your transcripts: [build it from
source](#building-from-source) instead. It's two commands.

## What you get

Perch lives in the menu bar with no Dock icon, showing your live-session count.
Clicking it opens a menu of usage stats and session rows — a real, tracked
`NSMenu`, which is what keeps it visible over a fullscreen app.

**Open Perch** opens the main window: a card for every project with its
description, session count, tokens, spend, a 14-day sparkline, and whether
anything is running or waiting right now. Selecting one shows its full session
history, your own notes, and aggregate stats. A separate Usage view adds hero
stats, 14 days of token-class bars, a top-projects ranking, and a per-model
breakdown.

From a project you can pin, archive, rename, resume an ended session
(`claude --resume <id>`), or start a fresh one — both in the project's own
directory, in your preferred terminal. Perch finds Terminal, iTerm2, Warp,
Ghostty, Alacritty, Kitty and WezTerm, and launches whichever you choose by
writing a one-shot script into its *own* folder and asking `NSWorkspace` to open
it. That is what avoids the Automation permission prompt an AppleScript
approach would need.

**The waiting-on-you notification** is the original point of the whole thing.
Turn it on and Perch tells you, once per episode, when a session has been
waiting longer than N minutes. Any project can override the threshold from its
own detail pane. macOS asks for notification permission at that moment rather
than at launch, and if you later deny it the toggle says so instead of silently
doing nothing.

**Settings** (⌘,) has nine panes and twenty-seven settings, with a search field
that narrows every pane to matching rows — so a setting can be found by what it
does rather than by guessing where it lives. Each pane shows what its settings
produce *in your own data*: the menu-bar title as it will actually read, how
many projects clear the active threshold. Settings live in Perch's own
`config.toml`, are watched for hand edits, and take effect without a relaunch.

**Diagnostics**, a Settings pane, answers "why isn't my session showing up?"
For every session record found, it names whether the record was accepted or the
specific reason it wasn't — no such process, that pid isn't `claude`, or the
record couldn't be parsed.

### Caveats worth knowing

- **Reported totals are a floor, not a total.** Perch indexes the top-level
  `*.jsonl` transcripts in each project directory. Subagent transcripts, under
  `<session-id>/subagents/`, are not yet indexed even though that work is billed
  separately — so your real usage is higher than what Perch reports, sometimes
  substantially.
- **Model prices are placeholders.** The shipped numbers are unverified and the
  Prices pane lets you correct them. A model with no price contributes its
  tokens but no cost, and is named as unpriced — never silently counted as $0.
- **The fullscreen behaviour is correct by construction, not by confirmation.**
  A tracked `NSMenu` is the documented way to stay visible over a fullscreen
  app, and two spikes confirmed the alternatives don't. Nobody has yet verified
  it on a real second display.

## MCP server

Perch ships `perch-mcp`, a read-only [MCP](https://modelcontextprotocol.io)
server, inside the app bundle. It lets Claude Code itself ask about your
projects, live sessions and usage.

Register it:

```bash
claude mcp add --scope user perch -- /Applications/Perch.app/Contents/MacOS/perch-mcp
```

Settings → Advanced has this line with your actual path, ready to copy.

Four tools: `list_projects`, `get_project`, `live_sessions`, `usage_summary`.
It opens Perch's index read-only, at the SQLite level — writes are refused by
the connection itself, not merely avoided — and has no network code at all.
Every response carries `index_updated_at` so an answer is never mistaken for
being fresher than the index it came from.

## Building from source

Requires macOS 15+, [Rust](https://rustup.rs) and Swift 6.

```bash
scripts/build-xcframework.sh        # builds perch-ffi, emits the xcframework + Swift bindings
cd apps/macos/Perch && make bundle  # builds and bundles build/Perch.app
open build/Perch.app
```

`apps/macos/Perch` is a SwiftPM executable; there is no Xcode project. The
xcframework and the generated Swift bindings are build output and git-ignored —
always regenerate them rather than trusting a checked-in copy, which is a
classic source of FFI bugs.

## Just the data layer

If you only want the numbers, the CLI needs nothing but Rust:

```bash
cargo run --release -p perch-cli -- index
cargo run --release -p perch-cli -- projects
cargo run --release -p perch-cli -- usage
cargo run --release -p perch-cli -- models
```

Perch finds your Claude Code data at `$CLAUDE_CONFIG_DIR`, then
`$XDG_CONFIG_HOME/claude`, then `~/.claude`. Override with `--config-dir`.

## How it works

Transcripts are append-only, so Perch stores a byte offset per file and
re-parses only new bytes on each pass. A full re-index of 120 MB happens once;
every pass after that costs microseconds per active session.

Project paths come from the `cwd` field inside transcripts, never from decoding
the directory name. That encoding flattens both `/` and `.` into `-`, so
`-Users-me-projects-foo-github-io` is ambiguous, and decoding it picks the wrong
directory.

The whole view-model — every string the UI shows, including the em dash that
means "not known" and the pluralised "N sessions are waiting on you" — is built
in Rust (`perch-core`) and exposed to Swift through
[UniFFI](https://github.com/mozilla/uniffi-rs). Swift draws; it never formats a
number. CI fails the build if a Swift `Text` ever does. The point is that a
later Linux or Windows shell can reuse the whole thing instead of
reimplementing every caption, and every reimplemented caption is one that can
disagree.

```
crates/perch-core   parsing, index, settings, and every user-visible string
crates/perch-cli    the data layer on its own
crates/perch-ffi    the UniFFI surface the macOS app is built on
crates/perch-mcp    the read-only MCP server
apps/macos/Perch    the SwiftUI/AppKit shell
```

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) first — it lists the invariants
(read-only, no network, no new dependencies, Rust owns every string) that CI
enforces and that aren't guessable from the source.

## License

[MIT](LICENSE).
