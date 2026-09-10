# Settings Window Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Perch's four-tab, eight-option settings window with a nine-pane sidebar carrying twenty-six settings, driven by a settings schema that Rust owns and Swift renders.

**Architecture:** `perch-core` gains a declarative schema — panes, groups, rows, labels, help text, bounds, control kinds, and already-resolved enablement — built from a `Settings` value. Search, per-pane reset, and disabled-not-hidden become functions over that schema rather than parallel lists maintained in Swift. Three panes whose content is not a list of scalar options (Model Prices, Diagnostics, About) stay bespoke SwiftUI.

**Tech Stack:** Rust 2021, `rusqlite` (bundled), `toml_edit`, UniFFI 0.32 proc-macro style, SwiftPM executable, SwiftUI + AppKit, macOS 15 minimum.

**Spec:** `docs/superpowers/specs/2026-09-06-settings-window-redesign.md`

## Global Constraints

Every task's requirements implicitly include this section.

- **Rust owns everything except drawing.** Every string the UI shows is produced in `perch-core`. Swift never formats a number, a duration, or a currency, and never compares against a Rust sentinel. A number crossing the FFI to drive a stepper is fine; a caption composed in Swift from that number is not. This rule has produced three defects in the last milestone alone — treat it as binding.
- **Perch is read-only with respect to the user's Claude Code directory.** It never writes to, moves, or deletes anything under `~/.claude`. CI enforces this over `perch-core` by exact path exclusion.
- **No network requests, no telemetry**, anywhere, in any language. Six CI guards enforce this, each with a positive-control self-test.
- **No new dependencies**, Cargo or SPM. Swift Charts and `SMAppService` ship in the SDK; that is the bar.
- **No transcript or message contents** may enter a view-model, a schema row, or a notification. Names, cwds, statuses, counts, and formatted totals only.
- **Never read `apps/macos/Perch/Sources/PerchFFI/perch_ffi.swift`** — 49 KB of generated code that has stalled several agents. Regenerate it with `./scripts/build-xcframework.sh`; never open it.
- **Generated output is git-ignored and must never be committed** — `apps/macos/Perch/Sources/PerchFFI/` and `apps/macos/Perch/Frameworks/`. Always `git add` explicit paths; never `git add -A`.
- **Test baseline is 234 + 9 + 24.** `ui::watcher::tests::refresh_forces_an_immediate_emit` and `stop_from_inside_the_callback_does_not_deadlock_or_panic` are known wall-clock flakes; note and re-run, never "fix".
- **Commit trailers**, on every commit:
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01JSsLEdzWiLDMmcwvUkH7Xy
  ```

## File Structure

**Created**

| File | Responsibility |
|---|---|
| `crates/perch-core/src/settings/keys.rs` | `SettingKey`, `SettingValue`, exhaustive `get`/`set` over `Settings` |
| `crates/perch-core/src/settings/schema.rs` | `PaneId`, `IconId`, `SettingsPane`, `SettingGroup`, `SettingRow`, `Control`, `build_schema`, `reset_pane` |
| `crates/perch-core/src/settings/search.rs` | `filter_panes` — matching a query against labels and help text |
| `crates/perch-core/src/terminals.rs` | Detecting installed terminal applications; supplies the Preferred terminal choice list |
| `apps/macos/Perch/Sources/Perch/Settings/SchemaPane.swift` | Generic renderer: one SwiftUI control per `Control` variant |
| `apps/macos/Perch/Sources/Perch/Settings/PricesPane.swift` | Bespoke editable model-price table |
| `apps/macos/Perch/Sources/Perch/Settings/AdvancedPane.swift` | Paths, index stats, reindex, reset-all, and the About group |

**Modified**

| File | Change |
|---|---|
| `crates/perch-core/src/settings/mod.rs` | `Settings` grows from 8 to 26 fields; new enums; `validated` gains their bounds |
| `crates/perch-core/src/settings/store.rs` | `SETTINGS_VERSION` 1 → 2; the `include_background` section move; template regenerated |
| `crates/perch-core/src/pricing.rs` | `all_prices`, `set_price`, `remove_price`, `reset_to_defaults` |
| `crates/perch-core/src/ui/model.rs` | `RECENT_LIMIT`, popover section visibility, density, staleness |
| `crates/perch-core/src/ui/main_window.rs` | `ACTIVE_WINDOW_MS` and `SPARK_DAYS` from settings |
| `crates/perch-core/src/ui/usage.rs` | `TOP_N`, top-projects range, chart days, burn-rate mode; `CHART_DAYS` deleted |
| `crates/perch-ffi/src/lib.rs` | Mirrors for schema, search, set, reset, prices, terminals |
| `apps/macos/Perch/Sources/Perch/Settings/SettingsWindow.swift` | Rewritten: sidebar, search field, pane routing |
| `apps/macos/Perch/Sources/Perch/StatusItemController.swift` | Icon variants, stale dimming |

---

### Task 1: Grow `Settings` from eight fields to twenty-six

**Files:**
- Modify: `crates/perch-core/src/settings/mod.rs`
- Modify: `crates/perch-core/src/settings/store.rs` (the `TEMPLATE` constant and `save`'s key assignments)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Settings` with the twenty-six fields named below; the enums `MenuBarIcon`, `RowDensity`, `BurnRate`; `Settings::validated` clamping each bounded field. Every later task addresses these exact field names.

The disk layout keeps `[general]`, `[menu_bar]`, `[sessions]`, `[notifications]` and adds `[popover]`, `[projects]`, `[usage]`. **A TOML section is not a UI pane** — `poll_seconds` and `preferred_terminal` stay under `[sessions]` while appearing in the General pane. Only one key changes section, in Task 3.

| Section | Keys |
|---|---|
| `[general]` | `launch_at_login`, `claude_config_dir` |
| `[menu_bar]` | `display`, `icon`, `dim_when_stale`, `stale_after_minutes` |
| `[sessions]` | `poll_seconds`, `preferred_terminal` |
| `[popover]` | `show_waiting`, `show_working`, `show_recent`, `recent_limit`, `row_density`, `show_row_folder`, `show_row_usage` |
| `[projects]` | `active_within_days`, `show_archived`, `chart_days` |
| `[usage]` | `top_projects_count`, `top_projects_days`, `burn_rate`, `show_cost` |
| `[notifications]` | `waiting_enabled`, `waiting_after_minutes`, `include_background`, `sound` |

- [ ] **Step 1: Write the failing round-trip test**

This is the test the spec identifies as load-bearing: the compiler already catches an *omitted* field (both `From` impls build struct literals), but it cannot see a field wired to the **wrong key**, because so many fields share a type. Distinct values are the entire point — a test reusing one value would pass with two fields swapped.

Add to `crates/perch-core/src/settings/store.rs`'s test module:

```rust
#[test]
fn every_field_survives_a_save_and_load_at_a_distinct_value() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // Each numeric field gets a different in-range number, and each bool the
    // opposite of its default, so a field wired to another field's key fails
    // rather than coincidentally matching.
    let want = Settings {
        launch_at_login: true,
        claude_config_dir: "/tmp/some-claude-dir".to_string(),
        menu_bar_display: MenuBarDisplay::CountAndWaiting,
        menu_bar_icon: MenuBarIcon::Binoculars,
        dim_when_stale: false,
        stale_after_minutes: 11,
        poll_seconds: 12,
        preferred_terminal: "iTerm2".to_string(),
        show_waiting: false,
        show_working: false,
        show_recent: false,
        recent_limit: 13,
        row_density: RowDensity::Compact,
        show_row_folder: true,
        show_row_usage: false,
        active_within_days: 14,
        show_archived: false,
        chart_days: 30,
        top_projects_count: 15,
        top_projects_days: 16,
        burn_rate: BurnRate::TokensPerHour,
        show_cost: false,
        waiting_enabled: true,
        waiting_after_minutes: 17,
        include_background: true,
        sound: false,
    };

    save(&path, &want).expect("save");
    let got = load(&path);
    assert!(got.error.is_none(), "reload reported: {:?}", got.error);
    assert_eq!(got.settings, want, "a field did not survive the round trip");
}
```

`Settings` needs `PartialEq` for this. Add it to the existing derive.

- [ ] **Step 2: Run it and watch it fail**

Run: `source "$HOME/.cargo/env" && cargo test -p perch-core every_field_survives`
Expected: FAIL to compile — `struct Settings has no field named menu_bar_icon`. That is the honest red for fields that do not exist yet.

- [ ] **Step 3: Add the three enums**

In `crates/perch-core/src/settings/mod.rs`, beside the existing `MenuBarDisplay`. Follow its established shape exactly: a `KNOWN_WIRE_VALUES` const, an `as_wire_str`, and a hand-written `Deserialize` that **can never return `Err`** — an unrecognised value degrades to the default rather than failing the whole file, because one unknown word must not cost the user every other setting.

```rust
/// Which glyph the status item draws. Semantic rather than an SF Symbol
/// name: the symbol is the one genuinely macOS-specific fact here, so the
/// shell maps this to its own glyph and a Linux shell picks a different one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuBarIcon {
    #[default]
    Bird,
    Binoculars,
    Dot,
    Bars,
}

impl MenuBarIcon {
    pub const KNOWN_WIRE_VALUES: [&'static str; 4] = ["bird", "binoculars", "dot", "bars"];

    pub fn as_wire_str(self) -> &'static str {
        match self {
            MenuBarIcon::Bird => "bird",
            MenuBarIcon::Binoculars => "binoculars",
            MenuBarIcon::Dot => "dot",
            MenuBarIcon::Bars => "bars",
        }
    }

    fn from_wire_str(s: &str) -> Self {
        match s {
            "binoculars" => MenuBarIcon::Binoculars,
            "dot" => MenuBarIcon::Dot,
            "bars" => MenuBarIcon::Bars,
            _ => MenuBarIcon::Bird,
        }
    }
}
```

`RowDensity { Comfortable, Compact }` and `BurnRate { Off, TokensPerHour, CostPerHour, CostPerDay, ProjectedWindow }` follow the identical pattern with wire strings `"comfortable"`/`"compact"` and `"off"`/`"tokens-per-hour"`/`"cost-per-hour"`/`"cost-per-day"`/`"projected-window"`.

Copy the `Serialize`/`Deserialize` impls from `MenuBarDisplay` verbatim, substituting the type and its `from_wire_str`. Do not derive them: the derive would reject an unknown value and fail the file.

- [ ] **Step 4: Add the eighteen new fields**

Extend `Settings` with the fields named in the test, and add three new wire sections mirroring the existing `SessionsSection` pattern — a `#[serde(default)]` struct plus a hand-written `Default` that reads from `Settings::default()`, so a default is stated once.

Defaults, chosen to preserve today's behaviour exactly so that upgrading changes nothing a user sees:

| Field | Default | Why |
|---|---|---|
| `menu_bar_icon` | `Bird` | what `StatusItemController` hardcodes today |
| `dim_when_stale` | `true` | the honest default; staleness is currently invisible |
| `stale_after_minutes` | `5` | one minute of slack over the 5 s poll's worst case |
| `show_waiting` / `show_working` / `show_recent` | `true` | today's popover shows all three |
| `recent_limit` | `3` | today's `RECENT_LIMIT` |
| `row_density` | `Comfortable` | today's layout |
| `show_row_folder` | `false` | not shown today |
| `show_row_usage` | `true` | shown today |
| `active_within_days` | `7` | today's `ACTIVE_WINDOW_MS` |
| `show_archived` | `true` | the group exists today |
| `chart_days` | `14` | today's `SPARK_DAYS` and `CHART_DAYS` |
| `top_projects_count` | `8` | today's `TOP_N` |
| `top_projects_days` | `30` | today's inline `30 * DAY_MS` |
| `burn_rate` | `CostPerHour` | today's burn-rate line |
| `show_cost` | `true` | shown today |
| `sound` | `true` | the platform default for a delivered alert |

- [ ] **Step 5: Extend `validated` with the new bounds**

Each bounded field clamps **and reports**, exactly as the two existing ones do — an out-of-range hand edit still runs, and the user is told it was changed. Add a helper so seven near-identical blocks do not accumulate:

```rust
fn clamp_note(name: &str, value: u32, lo: u32, hi: u32, notes: &mut Vec<String>) -> u32 {
    let clamped = value.clamp(lo, hi);
    if clamped != value {
        notes.push(format!("{name} was {value}; clamped to {clamped}"));
    }
    clamped
}
```

Bounds: `poll_seconds` 1–60, `waiting_after_minutes` 1–240, `stale_after_minutes` 1–120, `recent_limit` 1–20, `active_within_days` 1–90, `top_projects_count` 3–20, `top_projects_days` 7–180.

`chart_days` is a **choice**, not a range: an unrecognised value snaps to the nearest of 7, 14, 30, 90 and reports it.

Keep the existing directory check on `claude_config_dir` added in `b47a8bb` untouched.

- [ ] **Step 6: Extend `store.rs`'s `TEMPLATE` and `save`**

Every key appears in `TEMPLATE` at its default with a short comment, so the written file documents itself. Add the assignments in `save` for each new key, using `ensure_table` for the three new sections.

- [ ] **Step 7: Run the test and the suite**

Run: `cargo test -p perch-core settings`
Expected: PASS, including the round-trip test.

Then `cargo test --workspace` — expect the 234/9/24 baseline plus your new tests, and `cargo fmt --all && cargo clippy --workspace --all-targets`.

- [ ] **Step 8: Commit**

```bash
git add crates/perch-core/src/settings/mod.rs crates/perch-core/src/settings/store.rs
git commit -m "feat(settings): grow the settings file to twenty-six keys"
```

---

### Task 2: `SettingKey` and typed get/set

**Files:**
- Create: `crates/perch-core/src/settings/keys.rs`
- Modify: `crates/perch-core/src/settings/mod.rs` (add `mod keys; pub use keys::*;`)

**Interfaces:**
- Consumes: `Settings` and its three enums from Task 1.
- Produces: `SettingKey` (26 variants), `SettingValue`, `Settings::get(SettingKey) -> SettingValue`, `Settings::set(SettingKey, SettingValue) -> Result<(), SetError>`, and `SettingKey::ALL`. Tasks 4, 5, 6 and 11 all address settings exclusively through these.

This is where the spec's compile-time safety comes from: `get` and `set` are exhaustive `match`es, so adding a field to `Settings` without giving it a key fails the build.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn every_key_round_trips_through_get_and_set() {
    for key in SettingKey::ALL {
        let mut s = Settings::default();
        let original = s.get(key);
        let changed = flip(&original); // a different value of the same shape
        s.set(key, changed.clone()).expect("set accepts its own shape");
        assert_eq!(s.get(key), changed, "{key:?} did not store what it was given");
        assert_ne!(s.get(key), original, "{key:?} ignored the write");
    }
}

#[test]
fn setting_a_key_to_the_wrong_shape_is_an_error_not_a_panic() {
    let mut s = Settings::default();
    let err = s.set(SettingKey::PollSeconds, SettingValue::Bool(true));
    assert!(err.is_err(), "a bool is not a poll interval");
}
```

`flip` is a test helper: `Bool(b) => Bool(!b)`, `Int(n) => Int(n + 1)`, `Text(t) => Text(format!("{t}x"))`, `Choice(c) => Choice(<a different valid wire value>)`.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p perch-core every_key_round_trips`
Expected: FAIL to compile — `SettingKey` does not exist.

- [ ] **Step 3: Write `keys.rs`**

```rust
/// Every setting, addressed as data. The schema, search, per-pane reset and
/// the FFI all go through these rather than through field access, so a new
/// setting cannot be added without the compiler demanding it be handled in
/// each of those places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingKey {
    LaunchAtLogin,
    ClaudeConfigDir,
    MenuBarDisplay,
    MenuBarIcon,
    DimWhenStale,
    StaleAfterMinutes,
    PollSeconds,
    PreferredTerminal,
    ShowWaiting,
    ShowWorking,
    ShowRecent,
    RecentLimit,
    RowDensity,
    ShowRowFolder,
    ShowRowUsage,
    ActiveWithinDays,
    ShowArchived,
    ChartDays,
    TopProjectsCount,
    TopProjectsDays,
    BurnRate,
    ShowCost,
    WaitingEnabled,
    WaitingAfterMinutes,
    IncludeBackground,
    Sound,
}

impl SettingKey {
    pub const ALL: [SettingKey; 26] = [ /* every variant, in the order above */ ];
}

/// The four shapes a setting's value can take at the schema boundary. A
/// `Choice` carries the wire string its enum already defines, so no third
/// spelling of these values exists.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Text(String),
    Choice(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetError {
    pub key_label: String,
    pub message: String,
}
```

`get` and `set` are single exhaustive `match`es over `SettingKey`. `set` returns `SetError` when the `SettingValue` shape does not match the key, and **does not clamp** — clamping stays in `validated`, called once on the way to disk, so there is exactly one place that decides bounds.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p perch-core keys`
Expected: PASS.

- [ ] **Step 5: Add the count guard**

```rust
#[test]
fn all_lists_every_key_exactly_once() {
    let mut seen: Vec<SettingKey> = SettingKey::ALL.to_vec();
    let before = seen.len();
    seen.sort_by_key(|k| format!("{k:?}"));
    seen.dedup();
    assert_eq!(seen.len(), before, "ALL contains a duplicate");
    assert_eq!(before, 26, "ALL is missing a key, or gained one without this count");
}
```

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/settings/keys.rs crates/perch-core/src/settings/mod.rs
git commit -m "feat(settings): address every setting by a typed key"
```

---

### Task 3: The v1 → v2 migration

**Files:**
- Modify: `crates/perch-core/src/settings/store.rs`
- Modify: `crates/perch-core/src/settings/mod.rs` (`SETTINGS_VERSION` 1 → 2)

**Interfaces:**
- Consumes: Task 1's sections.
- Produces: a `migrate` arm that moves `include_background` from `[sessions]` to `[notifications]`.

Every other key this milestone adds is *new* with a default, which `#[serde(default)]` already handles — a v1 file simply lacks them and gets defaults. **One key genuinely moves**, and it is the only change that would otherwise lose a user's value in silence. This is the first real exercise of a primitive built one milestone early, on purpose.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn v1_carries_include_background_into_the_notifications_section() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# my own note\nversion = 1\n\n[sessions]\npoll_seconds = 9\ninclude_background = true\n",
    )
    .unwrap();

    let loaded = load(&path);
    assert!(loaded.settings.include_background, "the value must survive the move");
    assert_eq!(loaded.settings.poll_seconds, 9, "its neighbours are untouched");

    save(&path, &loaded.settings).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# my own note"), "comments survive a migrating save");
    assert!(!text.contains("[sessions]\npoll_seconds = 9\ninclude_background"),
            "the key no longer lives under [sessions]");
}

#[test]
fn a_v2_file_is_left_alone() {
    // A file already at version 2 with include_background under [notifications]
    // must not be re-migrated, and must not pick up a stale [sessions] copy.
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p perch-core v1_carries_include_background`
Expected: FAIL — `include_background` is false, because the v1 file's value sits under a section the v2 reader no longer looks in.

- [ ] **Step 3: Implement the migration arm**

`migrate` already runs on the `DocumentMut` **before deserialization** — a documented forward hazard, and here it is exactly what is wanted. Guard on the version, move the item if present, remove the old key, and set `version = 2`. Removing the source key is what stops a later hand-edit of the dead `[sessions]` copy from appearing to work.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p perch-core settings::store`
Expected: PASS, with the existing migration and format-preservation tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/perch-core/src/settings/mod.rs crates/perch-core/src/settings/store.rs
git commit -m "feat(settings): migrate include_background to the section that reads it"
```

---

### Task 4: The settings schema

**Files:**
- Create: `crates/perch-core/src/settings/schema.rs`
- Modify: `crates/perch-core/src/settings/mod.rs`

**Interfaces:**
- Consumes: `SettingKey`, `SettingValue`, `Settings`.
- Produces: `PaneId`, `IconId`, `SettingsPane`, `SettingGroup`, `SettingRow`, `Control`, `ChoiceOption`, and `build_schema(&Settings, &SchemaContext) -> Vec<SettingsPane>`. Tasks 5, 6, 11 and 12 consume these.

`SchemaContext` carries the facts the schema needs but does not own — the detected terminal list, the resolved Claude directory and how it resolved, the config path, the index statistics. Passing them in keeps `build_schema` a pure function of its inputs and therefore trivially testable.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_setting_key_appears_in_exactly_one_pane() {
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    let mut found: Vec<SettingKey> = panes
        .iter()
        .flat_map(|p| p.groups.iter())
        .flat_map(|g| g.rows.iter())
        .filter_map(|r| r.key)
        .collect();
    let before = found.len();
    found.sort_by_key(|k| format!("{k:?}"));
    found.dedup();
    assert_eq!(found.len(), before, "a key appears in two panes");
    assert_eq!(before, SettingKey::ALL.len(), "a key has no row to reach it from");
}

#[test]
fn a_dependent_row_is_disabled_when_its_master_is_off() {
    let mut s = Settings::default();
    s.waiting_enabled = false;
    let panes = build_schema(&s, &SchemaContext::empty());
    let row = find_row(&panes, SettingKey::WaitingAfterMinutes);
    assert!(!row.enabled, "the threshold means nothing while alerts are off");

    s.waiting_enabled = true;
    let panes = build_schema(&s, &SchemaContext::empty());
    assert!(find_row(&panes, SettingKey::WaitingAfterMinutes).enabled);
}

#[test]
fn a_row_knows_whether_it_still_holds_its_default() {
    let mut s = Settings::default();
    assert!(find_row(&build_schema(&s, &SchemaContext::empty()), SettingKey::PollSeconds).is_default);
    s.poll_seconds = 30;
    assert!(!find_row(&build_schema(&s, &SchemaContext::empty()), SettingKey::PollSeconds).is_default);
}

#[test]
fn every_stored_setting_explains_itself() {
    // The first settings window was rejected as "very blank". The single
    // biggest reason a pane reads as substantial is that each row says what
    // it does underneath its label -- a sentence, not a fragment. A row that
    // stores something and explains nothing is the defect this catches.
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    for row in all_rows(&panes) {
        let Some(key) = row.key else { continue };
        let help = row.help.as_deref().unwrap_or("");
        assert!(!help.is_empty(), "{key:?} has no help text");
        assert!(
            help.len() >= 20 && help.ends_with('.'),
            "{key:?}'s help is not a sentence: {help:?}"
        );
    }
}

#[test]
fn every_group_is_titled() {
    // Grouping is what turns a list into a form. A group with no heading is
    // an ungrouped list wearing a card.
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    for pane in &panes {
        for group in &pane.groups {
            assert!(group.heading.is_some(), "{:?} has an untitled group", pane.id);
        }
    }
}

#[test]
fn every_stepper_carries_a_finished_caption() {
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    for row in all_rows(&panes) {
        if let Control::Stepper { value_label, .. } = &row.control {
            assert!(!value_label.is_empty(), "{:?} left its caption to the shell", row.key);
            assert!(!value_label.ends_with(" 1 minutes"), "caption is not pluralized");
        }
    }
}
```

The last test is the structural defence against this project's most-repeated defect. A stepper that crosses without a finished caption is how "Check every 1 seconds" reached a shipped build twice.

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p perch-core schema`
Expected: FAIL to compile — `build_schema` does not exist.

- [ ] **Step 3: Write the types**

```rust
/// A pane's identity, used by the shell for selection and by `reset_pane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    General, MenuBar, Popover, Projects, Usage, Prices, Notifications, Diagnostics, Advanced,
}

/// A semantic icon, never an SF Symbol name. The shell maps this to whatever
/// glyph its platform draws; the mapping is the one macOS-specific fact in
/// the settings window, so it is the one thing that stays in Swift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconId {
    General, MenuBar, Popover, Projects, Usage, Prices, Notifications, Diagnostics, Advanced,
}

pub struct SettingRow {
    /// `None` for a row that shows something rather than storing it — an
    /// `Info` row, or an `Action` button.
    pub key: Option<SettingKey>,
    pub label: String,
    pub help: Option<String>,
    pub control: Control,
    /// Already resolved. The shell reads a bool; it never evaluates a
    /// dependency, so the rule lives in one language and is tested there.
    pub enabled: bool,
    /// Drives "a reset appears only once a value has drifted".
    pub is_default: bool,
    /// Renders indented under the master it depends on.
    pub indent: bool,
    /// Extra terms search should match beyond label and help — a synonym a
    /// user is likely to type ("hourglass" for the waiting icon).
    pub search_terms: Vec<String>,
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

- [ ] **Step 4: Write `build_schema`**

One function per pane, each returning a `SettingsPane`, so no single function grows past reading length. Dependent rows set `indent: true` and compute `enabled` from their master — for example every notification sub-row is `enabled: s.waiting_enabled`, and `stale_after_minutes` is `enabled: s.dim_when_stale`.

**Every `Stepper` composes its `value_label` here**, reusing the `plural` helper. Move `plural` from `ui/main_window.rs` to a shared location rather than copying it; a second copy is how two spellings of "1 minute" appear.

The Prices and Diagnostics panes appear in the returned list with a title, an icon and **no rows** — they exist so the sidebar and search know about them, while their content is drawn bespoke.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p perch-core schema`
Expected: PASS, all four.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/settings/schema.rs crates/perch-core/src/settings/mod.rs crates/perch-core/src/ui/main_window.rs
git commit -m "feat(settings): describe the settings window in Rust"
```

---

### Task 5: Search and per-pane reset

**Files:**
- Create: `crates/perch-core/src/settings/search.rs`
- Modify: `crates/perch-core/src/settings/schema.rs` (add `reset_pane`)

**Interfaces:**
- Consumes: `build_schema`'s output, `SettingKey::ALL`.
- Produces: `filter_panes(&[SettingsPane], &str) -> Vec<SettingsPane>` and `reset_pane(&mut Settings, PaneId)`.

Both are functions over the schema. This is the payoff the spec argues for: neither needs a hand-maintained list that can drift from the real options, and both are testable without a UI.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn search_matches_a_label() {
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    let hits = filter_panes(&panes, "terminal");
    assert_eq!(count_rows(&hits), 1);
    assert_eq!(first_row(&hits).key, Some(SettingKey::PreferredTerminal));
}

#[test]
fn search_matches_help_text_not_only_labels() {
    // "hourglass" appears in the menu-bar display option's help, not its label.
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    assert!(count_rows(&filter_panes(&panes, "hourglass")) > 0);
}

#[test]
fn search_ignores_case_and_surrounding_space() {
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    assert_eq!(count_rows(&filter_panes(&panes, "  TERMINAL ")), 1);
}

#[test]
fn an_empty_query_returns_everything_unchanged() {
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    assert_eq!(count_rows(&filter_panes(&panes, "   ")), count_rows(&panes));
}

#[test]
fn a_query_that_matches_nothing_returns_nothing() {
    // The failure mode worth naming: a filter that falls back to "everything"
    // when it matches nothing is indistinguishable from a broken filter.
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    assert_eq!(count_rows(&filter_panes(&panes, "zzzznotasetting")), 0);
}

#[test]
fn a_pane_that_keeps_no_rows_is_dropped_entirely() {
    let panes = build_schema(&Settings::default(), &SchemaContext::empty());
    let hits = filter_panes(&panes, "terminal");
    assert_eq!(hits.len(), 1, "only the pane holding the match survives");
}

#[test]
fn resetting_a_pane_restores_only_its_own_keys() {
    let mut s = Settings::default();
    s.poll_seconds = 30;          // General
    s.waiting_after_minutes = 99; // Notifications

    reset_pane(&mut s, PaneId::General);

    assert_eq!(s.poll_seconds, Settings::default().poll_seconds, "its own key resets");
    assert_eq!(s.waiting_after_minutes, 99, "another pane's key is untouched");
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p perch-core search`
Expected: FAIL to compile — `filter_panes` does not exist.

- [ ] **Step 3: Implement**

`filter_panes` lowercases and trims the query, matches against label, help and `search_terms`, keeps matching rows, and drops panes left with none. A pane whose *title* matches keeps all its rows — searching "prices" should reach a bespoke pane that has no rows to match.

`reset_pane` walks the pane's rows, and for each `Some(key)` writes `Settings::default().get(key)` back through `set`. Going through `get`/`set` rather than field assignment is what keeps this correct when a key is added: the schema coverage test from Task 4 guarantees the key is in some pane, and `set` is exhaustive.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p perch-core search reset_pane`
Expected: PASS, all seven.

- [ ] **Step 5: Commit**

```bash
git add crates/perch-core/src/settings/search.rs crates/perch-core/src/settings/schema.rs crates/perch-core/src/settings/mod.rs
git commit -m "feat(settings): search the schema, and reset one pane of it"
```

---

### Task 6: Unfreeze the constants

**Files:**
- Modify: `crates/perch-core/src/ui/model.rs` (`RECENT_LIMIT`)
- Modify: `crates/perch-core/src/ui/main_window.rs` (`ACTIVE_WINDOW_MS`, `SPARK_DAYS`)
- Modify: `crates/perch-core/src/ui/usage.rs` (`CHART_DAYS`, `TOP_N`, the inline `30 * DAY_MS`)

**Interfaces:**
- Consumes: `Settings` fields from Task 1.
- Produces: the same builder functions, each now taking `&Settings` where it previously read a constant.

Five constants become settings and **one constant disappears**: `CHART_DAYS` and `SPARK_DAYS` are two independent declarations that both equal 14, and exposing either alone would let the usage chart and the project sparkline silently disagree about their own date range. They collapse into `chart_days`.

The four constants that stay — `FIVE_HOURS_MS`, `DAY_MS`, `WEEK_MS`, `WINDOW_MS`, `MIN_PROJECTABLE_MS` — are **not** touched. They encode facts about Claude's real rate-limit window, or are welded to labels the UI would then misstate. Leave them and their comments exactly as they are.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_recent_section_honours_its_setting() {
    let db = seeded_db_with_ended_sessions(10);
    let mut s = Settings::default();
    s.recent_limit = 2;
    let m = build_popover_model(&db, &s, now());
    assert_eq!(m.recent.len(), 2);
}

#[test]
fn a_project_is_active_within_its_configured_window() {
    let db = seeded_db_with_project_last_used_days_ago(10);
    let mut s = Settings::default();

    s.active_within_days = 7;
    assert_eq!(group_of(&build_main_window(&db, &s, now())), ProjectGroup::Recent);

    s.active_within_days = 14;
    assert_eq!(group_of(&build_main_window(&db, &s, now())), ProjectGroup::Active);
}

#[test]
fn one_setting_drives_both_the_chart_and_the_sparkline() {
    // The defect this prevents: two constants that both happened to be 14.
    let db = seeded_db();
    let mut s = Settings::default();
    s.chart_days = 30;
    assert_eq!(build_usage(&db, &s, now()).daily.len(), 30);
    assert_eq!(build_project_detail(&db, &s, 1, now()).sparkline.len(), 30);
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p perch-core honours_its_setting`
Expected: FAIL to compile — these builders do not take `&Settings` yet.

- [ ] **Step 3: Thread `&Settings` through**

`build_project_detail` already takes `&Settings`; the others gain the parameter. Delete `CHART_DAYS`, `SPARK_DAYS`, `TOP_N`, `RECENT_LIMIT`, `ACTIVE_WINDOW_MS`, and replace the inline `30 * DAY_MS` in `top_projects`.

`recent_sessions(limit: 0)` once returned *every* session, because the `out.len() == limit` break never fired. `recent_limit` is bounded 1–20 so zero cannot arrive from settings, but leave the explicit early return in place — the bound and the guard defend different things.

- [ ] **Step 4: Run the tests and the suite**

Run: `cargo test --workspace`
Expected: PASS. Existing tests that call these builders need the new argument; pass `&Settings::default()` so their assertions hold unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/perch-core/src/ui/model.rs crates/perch-core/src/ui/main_window.rs crates/perch-core/src/ui/usage.rs
git commit -m "feat(ui): take five frozen constants from the settings file"
```

---

### Task 7: The new display behaviour in Rust

**Files:**
- Modify: `crates/perch-core/src/ui/model.rs` (staleness, section visibility, density, row detail)
- Modify: `crates/perch-core/src/ui/usage.rs` (burn-rate modes, `show_cost`)

**Interfaces:**
- Consumes: `Settings`, `BurnRate`, `RowDensity`.
- Produces: `PopoverModel.staleness: Option<Staleness>`, section visibility flags on the model, and `UsageModel.burn_rate: Option<String>` honouring the chosen mode.

Four of the six new capabilities are behaviour rather than storage, and all four are decided here so a shell only draws the result.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn data_is_not_stale_before_the_threshold() {
    let s = Settings::default(); // dim_when_stale, 5 minutes
    let m = build_popover_model_at(&db, &s, now - 2 * MIN, now);
    assert!(m.staleness.is_none());
}

#[test]
fn stale_data_says_how_old_it_is_in_finished_words() {
    let s = Settings::default();
    let m = build_popover_model_at(&db, &s, now - 9 * MIN, now);
    let stale = m.staleness.expect("nine minutes is past a five minute threshold");
    assert_eq!(stale.label, "Last updated 9m ago");
}

#[test]
fn staleness_is_never_reported_when_dimming_is_off() {
    let mut s = Settings::default();
    s.dim_when_stale = false;
    let m = build_popover_model_at(&db, &s, now - 99 * MIN, now);
    assert!(m.staleness.is_none(), "the setting is off; there is nothing to draw");
}

#[test]
fn burn_rate_off_produces_no_line_at_all() {
    let mut s = Settings::default();
    s.burn_rate = BurnRate::Off;
    assert!(build_usage(&db, &s, now()).burn_rate.is_none());
}

#[test]
fn each_burn_rate_mode_names_its_own_unit() {
    let cases = [
        (BurnRate::TokensPerHour, "per hour"),
        (BurnRate::CostPerHour,   "/hr"),
        (BurnRate::CostPerDay,    "/day"),
    ];
    for (mode, expected) in cases {
        let mut s = Settings::default();
        s.burn_rate = mode;
        let line = build_usage(&busy_db(), &s, now()).burn_rate.expect("enough data");
        assert!(line.contains(expected), "{mode:?} produced {line:?}");
    }
}

#[test]
fn a_thin_window_still_suppresses_every_mode() {
    // MIN_PROJECTABLE_MS is a statistical floor, not a preference: no display
    // mode may talk a projection out of a window too short to support one.
    for mode in [BurnRate::TokensPerHour, BurnRate::CostPerHour, BurnRate::CostPerDay] {
        let mut s = Settings::default();
        s.burn_rate = mode;
        assert!(build_usage(&thin_db(), &s, now()).burn_rate.is_none());
    }
}

#[test]
fn hiding_cost_hides_it_everywhere_it_would_appear() {
    let mut s = Settings::default();
    s.show_cost = false;
    let m = build_usage(&busy_db(), &s, now());
    assert!(m.hero.iter().all(|h| !h.value.contains('$')));
    assert!(m.burn_rate.as_deref().unwrap_or("").matches('$').count() == 0);
}
```

The last two matter most. `a_thin_window_still_suppresses_every_mode` protects the honesty rule that already governs burn rate — a display preference must not become a way to coax a projection out of insufficient data. `hiding_cost_hides_it_everywhere` is the test that catches the obvious partial implementation.

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p perch-core staleness burn_rate`
Expected: FAIL to compile — `Staleness` does not exist.

- [ ] **Step 3: Implement**

```rust
/// What the shell needs to dim honestly: a flag it can act on and a finished
/// sentence saying how old the data is. Composed here, like every other
/// string, so no shell decides what "9m" means.
pub struct Staleness {
    pub label: String,
}
```

`build_popover_model` gains the timestamp of the last successful read. Staleness is `None` when `!dim_when_stale`, and otherwise `Some` once the gap exceeds `stale_after_minutes`. Reuse `human_elapsed` for the duration; it caps at hours deliberately.

Section visibility and density become fields on `PopoverModel`, so the shell does not read settings itself. `BurnRate::ProjectedWindow` reports the projected total for the current five-hour window and stays subject to `MIN_PROJECTABLE_MS`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p perch-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/perch-core/src/ui/model.rs crates/perch-core/src/ui/usage.rs
git commit -m "feat(ui): decide staleness, burn-rate units, and cost visibility in Rust"
```

---

### Task 8: Prices and terminals

**Files:**
- Modify: `crates/perch-core/src/pricing.rs`
- Create: `crates/perch-core/src/terminals.rs`

**Interfaces:**
- Produces: `all_prices(&Db) -> Vec<ModelPriceRow>`, `set_price(&Db, &str, ModelPrice)`, `remove_price(&Db, &str)`, `reset_prices_to_defaults(&Db)`, and `detected_terminals() -> Vec<TerminalChoice>`.

The `prices` table already exists and already survives re-seeding — `a_user_edited_price_survives_reseeding` proves it. This adds the read/write API that table never had a UI for.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn prices_come_back_with_their_defaults_marked() {
    let db = seeded_db();
    let rows = all_prices(&db);
    assert!(rows.iter().all(|r| r.is_default), "an untouched table is all defaults");
}

#[test]
fn an_edited_price_is_no_longer_marked_default() {
    let db = seeded_db();
    set_price(&db, "claude-opus-5", ModelPrice { input_per_mtok: 99.0, ..existing() }).unwrap();
    let row = find(&all_prices(&db), "claude-opus-5");
    assert!(!row.is_default, "the reset affordance appears only once a value drifted");
    assert_eq!(row.price.input_per_mtok, 99.0);
}

#[test]
fn reset_restores_the_built_in_table_and_drops_added_models() {
    let db = seeded_db();
    set_price(&db, "claude-opus-5", ModelPrice { input_per_mtok: 99.0, ..existing() }).unwrap();
    set_price(&db, "my-local-model", ModelPrice::flat(1.0)).unwrap();

    reset_prices_to_defaults(&db).unwrap();

    assert_eq!(find(&all_prices(&db), "claude-opus-5").price, builtin("claude-opus-5"));
    assert!(all_prices(&db).iter().all(|r| r.model != "my-local-model"),
            "reset means the built-in table, not the built-in table plus leftovers");
}

#[test]
fn a_removed_model_still_counts_its_tokens_and_costs_nothing() {
    // The documented rule from spec §8, which must survive an editable table.
    let db = seeded_db_with_turns_for("claude-opus-5");
    remove_price(&db, "claude-opus-5").unwrap();
    let stats = day_stats(&db, now());
    assert!(stats.tokens > 0, "tokens are truth");
    assert_eq!(stats.cost, 0.0, "dollars are an estimate, and there is none");
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p perch-core prices`
Expected: FAIL to compile — `all_prices` does not exist.

- [ ] **Step 3: Implement the price API**

`ModelPriceRow { model, price, is_default }`, where `is_default` compares against the `DEFAULTS` table so the shell can show reset only where something drifted. `reset_prices_to_defaults` deletes every row and re-seeds, which is why the test asserts an added model is gone: a reset that left user-added rows behind would not be a reset.

- [ ] **Step 4: Write the terminal detection test and implementation**

```rust
#[test]
fn the_configured_terminal_is_always_offered_even_if_undetected() {
    // A hand-edited config naming a terminal we cannot see must not render a
    // picker with nothing selected -- the defect deferred from the last
    // milestone's review.
    let choices = terminal_choices(&["Terminal"], "Ghostty");
    assert!(choices.iter().any(|c| c.id == "Ghostty"));
    assert!(choices.iter().any(|c| c.id == "Terminal"));
}
```

`detected_terminals` checks a known list of bundle identifiers — Terminal, iTerm2, Warp, Ghostty, Alacritty, Kitty, WezTerm — for presence, using a **read-only** existence check. `terminal_choices(detected, configured)` unions the two so the current value always has a row.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p perch-core prices terminals`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/perch-core/src/pricing.rs crates/perch-core/src/terminals.rs crates/perch-core/src/lib.rs
git commit -m "feat(core): read and write model prices, and find installed terminals"
```

---

### Task 9: The FFI surface

**Files:**
- Modify: `crates/perch-ffi/src/lib.rs`

**Interfaces:**
- Produces: `Perch::settings_schema(query: Option<String>) -> Vec<SettingsPane>`, `Perch::set_setting(key, value) -> SettingsResult`, `Perch::reset_pane(pane) -> SettingsResult`, `Perch::prices()`, `Perch::set_price`, `Perch::reset_prices`, `Perch::terminals()`.

Mirror every record with a **destructuring** `From` impl, as this file already does throughout — the house pattern that makes a new field a compile error rather than a silent omission.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn setting_a_value_returns_the_schema_that_results_from_it() {
    let p = perch_in_temp();
    let out = p.set_setting(SettingKey::PollSeconds, SettingValue::Int(30)).unwrap();
    assert_eq!(stepper_value(&out.panes, SettingKey::PollSeconds), 30,
               "the caller must not have to re-read to see its own write");
}

#[test]
fn an_out_of_range_write_is_clamped_and_reported_not_rejected() {
    let p = perch_in_temp();
    let out = p.set_setting(SettingKey::PollSeconds, SettingValue::Int(9999)).unwrap();
    assert_eq!(stepper_value(&out.panes, SettingKey::PollSeconds), 60);
    assert!(out.notes.iter().any(|n| n.contains("clamped")),
            "a silently clamped value is a lie about what was stored");
}

#[test]
fn a_failed_save_surfaces_as_an_error_not_an_empty_result() {
    // The FFI seam swallowing failures is a defect this project has shipped
    // twice. A write that cannot land must not look like one that did.
    let p = perch_with_unwritable_config();
    assert!(p.set_setting(SettingKey::LaunchAtLogin, SettingValue::Bool(true)).is_err());
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p perch-ffi set_setting`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`SettingsResult { panes: Vec<SettingsPane>, notes: Vec<String> }` — every mutating call returns the resulting schema **and** whatever `validated` reported, so a shell never re-reads to see its own write and never has to discover a clamp on its own.

`settings_schema(query)` applies `filter_panes` in Rust when a query is present. The shell sends keystrokes and draws what comes back; it does no matching.

- [ ] **Step 4: Run the tests, regenerate, build**

```bash
cargo test --workspace
./scripts/build-xcframework.sh
cd apps/macos/Perch && swift build -c release
```

- [ ] **Step 5: Commit** (Rust and Swift sources only — verify with `git status --porcelain` that nothing under `Sources/PerchFFI/` or `Frameworks/` is staged)

```bash
git add crates/perch-ffi/src/lib.rs
git commit -m "feat(ffi): expose the settings schema, its edits, and its resets"
```

---

### Task 10: The window shell — sidebar, search, pane routing

**Files:**
- Modify: `apps/macos/Perch/Sources/Perch/Settings/SettingsWindow.swift` (rewritten)

**Interfaces:**
- Consumes: `Perch.settingsSchema(query:)`, `PaneId`, `IconId`.
- Produces: `SettingsWindowController` hosting a `NavigationSplitView`; a `pane(for:)` routing point Tasks 11 and 12 fill in.

The SwiftPM package has **no test target**, so Swift behaviour is verified by hand. Each Swift task ends with an explicit manual check, and those checks are the deliverable's proof.

- [ ] **Step 1: Replace the `TabView` with a `NavigationSplitView`**

**The visual target is macOS System Settings**, which is also what the app we studied matches. Concretely, and these are acceptance criteria rather than suggestions:

- The sidebar row is a **coloured rounded-square tile** containing a white glyph, then the pane title — not a bare monochrome symbol. Each pane gets its own accent colour, so the sidebar is scannable by colour before it is readable by text.
- The window's title bar shows the **selected pane's name**, centred.
- The detail side is a `Form` of **titled groups**: a bold section heading, then a card of rows with hairline separators between them.
- Every row is **label, then a grey explanatory sentence beneath it**, with the control right-aligned on the label's line. The sentence is `row.help`, composed in Rust. This is the single thing that most separates a comprehensive settings window from a blank one — a column of bare toggles reads as unfinished no matter how many there are.
- A destructive `Action` sits alone at the bottom right of its pane, not inline among the settings.

Sidebar lists the panes from the schema — never a hardcoded Swift list, or the sidebar and the schema drift. Each row maps `IconId` to a symbol **and a colour**; both are drawing concerns, so both live here and no Rust change is needed for either:

```swift
/// The single place a semantic icon becomes a macOS glyph. Rust names the
/// concept; only this function knows what SF Symbols calls it.
private func tile(_ icon: IconId) -> (symbol: String, color: Color) {
    switch icon {
    case .general:       return ("gearshape", .gray)
    case .menuBar:       return ("menubar.rectangle", .blue)
    case .popover:       return ("rectangle.on.rectangle", .teal)
    case .projects:      return ("folder", .orange)
    case .usage:         return ("chart.bar", .green)
    case .prices:        return ("dollarsign.circle", .mint)
    case .notifications: return ("bell", .red)
    case .diagnostics:   return ("stethoscope", .pink)
    case .advanced:      return ("slider.horizontal.3", .purple)
    }
}
```

- [ ] **Step 2: Add the search field above the sidebar**

The field's text goes to `settingsSchema(query:)` and the returned panes replace the sidebar contents. **Swift performs no matching.** When a query yields no panes, show an explicit empty state naming the query — never an empty sidebar, which is indistinguishable from a broken window.

- [ ] **Step 3: Persist the selected pane and sidebar width**

`@AppStorage`, which is per-viewer UI state rather than a setting — it does not belong in `config.toml`, which is the user's own hand-editable file.

- [ ] **Step 4: Preserve what the previous window got right**

Carry these across verbatim; each exists because of a defect found in review:
- `actionError` rendered **unconditionally**, not inside a branch reachable only while the model is nil, and **cleared at the top of `load()`**.
- `pendingSave` task-chain serialization of writes.
- Every control's `get` derived from the model, never from local state — no optimistic mutation.
- The window counts as open when miniaturized (`410e273`), or closing Settings drops the app to `.accessory`.

- [ ] **Step 5: Verify by hand**

Build and run. Confirm: every pane appears in the sidebar; typing "terminal" narrows to one pane and one row; clearing the field restores everything; typing nonsense shows the empty state; the selected pane survives closing and reopening the window; minimizing the main window and closing Settings keeps the Dock icon.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Perch/Sources/Perch/Settings/SettingsWindow.swift
git commit -m "feat(settings): a sidebar window, driven by the schema"
```

---

### Task 11: The schema renderer

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/Settings/SchemaPane.swift`

**Interfaces:**
- Consumes: `SettingsPane`, `SettingGroup`, `SettingRow`, `Control`.
- Produces: `SchemaPane`, a SwiftUI view rendering any schema-driven pane.

One `View` per `Control` case, inside a `Form` with `.formStyle(.grouped)` so the result looks like System Settings without hand-built chrome.

- [ ] **Step 1: Render each control case**

```swift
@ViewBuilder
private func control(for row: SettingRow) -> some View {
    switch row.control {
    case let .toggle(on):
        Toggle(row.label, isOn: binding(row, on))
    case let .stepper(value, min, max, step, valueLabel):
        // valueLabel is finished text from Rust. Never build one here: this
        // is the exact site that shipped "Check every 1 seconds" twice.
        Stepper(value: binding(row, value), in: min...max, step: step) {
            Text(valueLabel)
        }
    case let .choice(selected, options):
        Picker(row.label, selection: binding(row, selected)) {
            ForEach(options, id: \.id) { Text($0.label).tag($0.id) }
        }
    case let .info(valueLabel):
        LabeledContent(row.label) { Text(valueLabel).foregroundStyle(.secondary) }
    // ... text, folder, action
    }
}
```

- [ ] **Step 2: Apply enablement, indentation and help**

`.disabled(!row.enabled)` — **disabled, never hidden**, so the option stays legible and the user learns why it is off. `row.indent` adds leading padding. `row.help` renders beneath the control in `.caption` / `.secondary`.

- [ ] **Step 3: Add the per-pane reset**

A "Reset to defaults" button in the pane footer, shown **only when some row in the pane reports `is_default == false`**. Calls `Perch.resetPane(_:)` and adopts the returned schema.

- [ ] **Step 4: Route every edit through one path**

Each binding's setter calls `Perch.setSetting(key:value:)`, adopts the returned panes, and renders the returned notes. The `get` reads the model. A control must never mutate local state optimistically — the defect fixed once already this milestone.

- [ ] **Step 5: Verify by hand**

Turn notifications off and confirm its three sub-rows grey out but stay visible and indented. Change poll interval to 1 and confirm the caption reads "Check every 1 second". Type 9999 into a stepper's field and confirm it clamps to 60 **and says so**. Change one value and confirm Reset appears; use it and confirm it disappears.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Perch/Sources/Perch/Settings/SchemaPane.swift
git commit -m "feat(settings): render any schema pane with native controls"
```

---

### Task 12: The three bespoke panes

**Files:**
- Create: `apps/macos/Perch/Sources/Perch/Settings/PricesPane.swift`
- Create: `apps/macos/Perch/Sources/Perch/Settings/AdvancedPane.swift`
- Modify: `apps/macos/Perch/Sources/Perch/Settings/DiagnosticsView.swift`

**Interfaces:**
- Consumes: `Perch.prices()`, `Perch.setPrice`, `Perch.resetPrices`, `Perch.diagnostics()`, index statistics.

- [ ] **Step 1: The prices table**

A `Table` of model id and four rate columns, each editable. Rates are **per million tokens** — say so in a header, since a user entering a per-token rate would silently inflate every cost by a millionfold. Add and remove a model; "Reset to built-in prices" with a confirmation, since it discards user-added models.

Two `Info` rows carry the facts that must not be lost: dollars are an estimate, and a model with no price contributes its tokens but no cost.

- [ ] **Step 2: Diagnostics becomes a pane**

Same content, rendered inside the split view rather than a tab. It stays **visible in the sidebar**, not gated behind an advanced toggle: it exists for a user already having trouble, and a remedy should not require an incantation.

- [ ] **Step 3: Advanced, with About as its last group**

Read-only rows — config path, index path and size, sessions and turns indexed, last successful index — each with Reveal where a path is shown. Then "Reindex now", and "Reset all settings" marked destructive and confirmed. The About group carries version, build, the MIT licence, and links.

- [ ] **Step 4: Handle the two failure paths the spec names**

A rate that cannot be parsed leaves the stored value untouched and reports against
that row — cost display must never quietly become a number the user did not enter.
A failed reset, per-pane or global, reports and changes nothing: a partial reset is
worse than none, so apply it as one operation or not at all.

- [ ] **Step 5: Verify by hand**

Edit a rate and confirm the cost shown in the Usage view changes accordingly. Add a model, reset, and confirm it is gone. Confirm Diagnostics still renders after changing the Claude Code folder — the case left open from the last milestone.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Perch/Sources/Perch/Settings/
git commit -m "feat(settings): the prices, diagnostics, and advanced panes"
```

---

### Task 13: Consume the new settings, then close the milestone

**Files:**
- Modify: `apps/macos/Perch/Sources/Perch/StatusItemController.swift`
- Modify: `apps/macos/Perch/Sources/Perch/Cards/*.swift`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/BACKLOG.md`, `README.md`

- [ ] **Step 1: Menu-bar icon variants and stale dimming**

Replace the hardcoded `NSImage(systemSymbolName: "bird", ...)` with a mapping from the `MenuBarIcon` the model carries, rendered as a template image. When `staleness` is present, dim the status item and use its `label` as the accessibility description — the label is finished text from Rust.

- [ ] **Step 2: Popover section visibility, density, and row detail**

The popover renders the sections the model says to render, at the density it reports. Swift reads no settings directly; every one of these arrives on the model.

- [ ] **Step 3: Add a CI guard for the rule that keeps breaking**

Three defects this milestone came from Swift composing a caption from a number, and **no guard covers it**. Add one that fails on string interpolation of a numeric expression inside `Text(...)` in hand-written Swift.

Give it a positive-control self-test in **both** directions, as every guard here does: a fixture that interpolates a number into `Text` **is** caught, and one that interpolates two already-finished strings **is not** — `UsageView.swift:122` legitimately joins two Rust strings for VoiceOver and must keep passing. Write the fixtures under `/tmp`, never in the repository, and use `grep -nHE` so the filename is present even for single-file input.

- [ ] **Step 4: Full verification**

```bash
cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace --all-targets
./scripts/build-xcframework.sh && cd apps/macos/Perch && swift build -c release
```

Then extract every CI guard body to `/tmp` and run it against the real tree.

- [ ] **Step 5: Update the docs**

`README.md` gains the settings window; `docs/BACKLOG.md` loses what this milestone delivered and gains what it deferred — hooks, a settings CLI, and `preferred_terminal`'s remaining string-comparison in `Launcher.swift`.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Perch/Sources/Perch/ .github/workflows/ci.yml README.md docs/BACKLOG.md
git commit -m "feat(app): draw the icon, density, and staleness the settings choose"
```

---

## Test helpers these tasks assume

The tests above call helpers that do not exist yet. Write them once, in a shared
`#[cfg(test)]` module, the first time a task needs them — do not re-invent one per
file, which is how two spellings of the same fixture appear:

| Helper | Returns |
|---|---|
| `seeded_db()` | an in-memory `Db` with the schema applied and default prices seeded |
| `seeded_db_with_ended_sessions(n)` | `seeded_db()` plus `n` ended sessions, newest first |
| `seeded_db_with_project_last_used_days_ago(n)` | one project whose last activity is `n` days old |
| `busy_db()` / `thin_db()` | enough turns in the current window to project a burn rate, and too few |
| `find_row(&[SettingsPane], SettingKey) -> &SettingRow` | panics if the key has no row — the coverage test guarantees one |
| `all_rows(&[SettingsPane])` | every row across every pane, flattened |
| `count_rows(&[SettingsPane]) -> usize` | how many rows survived a filter |
| `stepper_value(&[SettingsPane], SettingKey) -> i64` | panics unless that row is a `Stepper` |
| `builtin(model) -> ModelPrice` | the entry from `pricing::DEFAULTS`, for comparing against a reset |
| `ModelPrice::flat(rate)` | all four rates equal — a fixture, not production code |
| `perch_in_temp()` / `perch_with_unwritable_config()` | a `Perch` over a temp dir, and one whose config path cannot be written |

`SchemaContext::empty()` is production code, not a helper: it is the context with no
detected terminals and no resolved paths, which is what a unit test wants and what a
first run before detection legitimately has.

## Manual verification checklist

The SwiftPM package has no test target, so these are the milestone's acceptance criteria and a human must run them.

- [ ] Every pane opens; the sidebar shows nine
- [ ] Search narrows to one row; clearing restores; nonsense shows an empty state
- [ ] A dependent row greys out but stays visible when its master is off
- [ ] A stepper at 1 reads "1 second" / "1 minute", never "1 seconds"
- [ ] An out-of-range hand edit to `config.toml` clamps **and** reports in the window
- [ ] Per-pane reset appears only after a value drifts, and restores only its pane
- [ ] A model price edit changes the cost shown in Usage
- [ ] Reset prices removes a user-added model
- [ ] Each menu-bar icon variant renders, and dims when data goes stale
- [ ] Turning a popover section off removes it; compact density visibly tightens rows
- [ ] Each burn-rate mode shows its own unit; Off shows no line; a thin window shows none in any mode
- [ ] Hiding cost removes every dollar figure, including from the burn-rate line
- [ ] A `claude_config_dir` pointing at a regular file still opens the app and says what is wrong
- [ ] Editing `config.toml` by hand updates the window live
