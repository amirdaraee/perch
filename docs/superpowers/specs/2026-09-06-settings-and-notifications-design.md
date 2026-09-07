# Perch settings, notifications, and diagnostics — design

**Date:** 2026-09-06
**Status:** Design decided. The user asked for the whole set, expressed in Perch's own idiom
rather than as a copy of the app it was researched from.
**Implements:** the 2026-08-30 spec's §9.5 (notifications) and the backlog's settings window,
plus a diagnostics surface and a versioned config file.
**Builds on:** the 2026-09-04 native-app design and the 2026-09-05 main-window design. Their
governing rule carries over unchanged.

---

## 1. Why, and why not shaped like the app we studied

Perch has no settings at all. Everything is a constant: a 5 s poll, a fixed menu-bar format,
`Terminal.app`, and no notifications — even though the problem the project started from is *a
session that sat blocked for 32 hours without telling anyone*.

We studied CodexBar's settings surface, which is unusually rich: ~90 options across eleven
panes, plus a repeating descriptor per provider across ~69 provider panes. The single most
useful thing in it is a three-state override — **Global / Custom / Off** — applied per provider.

Its shape follows from its subject. CodexBar manages many *services*, and a service is not a
place you visit; it is a row in a list. So its settings are provider-shaped: a Notifications
pane containing a row per provider.

**Perch's subject is different, so its shape is different.** Perch's entities are projects and
sessions, and a project *is* a place the user already visits — the detail pane, where they
already pin, archive, rename, and write a note. Putting a per-project override in a settings
list would mean maintaining a second, parallel list of every project, and a settings window
that grows a row each time the user starts work somewhere new.

> **The inversion:** global defaults live in Settings. Per-project overrides live **on the
> project**, beside the note. The three states are the same idea; where you set them is not.

Two consequences fall out, both good: the settings window stays small and finite, and the
override is visible exactly where its subject is.

## 2. Scope

**In**

- `perch-core::settings` — a versioned `config.toml`, watched for changes, with an env-var
  override and a migration that mirrors the database's version-gated shape.
- **Global settings** (§5), in a small settings window.
- **Per-project notification override** — `Default / Custom / Off`, stored on the project and
  edited in the project detail pane.
- **The waiting-on-you notification** (§6): edge-triggered, decided in Rust, delivered by Swift.
- **Diagnostics** (§7): what Perch sees, and why each session record was accepted or rejected.
- **Launch at login**, via `SMAppService` — in the SDK, no dependency.

**Out**

- The approaching-limit notification. It needs a real ceiling, which needs the tier-1 usage
  endpoint; §8 of the original spec forbids fabricating one. It lands with that endpoint.
- Hooks — running user-specified executables on session events. Genuinely tempting, and it
  does not itself break the read-only or no-network rules, but shipping a mechanism that runs
  arbitrary programs needs its own trust story. Backlog.
- Settings search, per-section reset, and a settings CLI. All become worthwhile at a size this
  settings surface is deliberately not reaching.
- Anything requiring the network: currency conversion, status checks, update checks, sync.

## 3. Architecture

```
crates/perch-core/src/
├── settings/
│   ├── mod.rs        Settings, Defaults, load/save, env override, versioned migration
│   └── watch.rs      config-file watcher (reuses the ui::watcher pattern)
├── notify.rs         NotificationDecision — edge-triggered, pure, testable
├── db.rs             + per-project notify override columns
└── ui/
    ├── settings.rs   SettingsModel — the settings window's view-model
    └── diagnostics.rs DiagnosticsModel — what Perch sees and why

crates/perch-ffi/     + settings read/write, diagnostics, a notification callback
apps/macos/Perch/Sources/Perch/
├── Settings/SettingsWindow.swift    small window, few panes
├── Settings/DiagnosticsView.swift   the "why isn't my session showing?" pane
└── Notifications/Notifier.swift     UNUserNotificationCenter delivery only
```

**The rule is unchanged:** Rust owns everything except drawing. Settings validation, defaults,
migration, the notification *decision*, and every displayed string live in `perch-core`, so a
later Linux or Windows shell reuses them and only supplies its own delivery mechanism.

## 4. The config file

`config.toml`, in Perch's own application-support directory beside `index.db`
(`~/Library/Application Support/Perch/` on macOS, `$XDG_DATA_HOME/perch/` elsewhere).
`PERCH_CONFIG` overrides the full path — the same precedence habit the data layer already has
for `CLAUDE_CONFIG_DIR` and `PERCH_DATA_DIR`.

**TOML, not JSON.** The file is watched and meant to be hand-edited; a config a user is invited
to edit but cannot annotate is worse. The parser is **`toml_edit`** — the crate Cargo itself
uses — because it is the one that preserves comments and key order across a write, which plain
serialization cannot. It brings eight small pure-parsing crates (`winnow`, `toml_parser`,
`toml_datetime`, `serde_spanned`, `serde_core`, `indexmap`, `hashbrown`, `equivalent`), none of
them capable of I/O beyond parsing, so the no-network CI audit is untouched.

It is written with a header comment and every key present at its default, so the file itself
documents the options. Unknown keys are preserved on rewrite rather than dropped — a user's
comment or a newer version's key must survive an older binary saving over it.

```toml
# Perch settings. Edit freely — Perch reloads this file when it changes.
version = 1

[general]
launch_at_login = false
claude_config_dir = ""        # blank = auto-detect; set this if Perch can't find your sessions

[menu_bar]
display = "count"             # "icon" | "count" | "count-and-waiting"

[sessions]
poll_seconds = 5              # 1–60

[notifications]
waiting_enabled = false       # off until you turn it on; macOS will ask permission
waiting_after_minutes = 10    # 1–240
include_background = false
```

**Migration** mirrors the database's shape exactly: a `version` key, a version-gated one-shot
migration, and the same "never destroy what the user owns" rule — unknown keys and comments
survive. The primitive is built now, while there is nothing to migrate, so the first real
schema change is not a special case.

**Watching** reuses `ui::watcher`'s established loop rather than inventing a second one:
debounced, with a poll backstop, and never creating the file it watches.

## 5. Global settings

Eight options, one small window. Each is here because something in the app is currently a
constant that should not be.

| Setting | Why it exists |
|---|---|
| Launch at login | A menu-bar app that must be started by hand is half-useless |
| Claude Code directory | A Finder-launched app does not inherit shell env, so `CLAUDE_CONFIG_DIR` never reaches it — this is the fix, and the first-run failure it repairs |
| Menu-bar display | Icon only · count · count with the waiting hourglass |
| Poll interval | 5 s suits most; a large index or a quiet machine may want otherwise |
| Notifications: waiting on you | Off by default; enabling it triggers the macOS permission prompt |
| …after N minutes | The threshold the alert fires at |
| …include background sessions | `kind: bg` excluded by default — they are not waiting on *you* |
| Preferred terminal | Where Resume and Open run. Detection by process tree is still backlogged; this makes the choice explicit meanwhile |

**Dependent controls are disabled, not hidden.** The three notification sub-options stay
visible and greyed when notifications are off. Hiding them makes the window twitch and hides
what is configurable.

## 6. Notifications

**The decision is Rust's; the delivery is Swift's.** `perch-core::notify` takes the live
session list, the settings, the per-project overrides, and the set of episodes already
notified, and returns zero or more `Notification { title, body, session_id }` — finished
strings, as everywhere else.

**Edge-triggered, per the original spec §9.5.** It fires once when a session crosses the
threshold and re-arms only after that session returns to working. Level-triggering would
re-fire every tick, which is how a helpful alert becomes an unusable one.

An episode is identified by `(session_id, waiting_since)`, so a session that blocks, resumes,
and blocks again is a new episode and notifies again — while the same continuous block never
notifies twice, even across a Perch restart, because the episode key is derived from data
Claude Code owns rather than from Perch's uptime.

Swift's `Notifier` does exactly one thing: hand the finished strings to
`UNUserNotificationCenter`. It requests authorization the first time the user enables the
setting, not at launch — a permission prompt on first run, before the app has shown its worth,
is the fastest way to get denied.

Clicking a notification opens the main window with that session's project selected.

## 7. Diagnostics

Not a debug dump. It answers one question — **"why isn't my session showing up?"** — and it is
the natural extension of a habit this codebase already has: never show a fabricated zero, always
say what is actually known.

It shows the resolved Claude Code directory and how it was resolved (env var, XDG, or default);
every session record file found; and for each, **accepted, or the specific reason it was
rejected** under the liveness rule — no such process, the pid belongs to something that is not
`claude`, or the record could not be parsed. Plus the config file path and whether it loaded,
the index path with its session and turn counts, and the last successful index time.

Reached from the settings window, not hidden behind a secret toggle. A user diagnosing a
problem should not have to know an incantation, and there is nothing here that is unsafe to
show — it is Perch describing its own inputs.

## 8. Error handling

- An unreadable or invalid `config.toml` → defaults are used, the app runs, and the settings
  window shows the parse error with the file path. Never a silent fallback: a user who
  hand-edited the file needs to know their edit did not take.
- A config write failure → surfaced in the settings window; the in-memory value is retained so
  the session's work is not lost.
- Notification authorization denied → the toggle reflects it and says how to change it in
  System Settings, rather than appearing on while delivering nothing.
- A per-project override on a project that has since vanished → harmless; it lives in the
  projects table and is ignored.

## 9. Testing

- **Rust carries the behaviour.** Defaults round-trip; every value validates at its bounds
  (`poll_seconds` 1–60, `waiting_after_minutes` 1–240); unknown keys and comments survive a
  save; a v0 file migrates without loss; a malformed file yields defaults *and* an error rather
  than a silent reset.
- **Notifications** are the most testable part and get the most tests: fires once at the
  threshold; does not re-fire while still waiting; re-arms after working; a new episode after
  resume-then-block notifies again; background sessions excluded unless enabled; a project set
  to Off never notifies; a project with a custom threshold uses it.
- **Diagnostics:** each rejection reason is produced for its own cause.
- **Swift:** builds in CI; behaviour verified by hand.

## 10. Cross-platform posture

`settings`, `notify`, `ui::settings`, and `ui::diagnostics` are platform-neutral. Only three
things are macOS-bound: `SMAppService`, `UNUserNotificationCenter`, and the default config
path — all already isolated behind the same seams the app-data path uses. A Linux shell reads
the same file, gets the same decisions, and supplies its own delivery.

## 11. Decisions

| Decision | Why | If wrong |
|---|---|---|
| Per-project overrides on the project, not in a settings list | Perch's entities are places the user already visits; a list would grow a row per project | Move one control |
| TOML over JSON | The file is watched and hand-edited; comments matter more than avoiding six pure-parser crates | Swap the parser |
| Config beside `index.db`, `PERCH_CONFIG` overrides | Matches the precedence habit already established twice | One path constant |
| Migration primitive built before it is needed | Cheapest exactly now, while nothing needs migrating | Unused code |
| Notification decision in Rust, delivery in Swift | The hard part is the edge-triggering, and it is the part another shell would otherwise reinvent | Move one function |
| Authorization requested on enable, not at launch | A prompt before the app has shown value gets denied | One call site |
| Diagnostics visible, not hidden behind a toggle | It exists for someone already having trouble; a secret is the wrong place for a remedy | Add a toggle |
| Approaching-limit notification deferred | Needs a real ceiling; §8 forbids inventing one | Ships with the endpoint |
| Hooks deferred | Running arbitrary user executables needs its own trust story | Backlog item |
