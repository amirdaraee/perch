# Perch backlog

Ideas, roughly ordered. Inspired in places by [CodexBar](https://github.com/steipete/CodexBar),
which is a usage meter across many providers. Perch is deliberately narrower — Claude Code
only — and deliberately wider in one direction CodexBar does not go: **sessions and projects**.
That is the edge, and the backlog leans into it.

Status legend: **now** = next task · **next** = this milestone or the following ·
**later** = after the main window and the rate-limit API land · **no** = decided against.

---

## Shipped (for orientation)

- Data layer: incremental transcript indexing, per-model/per-class token accounting, SQLite.
- Menu-bar app: tray, no Dock icon, popover under the icon, works over fullscreen apps.
- Live sessions: working / idle / waiting-with-reason, updates within a second, 5 s backstop.
- Popover: three usage stats (window · week · 24h, marked *est*), session rows, Escape.
- CI: no-network audit of every crate and the frontend, with self-testing guards.
- **Native macOS app.** Replaced the Tauri popover's fullscreen-visibility workaround with a
  real, tracked `NSMenu` on the status item — the structurally correct fix, not yet confirmed
  by a human on a real display. Engine is the same Rust view-model, exposed to Swift via
  UniFFI; the app is a SwiftPM executable built by `scripts/build-xcframework.sh` plus
  `make bundle`. The Tauri app remains until this reaches parity.
- **Richer session rows.** Second line: project · kind · Claude Code version, per-session
  tokens and ≈$ joined from the index, blocked sessions sorted first.
- **Recent section.** Last ended sessions with project and "ended 2h ago".
- **Taller popover**, list scrolls, hosted as SwiftUI cards inside the native menu.
- **Re-index on show**, not only on launch, so a long-running Perch never shows stale totals.

## Now

- **Settings window**: refresh interval, notification toggles, menu-bar display mode,
  preferred terminal for resume, `CLAUDE_CONFIG_DIR` override (a Finder-launched app does not
  inherit shell env — this is the fix).

## Next — sessions & projects (Perch's own ground)

- **Waiting-on-you notification.** macOS alert when a session has been `waiting` longer than
  N minutes (default 10), once per episode, background sessions excluded by default. This is
  the feature that solves the 32-hour-blocked-session problem the project started from.
- **Jump to session.** Focus the terminal or IDE window that owns the session (walk the process
  tree from the pid). Fallback: reveal the cwd in Finder.
- **Resume ended session.** `claude --resume <id>` in the user's preferred terminal.
- **Project "where I left off" note** — one line per project, user-owned, survives re-index.
- **Pin / archive / status** per project (active · paused · done).
- **Main window, hybrid layout** (spec §9.2): "Now" pinned above the project list; project
  detail with note, totals, 14-day sparkline, session history.
- **Prompt search.** `history.jsonl` already keys every typed prompt by session — search it to
  find "that session where I asked about X" without opening transcripts.
- **Session naming.** Rename a live session from Perch (Claude Code already supports
  `name` in the record; investigate whether it can be written back safely — otherwise store
  the alias in Perch's own DB).
- **Subagent transcripts.** Index `<session>/subagents/*.jsonl` and attribute to the parent.
  Today's totals are a floor; 164 such files on the reference machine.

## Next — usage (the CodexBar-shaped half)

- **Real rate-limit percentages** from the OAuth usage endpoint, with reset countdowns; local
  estimates remain the labelled fallback (spec §8). This is what finally puts a % in the menu
  bar (spec §9.1).
- **Menu-bar display modes** (from CodexBar): icon only · % · % + countdown · "N ⏳" when
  blocked. User-selectable; today it is count + ⏳.
- **Tiny usage meter in the icon** — a filled-bar glyph that reads at a glance without text.
- **Stale-data dimming** — dim the icon when the last successful refresh is older than X.
- **Approaching-limit alert** at a configurable threshold (default 80 %) once per window.
- **Burn-rate projection** — "at this pace the window fills in ~1h 50m" (approved mockup).
- **Usage view** (spec §9.3): daily stacked bars by token class, top projects, by model.
- **Verified price table** with an editable UI — today's numbers are placeholders.
- **Reset-time style** options: countdown vs absolute time (CodexBar has this; cheap).

## Next — settings & polish

- **Launch at login.**
- **Light-mode palette** — the popover is dark-only today.
- **Keyboard**: ⌥-click the tray for the menu; ↑↓ to move between sessions, ⏎ to jump.
- **First-run state**: "No Claude Code data found at …" with a path picker, instead of an
  empty popover.
- **Demo mode** (`--demo`) with synthetic sessions/projects — needed for README screenshots
  without leaking real project names, and for deterministic UI tests.
- **Remove the Tauri app and React frontend** once the native app has jump-to-session and
  resume, its last two gaps versus the popover it replaces.

## Later

- **CLI parity** (CodexBar's `codexbar cost` / `serve` pattern): `perch sessions --json`,
  `perch usage --json`, and a tiny local endpoint so SketchyBar / tmux / Stream Deck can show
  the blocked-session count. Perch already has a CLI; this is output formats plus a server.
- **Widgets**: a WidgetKit widget showing live sessions and window %.
- **Daily/weekly digest**: "yesterday: 3 sessions, 1.2M tokens, 40 min blocked".
- **Cross-project timeline**: which session was active when, laid out across the day.
- **Cost attribution by git branch** — the index already stores `git_branch` per session.
- **Multi-machine**: read a synced copy of another Mac's `~/.claude` (read-only, no server).
- **Windows / Linux**: the data layer is portable; the tray/panel/probe traits are the work.
- **Signed & notarised builds, Homebrew cask** — once there are users to justify the $99/yr.
- **Incident badge** when Anthropic's status page reports an outage (one more outbound host —
  make it opt-in and say so in the privacy section).
- **Localisation** — CodexBar ships 21 languages; not before the UI settles.

## No (decided against)

- **Other providers.** CodexBar covers 69; Perch covers one, deeply. Sessions and projects are
  where the value is, and every provider adds an auth surface to a privacy-led app.
- **Browser-cookie or keychain scraping beyond the one Claude Code credential** the usage
  endpoint needs.
- **Sending messages into running sessions** over the session socket — undocumented protocol,
  low value relative to risk (spec §2).
- **Telemetry of any kind.** CI fails the build if a network client or telemetry SDK appears.
- **Weekly-reset confetti.** Fun; not for us.

---

## How to use this file

Pick from **Now** first. Each item becomes a bounded task with its own brief and review. When
something moves, edit the section header it sits under rather than adding status tags.
