# Perch backlog

Ideas, roughly ordered. Inspired in places by [CodexBar](https://github.com/steipete/CodexBar),
which is a usage meter across many providers. Perch is deliberately narrower — Claude Code
only — and deliberately wider in one direction CodexBar does not go: **sessions and projects**.
That is the edge, and the backlog leans into it.

Status legend: **now** = next task · **next** = this milestone or the following ·
**later** = further out · **no** = decided against.

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
  `make bundle`. The Tauri app remains until it's removed (see **Now**).
- **Richer session rows.** Second line: project · kind · Claude Code version, per-session
  tokens and ≈$ joined from the index, blocked sessions sorted first.
- **Recent section.** Last three ended sessions with project and "ended 2h ago".
- **Taller popover**, list scrolls, hosted as SwiftUI cards inside the native menu.
- **Re-indexes on show**, not only on launch, so a long-running Perch never shows stale totals.
- **Main window.** A real window, opened from the status-item menu's **Open Perch**, that
  gives the app a Dock icon while it's up and returns to menu-bar-only on close. A sidebar
  lists every project — Pinned, Active, Recent, Archived — each row carrying its token and
  spend totals.
- **Per-project stats.** Selecting a project shows its own note (editable, survives
  re-index), aggregate totals (sessions · tokens · cost), a 14-day sparkline, and its full
  session history. Pin, archive, and rename all live here too.
- **Resume and open a project's terminal.** Resume runs `claude --resume <id>` for any past
  session; "Open new session" starts a fresh `claude` in the project's directory. Both are
  composed in Rust (`perch-core::actions`, every value POSIX-single-quoted) and handed to the
  preferred terminal from Settings (Terminal.app or iTerm2) via a one-shot script written into
  Perch's own application-support directory — no Automation permission prompt, nothing touches
  the Claude Code directory.
- **Usage view** (spec §9.3). Hero stats, 14 days of token-class stacked bars, a top-projects
  ranking, and a per-model breakdown, as a second tab alongside the project list.
- **Settings window.** A sidebar window of nine panes (General, Menu Bar, Popover, Projects,
  Usage, Prices, Notifications, Diagnostics, Advanced) reached from the status-item menu's
  **Settings…**, covering twenty-seven settings — every one wired to real behaviour, not just
  stored. The schema is built in Rust (`settings::schema`): panes, groups, rows, controls,
  bounds, help sentences, search terms, the attention badges, and the per-pane preview of
  what the current choices produce in the user's own data. Swift renders one SwiftUI control
  per control variant and composes no caption of its own. A search field narrows every pane
  to matching rows; a pane that has drifted offers to reset only itself; Advanced resets
  everything and carries the resolved paths, index counts, and a reindex button; Prices is an
  editable model-price table. Settings live in a watched, versioned `config.toml` under
  Perch's own application-support directory (`PERCH_CONFIG` overrides the path); a hand edit
  takes effect without a restart, an out-of-range value is clamped rather than rejected, and a
  file that won't parse falls back to defaults with the reason shown in the window rather than
  refusing to start.
- **Menu-bar icon variants and stale-data dimming.** Four glyphs to choose from, and the item
  fades — carrying a finished "last read N minutes ago" sentence as its accessibility label —
  once the index has gone unread for longer than the configured threshold.
- **Popover sections, density, and row detail.** The live list splits into Waiting on you and
  Working, each hideable; Recent is hideable and its length configurable; rows draw at
  comfortable or compact density and can carry the project folder and the per-session usage.
  Every one of these reaches the menu on the view-model, never read out of `Settings` by the
  shell.
- **Burn-rate line** (spec §9.3). Tokens per hour, cost per hour, cost per day, or "at this
  pace the window fills in ~1h 50m" — the mode is a setting, and a window too thin to project
  from honestly shows no line in any mode.
- **Waiting-on-you notification.** The feature the project started from: a session blocked for
  32 hours with nobody noticing. Turn it on in Settings and Perch tells you, once per episode,
  when a session has been waiting on you longer than N minutes (default 10; background
  sessions excluded by default). The decision is made in Rust — pure, edge-triggered, fires
  exactly once per blocked stretch — Swift only hands the finished strings to
  `UNUserNotificationCenter`. Authorization is requested when the setting is turned on, never
  at launch, and a denial is reflected honestly in the toggle. Any project can override the
  global threshold — Default, a custom number of minutes, or Off — from its own detail pane in
  the main window, next to pin, archive, and the note.

---

## Now

- **Remove the Tauri app and React frontend.** The native app now has the main window, resume,
  open, and per-project stats — the parity this was waiting on. `src-tauri` and `src/` (the
  React frontend) can come out once the native app is the one people actually run.

## Next — sessions & projects (Perch's own ground)

- **Preferred-terminal auto-detection.** Settings now lists the terminals it finds installed
  (`terminals::terminal_choices`), so the picker no longer offers apps that aren't there — but
  it still asks the user to pick, and only Terminal.app and iTerm2 are actually launched (see
  the `preferred_terminal` entry under **settings & polish**). The design spec's original idea
  — resolving the terminal by walking the process tree from a live session's pid, rather than
  asking at all — is still undone.
- **Hooks on session events.** Run a user-configured command when a session starts, ends, or
  starts waiting on you — the natural next step past a notification. Considered for the
  settings redesign and deliberately left out of it: it needs its own trust story before it
  ships, because this would be Perch running an arbitrary user-provided executable, which is a
  different risk profile from anything else in this read-only, no-network app. A settings pane
  is the easy half; deciding what the app is willing to execute is the work.
- **Jump to session.** The original spec §9.4 and this branch's own spec §6 both call for a
  third action alongside Resume and Open: Jump activates the *application* that owns a live
  session (`NSRunningApplication.activate()`, no special permission needed) rather than
  focusing the specific window (which would need Accessibility). The fallback — reveal the
  project's `cwd` in Finder — is always offered alongside it, live or not. Implemented nowhere
  yet; this item was previously dropped from the backlog without being built.
- **Project status beyond pin/archive** (active · paused · done). `db::set_status` already
  exists in the data layer; no UI surfaces the third state yet — only pinned and archived do.
- **Prompt search.** `history.jsonl` already keys every typed prompt by session — search it to
  find "that session where I asked about X" without opening transcripts.
- **Session naming.** Rename a live session from Perch (Claude Code already supports
  `name` in the record; investigate whether it can be written back safely — otherwise store
  the alias in Perch's own DB).
- **Subagent transcripts.** Index `<session>/subagents/*.jsonl` and attribute to the parent.
  Today's totals are a floor; 164 such files on the reference machine.
- **Worktree folding and an "N others" tail for top projects** (spec §9.3.3). `projects.
  parent_project_id` exists and is already populated, but the top-projects ranking doesn't use
  it yet — three worktrees of one repo currently compete as three separate rows instead of
  folding into their parent, and the ranking has no collapsed tail for everything past the
  top N.
- **Robust live-dot matching.** `main_window.rs` lights a project's live dot by comparing
  `live.cwd == project.real_path` exactly, so `/tmp` vs `/private/tmp` (a real macOS symlink)
  can silently fail to match even though a session is genuinely live there. Session rows in
  the same window match by `session_id` instead and are robust to this. The two liveness
  signals can disagree within one window; unify on the robust comparison.

## Next — usage (the CodexBar-shaped half)

- **Real rate-limit percentages** from the OAuth usage endpoint, with reset countdowns; local
  estimates remain the labelled fallback (spec §8). This is what finally puts a % in the menu
  bar (spec §9.1).
- **Hover detail on the daily chart** (spec §9.3.2). Consciously scoped out of the first cut
  of the daily stacked bars — the four token classes and their totals render, but nothing
  shows on hover yet.
- **Move the token-class labels into Rust.** `PerchChartPalette.order` in `PerchStyle.swift`
  still hand-writes "Input" / "Output" / "Cache read" / "Cache write" as Swift string
  literals (now mapped by a typed key, but still Swift-authored English); the purist fix is
  putting these in `ui::usage` alongside the rest of the finished strings, so a future
  Linux/Windows shell doesn't reinvent them.
- **Percentage-based menu-bar display modes** (from CodexBar): % · % + countdown. Icon-only,
  count, and count-plus-waiting-glyph already ship as a user-selectable Settings option; the
  percentage modes wait on the same OAuth usage endpoint as the item above.
- **Tiny usage meter in the icon** — a filled-bar glyph that reads at a glance without text.
- **Approaching-limit notification** at a configurable threshold (default 80 %) once per
  window — the same edge-triggered shape as the waiting-on-you notification. Blocked on the
  OAuth usage endpoint: spec §8 forbids fabricating a percentage from the estimated tier, and
  a threshold alert needs a real one.
- **Verified prices.** The editable price table ships; the numbers in it do not come from
  anywhere authoritative. Someone has to check them against Anthropic's published rates and
  say, in the repo, where they came from and when.
- **Reset-time style** options: countdown vs absolute time (CodexBar has this; cheap).

## Next — settings & polish

- **A settings CLI.** `SettingKey` and the typed get/set beneath it already make
  `perch settings list`, `perch settings get <key>` and `perch settings set <key> <value>`
  a thin wrapper over what the window uses — same validation, same clamping, same file. The
  redesign deliberately scoped it out so the window landed first; it is the cheapest item on
  this list now that the keys are typed. Fits alongside the CLI parity entry under **Later**.
- **The four `ui::watcher` tests are wall-clock flaky under compile load.** All four passed
  5/5 on an idle machine and all four failed together on the first run after a checkout,
  when `cargo` was still building other crates' test binaries. They assert against real
  timeouts, so a cold CI cache is exactly the condition that trips them. Worth rewriting
  against an injectable clock rather than raising the timeouts, which only moves the
  threshold. A vacuous test has already hidden in this module once, so any rewrite needs
  proof it still fails when the behaviour it names regresses.
- **`preferred_terminal` still crosses the FFI as a bare string**, and `Launcher.swift` still
  compares against its values (`case "iTerm2":`), unlike `menu_bar_display`'s enum. The
  redesign fixed the Rust half — `terminals::terminal_choices` detects what is installed and
  already knows each one's bundle identifier — but Swift never asked for it, so choosing
  Warp, Ghostty, Alacritty, Kitty or WezTerm in Settings silently launches Terminal.app. The
  fix is to carry the bundle id across on the choice and have `Launcher` use it, which
  retires the string comparison at the same time.
- **Light-mode palette** — the popover is dark-only today.
- **Keyboard**: ⌥-click the tray for the menu; ↑↓ to move between sessions, ⏎ to jump.
- **First-run state**: "No Claude Code data found at …" with a path picker, instead of an
  empty popover.
- **Demo mode** (`--demo`) with synthetic sessions/projects — needed for README screenshots
  without leaking real project names, and for deterministic UI tests.

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
