# Perch settings window — redesign

**Date:** 2026-09-06
**Status:** Scope chosen by the user: a comprehensive sidebar settings window, every worthwhile
constant unfrozen, plus six new capabilities (§6). Design decided by the assistant under that
delegation; the calls made on the user's behalf are listed in §11.
**Supersedes:** the settings-window portion of the 2026-09-06 settings-and-notifications design.
Everything else in that document — the config file, the notification engine, the per-project
override inversion, diagnostics — stands unchanged and is built on here.
**Builds on:** the 2026-09-04 native-app design and the 2026-09-05 main-window design. Their
governing rule carries over unchanged.

---

## 1. Why this is being redone

The first settings window shipped four tabs and eight options. The user's verdict was that it is
not comprehensive enough, in functionality or in UI, and they are right on both counts.

The functional miss was a design call I made and got wrong: I argued for a deliberately small and
finite settings surface, on the grounds that Perch's per-project overrides live on the project.
That inversion is still correct — it is what keeps Settings from growing a row per project — but
it does not justify leaving everything *else* in the app frozen. It answered a question nobody
asked.

The UI miss is simpler. A four-tab `TabView` is not what macOS settings look like any more. A
sidebar with panes is, and there is no argument for the tab bar beyond it having been quicker.

## 2. What actually makes a settings window feel comprehensive

We studied CodexBar's, which is unusually good. The instructive result is that **its option count
is not the reason.** Roughly half its ~90 options exist to manage 69 separate providers — plugin
installation, connection-method pickers, token accounts, org switchers, per-provider accent
colours, merged-icon provider selection. Perch has one provider, so none of that transfers. Three
more of its options are forbidden here outright: currency conversion, provider status pings, and
update checks are all network calls.

It also does **not** have settings search. Its single search field is scoped to the provider list
and does nothing for the eleven app panes. Its own densest pane holds fifteen controls in one flat
form. The wall-of-settings problem is distributed there, not solved.

What does transfer is craft, and it is copyable without copying a single option:

- **Dependent controls are disabled, not hidden.** The option stays visible and greyed, so the
  user learns it exists and why it is off. Hiding rows makes the window twitch.
- **Sub-options are indented under the master they depend on**, so the dependency is visible
  before you click anything.
- **Panes contain information, not only controls.** Read-only rows — a resolved path, a last-run
  timestamp, a live status under the setting it explains — are a large part of why a pane reads
  as substantial. This is the one we had most underweighted, and it is free: Perch already knows
  all of it.
- **A reset appears only once a value has drifted from its default**, and resets to a compile-time
  constant rather than to a snapshot of whatever the user last saw.
- **Numeric options carry real ranges and steppers**, not free text boxes.

## 3. Scope

**In**

- A sidebar settings window with **nine panes** (§5), modelled on macOS System Settings.
- **A declarative settings schema owned by Rust** (§4) — the architectural core of this milestone.
- Every constant worth unfreezing, per the inventory (§5), and the two defects that inventory
  turned up (§7).
- The six new capabilities (§6): settings search, per-pane reset, an editable model price table,
  a stale-data dimming threshold, burn-rate display modes, and menu-bar icon variants.
- `SETTINGS_VERSION` 1 → 2, with the first real migration (§8).

**Out**

- Hooks — running user-specified executables on session events. Unchanged from the previous
  spec: it needs its own trust story. Backlog.
- A settings CLI (`perch config dump/validate`). Natural, since Rust already owns validation, but
  it is a second entry point to keep in sync. Backlog.
- Import/export of settings. The config file is already a hand-editable TOML the user can copy.
- Anything requiring the network. Unchanged and non-negotiable.
- Making the sidebar's sort order configurable. It is a small feature rather than a constant, and
  it belongs with the main window rather than in this milestone.

## 4. Architecture: Rust describes the settings; Swift draws them

The governing rule already says Rust owns everything except drawing. A settings window is almost
entirely *not* drawing: pane titles, group headings, option labels, help text, units, bounds,
which control type each option wants, and which options a given option disables. That is content.

> **Rust emits a settings schema as data. Swift renders it with native controls.**

```
crates/perch-core/src/settings/
├── mod.rs        Settings (storage truth), SettingKey, validation
├── schema.rs     NEW  panes, groups, rows, labels, help, bounds, enablement
├── search.rs     NEW  filtering the schema by query
└── store.rs      load/save/watch/migrate — now driven by the schema
```

```rust
pub struct SettingsPane  { id: PaneId, title: String, icon: IconId, groups: Vec<SettingGroup> }
pub struct SettingGroup  { heading: Option<String>, rows: Vec<SettingRow> }
pub struct SettingRow {
    key: SettingKey,
    label: String,
    help: Option<String>,
    control: Control,
    enabled: bool,          // already resolved; Swift never evaluates a dependency
    is_default: bool,       // drives "reset appears only once drifted"
    indent: bool,
}
pub enum Control {
    Toggle  { on: bool },
    Stepper { value: i64, min: i64, max: i64, step: i64, value_label: String },
    Choice  { selected: String, options: Vec<ChoiceOption> },
    Text    { value: String, placeholder: String },
    Folder  { value: String, resolved_label: String },
    Action  { button_label: String, destructive: bool },
    Info    { value_label: String },
}
```

**Why this and not twenty-six hand-written SwiftUI controls:**

- **Search, per-pane reset, and disabled-not-hidden all become free and testable.** Each is a
  function over the schema rather than a hand-maintained parallel list that drifts from the real
  options. Search in particular is a Rust function with unit tests, not a Swift string comparison.
- **`enabled` is resolved in Rust.** Swift never asks "is notifications off?" — it reads a bool.
  Dependency logic is declared once, in the language that owns the rule, and tested there.
- **A later Linux or Windows shell gets the entire window description**, not just the values, and
  supplies only the controls.
- **`IconId` is semantic, not an SF Symbol name.** Rust emits `IconId::Notifications`; Swift maps
  it to `bell`. The symbol name is the one genuinely macOS-specific fact, so it is the one thing
  that stays in Swift.

**Where the schema stops.** Three panes are not lists of scalar options and get bespoke SwiftUI:
**Model Prices** (a table), **Diagnostics** (already bespoke, unchanged), and **About**. A generic
renderer forced over those would be worse than hand-writing them, and pretending otherwise is how
schema-driven UI earns its bad reputation.

**Values still cross as values.** A stepper needs a number to edit, so numbers cross the FFI as
numbers and their display strings cross as strings — the same split already ruled for chart values
and their labels. Swift still formats nothing.

### 4.1 What the compiler already catches, and the one thing it does not

Each setting is written out five times: the flat `Settings` field, the wire section field, that
section's `Default`, and both directions of the `From` conversion. At eight settings that is
tolerable. At twenty-six it wants a safety story — but a narrower one than it first appears.

**Omission is already safe.** Both `From` impls build struct literals, and a struct literal missing
a field does not compile. Adding a field to `Settings` therefore fails the build until it has been
threaded through a section and back. No test is needed for this and none should be written.

**Mis-mapping is the real hazard, and the compiler is blind to it.** Wiring `poll_seconds` to the
`waiting_after_minutes` key type-checks perfectly — both are `u32` — and the symptom is a setting
that silently loads as some other setting's value. Same-typed fields are common here: four `u32`
counts, several `bool` toggles.

Two mechanisms, in order of strength:

1. **`SettingKey` is an enum, and get/set over `Settings` is an exhaustive `match`.** This is what
   the schema, search, and per-pane reset all address settings through, and it is a compile error
   to add a key without handling it.
2. **A round-trip test sets every key to a distinct non-default value and asserts each survives
   save → load unchanged.** Distinct values are the point: a test using the same value for two
   fields would pass with them swapped. This is the only thing that catches mis-mapping.

Plus one coverage test — every `SettingKey` appears in exactly one pane of the schema — so a key
cannot exist with no way to reach it.

**A declarative macro was considered and rejected.** It would collapse all five places into one
declaration, but this codebase's style is explicit, greppable, heavily commented Rust, and a macro
generating a struct, its serialization, and its schema is none of those. With omission already
handled by the compiler, a macro would buy only the mis-mapping guarantee that one test buys more
cheaply.

### 4.2 A pane is not a TOML section

The schema's panes are a UI grouping; `[general]`, `[sessions]`, `[notifications]` are a storage
grouping. They are allowed to differ, and they do — "Preferred terminal" is shown in the General
pane while remaining under `[sessions]` on disk.

This matters because moving a key between TOML sections needs a migration, and reorganising the
window must not drag one behind it. Exactly one key moves on disk in this milestone (§8), because
exactly one was genuinely misfiled.

## 5. The panes

Nine panes. Every row below is either an existing setting, a constant the inventory judged worth
unfreezing, one of the six new capabilities, or a read-only fact Perch already knows.

### General
| Row | Control | Source |
|---|---|---|
| Launch at login | Toggle | existing |
| Claude Code folder | Folder (blank = auto-detect) | existing |
| → Resolved folder, and how it resolved | Info | new, read-only |
| Check for sessions every N seconds | Stepper 1–60 | existing (`poll_seconds`) |
| Preferred terminal | Choice, **from detected applications** | existing, defect fixed (§7) |
| → The command Resume will run | Info | new, read-only |

### Menu Bar
| Row | Control | Source |
|---|---|---|
| Icon | Choice of variants | **new capability** |
| Menu bar shows | Choice: icon / count / count and waiting | existing |
| Dim when data is stale | Toggle | **new capability** |
| → Consider data stale after N minutes | Stepper 1–120, indented | **new capability** |

### Popover
| Row | Control | Source |
|---|---|---|
| Show Waiting / Working / Recent sections | three Toggles, one group | new |
| Recent sessions shown | Stepper 1–20 | unfreeze `RECENT_LIMIT` |
| Row density | Choice: comfortable / compact | new |
| Show project folder on each row | Toggle | new |
| Show tokens and cost on each row | Toggle | new |

### Projects
| Row | Control | Source |
|---|---|---|
| A project is Active if used within N days | Stepper 1–90 | unfreeze `ACTIVE_WINDOW_MS` |
| Show the Archived group | Toggle | new |
| Days of history in charts | Choice: 7 / 14 / 30 / 90 | unfreeze `SPARK_DAYS` + `CHART_DAYS`, unified (§7) |

### Usage
| Row | Control | Source |
|---|---|---|
| Top projects shown | Stepper 3–20 | unfreeze `TOP_N` |
| Top projects measured over N days | Stepper 7–180 | unfreeze the inline `30 * DAY_MS` |
| Burn rate | Choice: off / tokens per hour / cost per hour / cost per day / projected window | **new capability** |
| Show cost estimates | Toggle | new |
| → Dollars are always an estimate | Info | new, read-only |

### Model Prices — bespoke
An editable table over the existing `prices` SQLite table: model id, and four per-million-token
rates (input, output, cache read, cache write). Add and remove a model; **Reset to built-in
prices**. Two read-only notes carry the facts the inventory confirmed: rates are per million
tokens, and a model with no price contributes its tokens but no cost rather than erroring.

### Notifications
| Row | Control | Source |
|---|---|---|
| Tell me when a session is waiting on me | Toggle | existing |
| → After N minutes | Stepper 1–240, indented | existing |
| → Include background sessions | Toggle, indented | existing, **moved here** (§8) |
| → Play a sound | Toggle, indented | new |
| Notification permission status | Info + Action | existing behaviour, surfaced |

### Diagnostics — bespoke, unchanged
Kept visible rather than gated behind an advanced toggle. CodexBar hides its Debug pane because it
is a developer surface; Perch's Diagnostics answers a *user's* question — "why isn't my session
showing up?" — and a remedy should not require an incantation.

### Advanced
| Row | Control | Source |
|---|---|---|
| Config file path | Info + Reveal | new, read-only |
| Index database path and size | Info | new, read-only |
| Sessions and turns indexed | Info | new, read-only |
| Last successful index | Info | new, read-only |
| Reindex now | Action | new |
| Reset all settings | Action, destructive | new |

**About** folds into Advanced as a final group — version, build, MIT licence, and links. It does
not earn its own pane in an app this size.

## 6. The six new capabilities

- **Settings search.** A field above the sidebar filters panes and rows by label and help text.
  Implemented in Rust over the schema, so it is unit-testable and a Linux shell gets it too. This
  is the one place we deliberately exceed what we studied.
- **Per-pane reset.** A pane's reset restores its keys to their compile-time defaults. Following
  the pattern worth stealing, the control appears only when at least one row in the pane has
  drifted, and the schema's `is_default` per row is what tells Swift when to show it.
- **Editable model prices.** §5. The storage already supports it; this is the UI it never had.
- **Stale-data dimming.** Perch polls; when the last successful read is older than the threshold,
  what it shows may be wrong. Rust computes staleness and emits a flag with a label; Swift dims.
  Honest by the same rule that forbids fabricating a rate-limit ceiling.
- **Burn-rate display modes.** The burn rate is already computed and already suppressed when the
  data is too thin to project honestly. This chooses its unit, and adds "off".
- **Menu-bar icon variants.** Replaces the hardcoded `NSImage(systemSymbolName: "bird")`. Rust
  emits an `IconId`; Swift maps it to an SF Symbol and renders it as a template image.

## 7. Two defects the inventory found

Both are fixed as part of this milestone, because both sit exactly where new settings land.

- **`CHART_DAYS` and `SPARK_DAYS` are two independent constants that both equal 14.** Exposing one
  as a setting would let the usage chart and the project sparkline silently disagree about their
  own date range. They collapse into the single "Days of history in charts" setting.
- **The terminal picker is hardcoded in Swift** to `Terminal` and `iTerm2`, while
  `preferred_terminal` is an unvalidated free string in Rust — so a Warp, Ghostty, Alacritty or
  Kitty user cannot choose their terminal, and the limitation lives in the view layer where the
  architecture says it must not. Rust supplies the choice list from detected applications.

## 8. Migration — the first real one

`SETTINGS_VERSION` goes 1 → 2, and it is an honest exercise of the primitive rather than a
manufactured one: **`include_background` moves from `[sessions]` to `[notifications]`.** It has
only ever been read by the notification engine; it was filed under sessions by mistake.

Everything else this milestone adds is a *new* key with a default, which needs no migration at all
— `#[serde(default)]` already handles a file that predates it. The move is the only change that
would lose a user's value in silence, which is precisely the case the primitive exists for.

Migration rules, unchanged from the database's shape: comments and unknown keys survive; the
migration runs once, gated on the version; a value that has been moved is carried across rather
than reset.

## 9. Error handling

Unchanged from the previous spec, with three additions:

- **A per-row validation note.** Because Rust validates on every `set`, an out-of-range value
  entered by hand reports which key was clamped and to what, against that row.
- **A price edit that cannot be parsed** leaves the stored rate untouched and reports it against
  the row. Cost display never silently changes to a number the user did not enter.
- **A failed reset** — per-pane or global — reports and changes nothing. Partial resets are worse
  than none.

## 10. Testing

- **The schema carries most of the new behaviour and most of the new tests.** Every `SettingKey`
  appears in exactly one pane. Every key round-trips through save and load at a distinct
  non-default value. Every bound clamps at both ends and reports. Every dependent row resolves
  `enabled` correctly with its master on and off.
- **Search:** matches on label and on help text; is case- and accent-insensitive; an empty query
  returns everything; a query matching nothing returns an empty result rather than everything.
- **Per-pane reset** restores exactly its own pane's keys and touches no other.
- **Migration:** a v1 file with `include_background` under `[sessions]` lands with the value
  preserved under `[notifications]`; comments and unknown keys survive; a v2 file is untouched.
- **Prices:** an edited rate survives re-seeding (already covered, must keep passing); reset
  restores the built-in table; an unknown model still contributes tokens and no cost.
- **Swift:** builds in CI; behaviour verified by hand.

## 11. Decisions taken without the user

| Decision | Why | If wrong |
|---|---|---|
| Rust emits a settings schema; Swift renders it | Search, reset, and enablement become testable Rust instead of duplicated Swift; a second shell gets the whole window | A larger FFI surface to unwind |
| Three panes stay bespoke Swift | A table, a diagnostic report, and an about box are not lists of scalar options | Hand-write more |
| Explicit structs plus exhaustive `match`, not a macro | The codebase's style is greppable, commented Rust; the compiler catches omission either way | Revisit at ~60 settings |
| Nine panes, About folded into Advanced | An about box does not earn a sidebar row in an app this size | Split one pane |
| Diagnostics stays visible, not gated | It is a user remedy, not a developer surface | Add the gate |
| 19 of 27 constants stay frozen | Several encode facts (Claude's real 5-hour window) or are welded to labels; exposing them would let the UI lie | Unfreeze one more |
| `CHART_DAYS` and `SPARK_DAYS` unify | Two constants equal to 14 would drift the moment one is exposed | Split them again |
| Terminal list comes from detected apps | The hardcoded pair excluded four common terminals, in the layer least entitled to decide | Ship a longer literal list |
| `include_background` moves sections | It is read only by notifications; filing it under sessions was a mistake, and the migration primitive exists for exactly this | Leave it and migrate later |
| Settings search, which the studied app lacks | The one place worth exceeding it; nearly free once the schema exists | Hide the field |
