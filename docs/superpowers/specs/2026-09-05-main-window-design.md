# Perch main window — design

**Date:** 2026-09-05
**Status:** Design decided; scope and the two terminal actions confirmed by the user, remaining
decisions taken by the assistant under an explicit "do the rest" delegation and listed in §10.
**Implements:** the 2026-08-30 spec's §9.2 (main window), §9.3 (usage view), and §9.4 (actions),
which were deferred past the data layer and the menu-bar milestones.
**Builds on:** the 2026-09-04 native-app design. Its governing rule carries over unchanged.

---

## 1. Why

The popover answers *"what needs me right now?"* in a glance. It cannot answer *"what have I
been working on?"* — that needs room. Today Perch indexes every project and every past session
but shows only the live ones, so most of what it knows is invisible.

The main window is where the index becomes visible: every project, active or archived; each
project's stats and full session history; and the two actions that turn looking into doing —
resuming a past session, or starting a fresh one in that project.

The governing rule from the native-app design is unchanged and applies to every new surface:

> **Rust owns everything except drawing.** Every string the UI shows is produced in
> `perch-core`. Swift lays out and draws. Chart *values* cross as numbers; their *labels* cross
> as finished strings.

## 2. Scope

**In**

- A second window in the same app: sidebar (**Now** + project list) and two tabs, Overview and
  Usage, per the 2026-08-30 spec §9.2.
- **Project list:** every project — pinned, active, recent, archived — with per-project token
  and spend totals. Pin, archive, and rename are editable from the window.
- **Project detail:** the user's note, aggregate tokens and estimated spend, a 14-day
  sparkline, and the full session history (not just live sessions), each row with its status,
  duration, tokens, and actions.
- **Usage view** (§9.3): hero stats with a burn-rate projection, daily stacked bars by token
  class, top projects over 30 days, and a by-model breakdown.
- **Actions** (§9.4), both confirmed by the user:
  - **Resume** — `claude --resume <sessionId>` in the project's directory.
  - **Open** — a fresh `claude` in the project's directory.
  - Plus **Reveal in Finder**, and **Jump**, which activates the application that owns a live
    session.
- The menu-bar popover gains an **Open Perch** item.

**Out**

- Real rate-limit percentages (still needs the tier-1 endpoint; §8 of the original spec forbids
  fabricating one). The Usage view's hero stats show what can be shown honestly.
- Prompt search over `history.jsonl`, the cross-project timeline, and notifications — separate
  backlog items.
- Removing the Tauri app. It stays until the native app has parity, which this milestone
  completes; its removal is the next task after this one.
- Focusing a *specific window* of another application, which needs Accessibility permission.
  See §6.

## 3. Architecture

```
crates/perch-core/src/
├── ui/
│   ├── model.rs          (existing) PopoverModel
│   ├── main_window.rs    NEW  ProjectRow, ProjectDetail, SessionHistoryRow, MainWindowModel
│   └── usage.rs          NEW   UsageModel: hero stats, daily series, top projects, by model
├── actions.rs            NEW   TerminalCommand — composes what to run; never spawns
├── query.rs              + project_rows, project_detail, session_history, daily_usage,
│                           top_projects, usage_by_model_ranked
└── db.rs                 + set_archived, set_status

crates/perch-ffi/src/lib.rs   + mirrors, + Perch::{main_window, project_detail, usage,
                                set_note, set_pinned, set_archived, rename_project,
                                resume_command, open_command}

apps/macos/Perch/Sources/Perch/
├── MainWindow/
│   ├── MainWindowController.swift   NSWindow host, activation-policy switching
│   ├── Sidebar.swift                Now + project list, grouped
│   ├── ProjectDetail.swift          note, stats, sparkline, session history
│   ├── UsageView.swift              the four §9.3 blocks, Swift Charts
│   └── Launcher.swift               writes a one-shot script, `open -a <terminal>`
└── StatusItemController.swift       + "Open Perch" menu item
```

**Charts use Swift Charts**, which ships in the macOS SDK. No new dependency, which keeps the
no-network CI audit meaningful — an SPM dependency would be the first thing Perch fetches.

**The window is a second window in the existing app,** not a second binary: one engine, one
install, one index. While it is open the app switches its activation policy from `.accessory`
to `.regular`, so it gets a Dock icon and can be reached with Cmd-Tab; closing it switches back.
The status item remains throughout.

## 4. Data flow

The popover's live-push model does not fit a window the user reads and edits. The window
**pulls**: it asks for a model when a view appears and after any edit it makes.

```
window opens        → Perch.main_window()      → MainWindowModel  (sidebar + Now)
project selected    → Perch.project_detail(id) → ProjectDetail    (note, stats, history)
Usage tab selected  → Perch.usage()            → UsageModel
edit (note/pin/…)   → Perch.set_*()            → returns the refreshed model
watcher tick        → existing PopoverModel push also refreshes the window's live rows
```

Reads run on a background queue and hop to the main actor to render, so a large index never
blocks the UI — the mistake caught in the last milestone's final review, avoided here by
construction.

## 5. The view-model

All strings final; all chart values numeric with their labels pre-formatted.

```
ProjectRow      { id, name, path, group: Pinned|Active|Recent|Archived,
                  session_count, tokens: String, cost: String,
                  last_active: String, live_session_count }
MainWindowModel { now: PopoverModel, projects: [ProjectRow], error: String? }

SessionHistoryRow { id, name, started: String, duration: String, tokens: String,
                    cost: String, branch: String?, is_live: bool, can_resume: bool }
ProjectDetail   { id, name, path, note: String, tokens: String, cost: String,
                  session_count: String, sparkline: [SparkPoint], sessions: [SessionHistoryRow],
                  pinned: bool, archived: bool }
SparkPoint      { day_index: i32, tokens: u64, label: String }

UsageModel { hero: [HeroStat], daily: [DailyBar], top_projects: [RankedProject],
             by_model: [ModelUsage], burn_rate: String? }
DailyBar   { day_index, label: String, input, output, cache_read, cache_write, thinking: u64 }
```

`burn_rate` is `None` unless there is enough data to project honestly; the UI omits the line
rather than showing a guess.

## 6. Actions

**Rust composes; the shell spawns.** `actions::TerminalCommand { program, args, cwd }` with a
`shell_line()` that produces a correctly quoted one-liner. This is pure, testable, and reusable:
a Linux shell gets the same command and runs it its own way.

Spawning stays in Swift because it is irreducibly platform-specific — and because
`perch-core` must remain free of filesystem writes, which CI enforces.

**How a terminal is launched without Automation permission:** Swift writes the one-liner to a
single-use script inside Perch's own application-support directory and runs
`open -a <TerminalApp> <script>`. Terminal.app and iTerm2 both execute a script opened this
way. AppleScript would need `NSAppleEventsUsageDescription` and a permission prompt; this needs
neither.

**Which terminal:** the application that owns the most live sessions, resolved by walking the
process tree from their pids; `Terminal.app` when nothing is running. A user override is a
settings item, which this milestone does not build.

**Jump** activates the owning *application* via `NSRunningApplication.activate()`, which needs
no special permission. Focusing the specific *window* would require Accessibility, so it is
out of scope and the fallback — reveal the `cwd` in Finder — is always offered.

**Perch never writes to the Claude Code directory.** These actions launch a process in a
project directory; the script lives in Perch's own data directory. The read-only promise is
unchanged, and CI's write audit continues to enforce it over `perch-core`.

## 7. Error handling

- Index unreadable → the sidebar shows an honest error row; the popover's behaviour is
  unchanged.
- A project whose directory no longer exists → its actions are disabled with the reason shown,
  rather than failing on click.
- A terminal that cannot be launched → the error surfaces in the window; nothing is silently
  swallowed. This is the defect the last milestone's final review found at the FFI seam, so the
  new FFI methods return typed errors rather than `Option`.
- Edits (note, pin, archive, rename) that fail → the previous value is restored and the error
  shown.

## 8. Testing

- **Rust** carries the behaviour: grouping and ordering of the project list; the 14-day
  sparkline including days with no activity; session history for a project with none; burn-rate
  suppressed when data is thin; `shell_line()` quoting for paths with spaces and quotes; every
  edit round-tripping through the database.
- **FFI:** each new method reaches its model, and a failure surfaces as a typed error.
- **Swift:** builds in CI; behaviour verified by hand.
- **CI:** the existing audits already cover the new Rust and Swift files by glob; the write
  audit must keep passing over `perch-core` with `actions.rs` present.

## 9. Cross-platform posture

Unchanged and reinforced. `ui::main_window`, `ui::usage`, and `actions` are platform-neutral;
`perch-ffi` gains no OS-conditional code. A Linux shell would render the same models and run the
same composed commands through its own launcher.

## 10. Decisions taken without the user

The user chose the scope ("everything") and both terminal actions, then delegated the rest.
These are the calls made on their behalf, each with what it costs if wrong:

| Decision | Why | If wrong |
|---|---|---|
| One app, second window — not a second binary | One engine, one index, one install | A window to move |
| Activation policy flips `.accessory` ↔ `.regular` with the window | A menu-bar app with an occasional window needs a Dock icon only while that window is up | One line |
| SwiftUI `NavigationSplitView`, AppKit-hosted | No `NSMenu` constraint here; the cards are already SwiftUI | Layout rework |
| Swift Charts, not a package | In the SDK; adding an SPM dependency would be Perch's first fetch and weakens the no-network story | Swap the chart layer |
| Window **pulls**; the popover keeps **pushing** | A window is read and edited, not glanced at; pushing would fight editing | Change one call site |
| Rust composes commands, Swift spawns | Keeps `perch-core` write-free (CI-enforced) and the logic reusable | Move one function |
| Script + `open -a`, not AppleScript | Avoids an Automation permission prompt entirely | Add the usage description and prompt |
| Jump activates the app, not the window | Per-window focus needs Accessibility; app activation needs nothing | A permission prompt and more code |
| Terminal inferred from the process tree | The user's real terminal, discovered rather than configured | A settings row |
