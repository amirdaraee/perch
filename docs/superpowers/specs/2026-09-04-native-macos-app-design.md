# Perch native macOS app — design

**Date:** 2026-09-04
**Status:** Design approved in conversation; ready for implementation planning
**Supersedes:** the Tauri/React presentation layer from the 2026-08-30 design (§4, §9.1). The
data-layer sections of that spec remain binding.

---

## 1. Why

The Tauri popover cannot keep the macOS menu bar visible while open over a fullscreen app.
Two spikes proved why: only a *tracked `NSMenu`* holds the bar; an `NSPopover` or `NSPanel`,
even one owned by the status item, does not. CodexBar behaves correctly because its popover
is a real `NSMenu` whose items host SwiftUI views. A Tauri webview cannot live inside a menu
item, so the presentation layer moves to native AppKit/SwiftUI.

The user also wants Linux and Windows versions later. That sets the governing rule:

> **Rust owns everything except drawing.** Liveness, watching, usage math, formatting, and the
> view-model live in `perch-core`. Each platform shell is a renderer over a `PopoverModel`.

## 2. Scope

**In**
- A SwiftPM macOS app: status item, `NSMenu`, SwiftUI cards in menu items, Quit.
- The **richer popover** from the backlog, not parity with today's: per-row project · kind ·
  version; per-session tokens and ≈$; a Recent section; taller content.
- `perch-core::ui` — the view-model and the watcher, moved out of `src-tauri`.
- `perch-ffi` — UniFFI bindings; a script that builds the XCFramework.
- CI: build the framework and the app on macOS.

**Out**
- Main window, jump/resume, notifications, the rate-limit endpoint — later milestones, now
  targeting the native app.
- Deleting Tauri/React. They stay until the native app reaches parity, then a separate task
  removes them.
- Signing/notarisation, Homebrew — unchanged from the previous design (§10).

**Minimum macOS:** 15 Sequoia. Swift 6.x, SwiftPM only — no Xcode project.

## 3. Architecture

```
crates/perch-core/src/ui/
├── model.rs      PopoverModel + builders (pure; table-tested)
├── format.rs     human_tokens / human_cost / human_elapsed (moved from perch-cli)
└── watcher.rs    Watcher: notify + poll backstop + debounce (moved from src-tauri)

crates/perch-ffi/   UniFFI crate: `Perch` object, callback interface, staticlib
scripts/build-xcframework.sh
apps/macos/Perch/   SwiftPM executable
├── Package.swift          binaryTarget → PerchCore.xcframework
├── Sources/Perch/
│   ├── PerchApp.swift          NSApplicationDelegate, activation policy Accessory
│   ├── StatusItemController.swift   NSStatusItem, NSMenu, item ↔ card wiring
│   ├── Cards/StatsCard.swift        window · week · 24h (+ est)
│   ├── Cards/SessionRow.swift       dot · name · second line · elapsed · tokens
│   ├── Cards/RecentCard.swift       last three ended sessions
│   ├── Cards/EmptyCard.swift        no data / no config dir / error
│   └── Bridge/PerchEngine.swift     owns the FFI object; marshals callbacks to main
└── Makefile                 build, run, bundle (.app)
```

**`PopoverModel`** (Rust, `Serialize` + UniFFI record):
```
PopoverModel { stats: Stats?, live: [SessionRow], recent: [RecentRow],
               tray_title: String, error: String? }
Stats      { window_tokens, week_tokens, day_tokens: String (formatted),
             day_cost: String, estimated: bool, has_data: bool }
SessionRow { id, name, project, kind, version, status: Status,
             status_label: String, elapsed: String?, tokens: String?, cost: String? }
RecentRow  { id, name, project, ended_ago: String, tokens: String? }
Status     { Working | Idle | Waiting | Background }
```
Every string the UI shows is produced in Rust. Swift never formats numbers or durations.

**Watcher** (Rust): the existing debounced `notify` loop with the 5 s poll backstop, the
poll-only fallback when the sessions dir is absent, and the trailing-edge re-emit — moved
verbatim, minus Tauri. On each tick it rebuilds the model and invokes the callback.

**UniFFI surface** (`perch-ffi`):
```
interface Perch {
  constructor(config_dir: string?);        // null → resolve per spec §3
  PopoverModel current();                   // synchronous snapshot
  void start(PerchListener listener);       // reindex, then watch; idempotent
  void refresh();                           // reindex + emit now (menu open)
  void stop();
}
callback interface PerchListener { void on_model(PopoverModel model); }
```
Errors are a UniFFI `PerchError` enum: `NoConfigDir(path)`, `Database(msg)`, `Io(msg)`.

**Swift**: `PerchEngine` holds the `Perch` object and forwards `on_model` to the main actor.
`StatusItemController` builds one `NSMenu` per model; each section is an `NSMenuItem` with
`view = NSHostingView(rootView: Card(model:))`. `menuWillOpen` calls `refresh()`. The tray
title comes from `model.tray_title`. Quit is a plain item.

## 4. Data flow

Launch → `Perch(nil).start(listener)` → Rust: reindex (0.25 s cold, ~0.01 s warm), build
model, emit, start watcher → each emit → main-actor → rebuild card views (menu items are
reused; only their hosted views are replaced). Menu open → `refresh()` → same path. The
watcher also emits every 5 s, so usage stays fresh without a Swift timer.

Read-only invariant unchanged: only `perch-core` reads `~/.claude`; only it writes Perch's
own app-data directory. Swift touches neither.

## 5. Error handling

- `NoConfigDir` → `EmptyCard` with the resolved path and a one-line hint.
- Database errors → `EmptyCard("Index unavailable")`; sessions still show (they need no DB).
- Watcher failure → poll-only, logged, as today.
- A model with `has_data: false` renders `—` for stats and no `est` badge.

## 6. Testing

- Rust: `ui::model` table tests — blocked-first then name then pid; `—` for absent
  timestamps; `24h` labelling; `has_data` gating; recent = last three ended by
  `last_activity_at`. Existing 87 tests unchanged.
- FFI: one smoke test that constructs `Perch` against a temp config dir and receives a model.
- Swift: `swift build` in CI; behaviour verified by hand: bar stays visible over fullscreen,
  Escape closes, live update within ~2 s, click-away closes, tray click toggles.
- CI: `xcframework` job on macOS (cargo build staticlib → bindgen → xcframework → `swift build`).

## 7. Cross-platform posture

`perch-core::ui` and `perch-ffi` are platform-neutral. A Linux tray (e.g. libayatana via GTK)
or a Windows shell would link the same `perch-ffi` and render the same `PopoverModel`. The
`platform` traits (§4.1 of the previous spec) remain the only OS-bound Rust.

## 8. Decisions

| Decision | Why |
|---|---|
| Native AppKit `NSMenu`, not `MenuBarExtra`/popover | Only a tracked menu holds the fullscreen menu bar (two spikes) |
| UniFFI, not subprocess | In-process, typed, callback-driven; higher integration quality |
| View-model in Rust | Linux/Windows reuse; one place for formatting rules |
| macOS 15 minimum | User's choice |
| Keep Tauri/React until parity | User's choice; removal is its own task |
| SwiftPM executable, no Xcode project | Matches CodexBar; scriptable CI |
