//! Perch's own settings. Every value here replaces something that used to be a
//! constant, and every one of them is validated in Rust so a hand-edited file
//! behaves the same way whichever shell is reading it.
//!
//! On disk the file is seven TOML tables — `[general]`, `[menu_bar]`,
//! `[sessions]`, `[popover]`, `[projects]`, `[usage]`, `[notifications]` —
//! plus a top-level `version`. A section is a wire-format grouping, not a
//! settings-window pane: `poll_seconds` and `preferred_terminal` live under
//! `[sessions]` while being shown under General, and moving a key between
//! sections costs a migration, so they stay where they were first written.
//! In Rust it is
//! one flat [`Settings`] struct: the section tables are an artifact of the
//! wire format, not something callers should have to know about, so the
//! (de)serialization is written by hand against a private nested
//! [`SettingsFile`] rather than derived directly on the flat struct.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::Path;

pub mod keys;
pub mod store;

pub use keys::*;

/// Bumped whenever the on-disk shape changes in a way a migration needs to
/// know about. Not itself a field of [`Settings`] — later tasks write it
/// alongside the settings tables.
pub const SETTINGS_VERSION: i64 = 1;

/// What the menu-bar extra shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuBarDisplay {
    /// Just the Perch icon, no count.
    Icon,
    /// The icon plus a count of active sessions.
    #[default]
    Count,
    /// The count, plus a distinct glyph when something is waiting on you.
    CountAndWaiting,
}

impl MenuBarDisplay {
    /// Every wire value this binary recognizes for `menu_bar.display`. Kept
    /// as one array so `Deserialize` below (accepting these) and
    /// `store::load` (detecting anything *outside* this set, to report the
    /// silent-degrade case `Deserialize` cannot report on its own) share a
    /// single source of truth instead of two hand-maintained lists.
    pub const KNOWN_WIRE_VALUES: [&'static str; 3] = ["icon", "count", "count-and-waiting"];

    /// The exact string this variant round-trips to on the wire.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            MenuBarDisplay::Icon => "icon",
            MenuBarDisplay::Count => "count",
            MenuBarDisplay::CountAndWaiting => "count-and-waiting",
        }
    }

    fn from_wire_str(s: &str) -> Self {
        match s {
            "icon" => MenuBarDisplay::Icon,
            "count-and-waiting" => MenuBarDisplay::CountAndWaiting,
            // "count" and anything unrecognised both land on the default.
            _ => MenuBarDisplay::Count,
        }
    }
}

/// Deserialized by hand, not derived: an unrecognized wire value (an older or
/// newer Perch's variant name the user has never heard of, or a plain typo in
/// a hand-edited file) must degrade to the default display rather than
/// failing deserialization of the whole settings file. A derived
/// `Deserialize` — even with `#[serde(other)]` — would still need a variant
/// to land on, and a rejected value would bubble up as a hard error that
/// takes every other setting in the file down with it.
///
/// This silent degrade is exactly why `settings::store::load` separately
/// inspects the raw document for `menu_bar.display` and reports it via a
/// note when the value is not one of [`MenuBarDisplay::KNOWN_WIRE_VALUES`] —
/// this impl has no path back to a caller-visible note, only this default.
impl<'de> Deserialize<'de> for MenuBarDisplay {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(MenuBarDisplay::from_wire_str(&raw))
    }
}

/// Which glyph the status item draws. Semantic rather than an SF Symbol
/// name: the symbol is the one genuinely macOS-specific fact here, so the
/// shell maps this to its own glyph and a Linux shell picks a different one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuBarIcon {
    #[default]
    Bird,
    Binoculars,
    Dot,
    Bars,
}

impl MenuBarIcon {
    /// Every wire value this binary recognizes for `menu_bar.icon`; see
    /// [`MenuBarDisplay::KNOWN_WIRE_VALUES`] for why this is one array.
    pub const KNOWN_WIRE_VALUES: [&'static str; 4] = ["bird", "binoculars", "dot", "bars"];

    /// The exact string this variant round-trips to on the wire.
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
            // "bird" and anything unrecognised both land on the default.
            _ => MenuBarIcon::Bird,
        }
    }
}

/// Hand-written for the same reason [`MenuBarDisplay`]'s is, and it may
/// never return `Err`: one unrecognized glyph name must cost the user that
/// one setting, not every other setting in the file.
impl<'de> Deserialize<'de> for MenuBarIcon {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(MenuBarIcon::from_wire_str(&raw))
    }
}

/// How tightly the popover packs a session row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RowDensity {
    #[default]
    Comfortable,
    Compact,
}

impl RowDensity {
    /// Every wire value this binary recognizes for `popover.row_density`.
    pub const KNOWN_WIRE_VALUES: [&'static str; 2] = ["comfortable", "compact"];

    /// The exact string this variant round-trips to on the wire.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            RowDensity::Comfortable => "comfortable",
            RowDensity::Compact => "compact",
        }
    }

    fn from_wire_str(s: &str) -> Self {
        match s {
            "compact" => RowDensity::Compact,
            // "comfortable" and anything unrecognised both land on the default.
            _ => RowDensity::Comfortable,
        }
    }
}

/// Hand-written for the same reason [`MenuBarDisplay`]'s is, and it may
/// never return `Err`.
impl<'de> Deserialize<'de> for RowDensity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(RowDensity::from_wire_str(&raw))
    }
}

/// Which unit the usage view's burn-rate line speaks in, or `Off` for no
/// line at all. Only the unit is a preference: whether there is enough data
/// to project honestly is not, and stays decided in `ui::usage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BurnRate {
    Off,
    TokensPerHour,
    #[default]
    CostPerHour,
    CostPerDay,
    ProjectedWindow,
}

impl BurnRate {
    /// Every wire value this binary recognizes for `usage.burn_rate`.
    pub const KNOWN_WIRE_VALUES: [&'static str; 5] = [
        "off",
        "tokens-per-hour",
        "cost-per-hour",
        "cost-per-day",
        "projected-window",
    ];

    /// The exact string this variant round-trips to on the wire.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            BurnRate::Off => "off",
            BurnRate::TokensPerHour => "tokens-per-hour",
            BurnRate::CostPerHour => "cost-per-hour",
            BurnRate::CostPerDay => "cost-per-day",
            BurnRate::ProjectedWindow => "projected-window",
        }
    }

    fn from_wire_str(s: &str) -> Self {
        match s {
            "off" => BurnRate::Off,
            "tokens-per-hour" => BurnRate::TokensPerHour,
            "cost-per-day" => BurnRate::CostPerDay,
            "projected-window" => BurnRate::ProjectedWindow,
            // "cost-per-hour" and anything unrecognised both land on the default.
            _ => BurnRate::CostPerHour,
        }
    }
}

/// Hand-written for the same reason [`MenuBarDisplay`]'s is, and it may
/// never return `Err`.
impl<'de> Deserialize<'de> for BurnRate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(BurnRate::from_wire_str(&raw))
    }
}

/// Perch's settings, flattened for callers even though the file on disk is
/// sectioned. Every field is meaningful on its own; see [`Settings::default`]
/// for the values a fresh install starts with and [`Settings::validated`] for
/// the bounds a hand-edited file is held to.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub launch_at_login: bool,
    /// "" means auto-detect Claude's config directory.
    pub claude_config_dir: String,
    pub menu_bar_display: MenuBarDisplay,
    pub menu_bar_icon: MenuBarIcon,
    /// Dim the status item once the index has not been refreshed for
    /// `stale_after_minutes`.
    pub dim_when_stale: bool,
    /// 1..=120.
    pub stale_after_minutes: u32,
    /// 1..=60.
    pub poll_seconds: u32,
    /// "Terminal" | "iTerm2" | ...
    pub preferred_terminal: String,
    pub show_waiting: bool,
    pub show_working: bool,
    pub show_recent: bool,
    /// 1..=20.
    pub recent_limit: u32,
    pub row_density: RowDensity,
    pub show_row_folder: bool,
    pub show_row_usage: bool,
    /// 1..=90.
    pub active_within_days: u32,
    pub show_archived: bool,
    /// One of 7, 14, 30, 90 — a choice, not a range. Drives both the usage
    /// chart and the project sparkline, which is why it is one key: two
    /// constants that both happened to be 14 could drift apart silently.
    pub chart_days: u32,
    /// 3..=20.
    pub top_projects_count: u32,
    /// 7..=180.
    pub top_projects_days: u32,
    pub burn_rate: BurnRate,
    pub show_cost: bool,
    pub waiting_enabled: bool,
    /// 1..=240.
    pub waiting_after_minutes: u32,
    pub include_background: bool,
    /// Play the platform's default alert sound with a delivered notification.
    pub sound: bool,
}

/// Every default is chosen to preserve the behaviour Perch already had when
/// these values were constants, so upgrading changes nothing a user sees.
/// The comment beside each new one names the constant it replaces.
impl Default for Settings {
    fn default() -> Self {
        Settings {
            launch_at_login: false,
            claude_config_dir: String::new(),
            menu_bar_display: MenuBarDisplay::Count,
            // What `StatusItemController` hardcodes today.
            menu_bar_icon: MenuBarIcon::Bird,
            // Staleness is invisible today; on is the honest default.
            dim_when_stale: true,
            // A minute of slack over the 5 s poll's worst case.
            stale_after_minutes: 5,
            poll_seconds: 5,
            preferred_terminal: "Terminal".to_string(),
            // Today's popover shows all three sections.
            show_waiting: true,
            show_working: true,
            show_recent: true,
            // Today's `ui::model::RECENT_LIMIT`.
            recent_limit: 3,
            // Today's layout.
            row_density: RowDensity::Comfortable,
            // Not shown today.
            show_row_folder: false,
            // Shown today.
            show_row_usage: true,
            // Today's `ui::main_window::ACTIVE_WINDOW_MS`.
            active_within_days: 7,
            // The archived group exists today.
            show_archived: true,
            // Today's `SPARK_DAYS` and `CHART_DAYS`, which are the same 14.
            chart_days: 14,
            // Today's `ui::usage::TOP_N`.
            top_projects_count: 8,
            // Today's inline `30 * DAY_MS` in `top_projects`.
            top_projects_days: 30,
            // Today's burn-rate line.
            burn_rate: BurnRate::CostPerHour,
            // Shown today.
            show_cost: true,
            waiting_enabled: false,
            waiting_after_minutes: 10,
            include_background: false,
            // The platform default for a delivered alert.
            sound: true,
        }
    }
}

/// Clamp one bounded value and, when that changed it, say so in the same
/// shape every other bound reports in. One helper rather than eight
/// near-identical blocks, so a new bound cannot quietly grow a different
/// wording for the same event.
fn clamp_note(name: &str, value: u32, lo: u32, hi: u32, notes: &mut Vec<String>) -> u32 {
    let clamped = value.clamp(lo, hi);
    if clamped != value {
        notes.push(format!("{name} was {value}; clamped to {clamped}"));
    }
    clamped
}

/// The only ranges `chart_days` may take. Not a min/max: a chart drawn over
/// 23 days would be a legible chart of a range nobody asked for, so an
/// unrecognized value snaps to the nearest offered choice instead of being
/// clamped into the interval.
pub const CHART_DAY_CHOICES: [u32; 4] = [7, 14, 30, 90];

/// Snap `chart_days` to the nearest [`CHART_DAY_CHOICES`] entry, reporting
/// the change exactly as a clamp does.
fn snap_chart_days(value: u32, notes: &mut Vec<String>) -> u32 {
    if CHART_DAY_CHOICES.contains(&value) {
        return value;
    }
    let snapped = CHART_DAY_CHOICES
        .into_iter()
        .min_by_key(|c| c.abs_diff(value))
        .expect("CHART_DAY_CHOICES is never empty");
    notes.push(format!(
        "chart_days was {value}, which is not one of 7, 14, 30 or 90; using {snapped}"
    ));
    snapped
}

impl Settings {
    /// Clamps every out-of-range value to its bound and reports what it
    /// changed, so a hand-edited file is never silently ignored *or* silently
    /// obeyed: an out-of-range value still runs (clamped), and the user is
    /// told it happened.
    ///
    /// `claude_config_dir` is the one field whose "out of range" can only be
    /// answered by looking at the disk, so this is the one place that does:
    /// a path that is not a directory is dropped back to "" (auto-detect)
    /// and reported, exactly like a clamp. It is dropped here, rather than
    /// refused wherever it is *used*, so that every consumer -- `Perch::new`,
    /// the running engine's settings watch, and the diagnostics pane's
    /// "where did this directory come from" verdict -- sees one and the same
    /// answer, and none of them can be brought down by a value the others
    /// have already ruled out. A setting that points somewhere wrong must
    /// degrade to auto-detection, never to an app that will not open.
    pub fn validated(self) -> (Settings, Vec<String>) {
        let mut settings = self;
        let mut notes = Vec::new();

        settings.poll_seconds =
            clamp_note("poll_seconds", settings.poll_seconds, 1, 60, &mut notes);
        settings.waiting_after_minutes = clamp_note(
            "waiting_after_minutes",
            settings.waiting_after_minutes,
            1,
            240,
            &mut notes,
        );
        settings.stale_after_minutes = clamp_note(
            "stale_after_minutes",
            settings.stale_after_minutes,
            1,
            120,
            &mut notes,
        );
        settings.recent_limit =
            clamp_note("recent_limit", settings.recent_limit, 1, 20, &mut notes);
        settings.active_within_days = clamp_note(
            "active_within_days",
            settings.active_within_days,
            1,
            90,
            &mut notes,
        );
        settings.top_projects_count = clamp_note(
            "top_projects_count",
            settings.top_projects_count,
            3,
            20,
            &mut notes,
        );
        settings.top_projects_days = clamp_note(
            "top_projects_days",
            settings.top_projects_days,
            7,
            180,
            &mut notes,
        );
        settings.chart_days = snap_chart_days(settings.chart_days, &mut notes);

        let configured_dir = settings.claude_config_dir.trim();
        if !configured_dir.is_empty() && !Path::new(configured_dir).is_dir() {
            notes.push(format!(
                "claude_config_dir was {configured_dir:?}, which is not a directory; \
                 ignoring it and auto-detecting Claude Code's directory instead"
            ));
            settings.claude_config_dir = String::new();
        }

        (settings, notes)
    }
}

// --- Wire format: seven sectioned tables, each defaulted independently so a
// file missing a whole section (or missing individual keys within one) still
// parses. `Settings`'s own Serialize/Deserialize are written against this
// private struct rather than derived, since the public struct is flat and
// the file on disk is not.
//
// Each section's `Default` reads from `Settings::default()` rather than
// restating a literal, so every default in this module is stated exactly
// once and a section cannot drift from the struct it mirrors.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct GeneralSection {
    launch_at_login: bool,
    claude_config_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct MenuBarSection {
    display: MenuBarDisplay,
    icon: MenuBarIcon,
    dim_when_stale: bool,
    stale_after_minutes: u32,
}

impl Default for MenuBarSection {
    fn default() -> Self {
        let d = Settings::default();
        MenuBarSection {
            display: d.menu_bar_display,
            icon: d.menu_bar_icon,
            dim_when_stale: d.dim_when_stale,
            stale_after_minutes: d.stale_after_minutes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct SessionsSection {
    poll_seconds: u32,
    include_background: bool,
    preferred_terminal: String,
}

impl Default for SessionsSection {
    fn default() -> Self {
        let d = Settings::default();
        SessionsSection {
            poll_seconds: d.poll_seconds,
            include_background: d.include_background,
            preferred_terminal: d.preferred_terminal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct PopoverSection {
    show_waiting: bool,
    show_working: bool,
    show_recent: bool,
    recent_limit: u32,
    row_density: RowDensity,
    show_row_folder: bool,
    show_row_usage: bool,
}

impl Default for PopoverSection {
    fn default() -> Self {
        let d = Settings::default();
        PopoverSection {
            show_waiting: d.show_waiting,
            show_working: d.show_working,
            show_recent: d.show_recent,
            recent_limit: d.recent_limit,
            row_density: d.row_density,
            show_row_folder: d.show_row_folder,
            show_row_usage: d.show_row_usage,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ProjectsSection {
    active_within_days: u32,
    show_archived: bool,
    chart_days: u32,
}

impl Default for ProjectsSection {
    fn default() -> Self {
        let d = Settings::default();
        ProjectsSection {
            active_within_days: d.active_within_days,
            show_archived: d.show_archived,
            chart_days: d.chart_days,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct UsageSection {
    top_projects_count: u32,
    top_projects_days: u32,
    burn_rate: BurnRate,
    show_cost: bool,
}

impl Default for UsageSection {
    fn default() -> Self {
        let d = Settings::default();
        UsageSection {
            top_projects_count: d.top_projects_count,
            top_projects_days: d.top_projects_days,
            burn_rate: d.burn_rate,
            show_cost: d.show_cost,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct NotificationsSection {
    waiting_enabled: bool,
    waiting_after_minutes: u32,
    sound: bool,
}

impl Default for NotificationsSection {
    fn default() -> Self {
        let d = Settings::default();
        NotificationsSection {
            waiting_enabled: d.waiting_enabled,
            waiting_after_minutes: d.waiting_after_minutes,
            sound: d.sound,
        }
    }
}

/// The `version` key is read and ignored here: this task only needs
/// `Settings` to tolerate its presence in a file, not to act on it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SettingsFile {
    version: i64,
    general: GeneralSection,
    menu_bar: MenuBarSection,
    sessions: SessionsSection,
    popover: PopoverSection,
    projects: ProjectsSection,
    usage: UsageSection,
    notifications: NotificationsSection,
}

impl From<Settings> for SettingsFile {
    fn from(s: Settings) -> Self {
        SettingsFile {
            version: SETTINGS_VERSION,
            general: GeneralSection {
                launch_at_login: s.launch_at_login,
                claude_config_dir: s.claude_config_dir,
            },
            menu_bar: MenuBarSection {
                display: s.menu_bar_display,
                icon: s.menu_bar_icon,
                dim_when_stale: s.dim_when_stale,
                stale_after_minutes: s.stale_after_minutes,
            },
            sessions: SessionsSection {
                poll_seconds: s.poll_seconds,
                include_background: s.include_background,
                preferred_terminal: s.preferred_terminal,
            },
            popover: PopoverSection {
                show_waiting: s.show_waiting,
                show_working: s.show_working,
                show_recent: s.show_recent,
                recent_limit: s.recent_limit,
                row_density: s.row_density,
                show_row_folder: s.show_row_folder,
                show_row_usage: s.show_row_usage,
            },
            projects: ProjectsSection {
                active_within_days: s.active_within_days,
                show_archived: s.show_archived,
                chart_days: s.chart_days,
            },
            usage: UsageSection {
                top_projects_count: s.top_projects_count,
                top_projects_days: s.top_projects_days,
                burn_rate: s.burn_rate,
                show_cost: s.show_cost,
            },
            notifications: NotificationsSection {
                waiting_enabled: s.waiting_enabled,
                waiting_after_minutes: s.waiting_after_minutes,
                sound: s.sound,
            },
        }
    }
}

impl From<SettingsFile> for Settings {
    fn from(f: SettingsFile) -> Self {
        Settings {
            launch_at_login: f.general.launch_at_login,
            claude_config_dir: f.general.claude_config_dir,
            menu_bar_display: f.menu_bar.display,
            menu_bar_icon: f.menu_bar.icon,
            dim_when_stale: f.menu_bar.dim_when_stale,
            stale_after_minutes: f.menu_bar.stale_after_minutes,
            poll_seconds: f.sessions.poll_seconds,
            preferred_terminal: f.sessions.preferred_terminal,
            show_waiting: f.popover.show_waiting,
            show_working: f.popover.show_working,
            show_recent: f.popover.show_recent,
            recent_limit: f.popover.recent_limit,
            row_density: f.popover.row_density,
            show_row_folder: f.popover.show_row_folder,
            show_row_usage: f.popover.show_row_usage,
            active_within_days: f.projects.active_within_days,
            show_archived: f.projects.show_archived,
            chart_days: f.projects.chart_days,
            top_projects_count: f.usage.top_projects_count,
            top_projects_days: f.usage.top_projects_days,
            burn_rate: f.usage.burn_rate,
            show_cost: f.usage.show_cost,
            waiting_enabled: f.notifications.waiting_enabled,
            waiting_after_minutes: f.notifications.waiting_after_minutes,
            include_background: f.sessions.include_background,
            sound: f.notifications.sound,
        }
    }
}

impl Serialize for Settings {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SettingsFile::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SettingsFile::deserialize(deserializer).map(Settings::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_ones() {
        let s = Settings::default();
        assert!(!s.launch_at_login);
        assert_eq!(s.claude_config_dir, "");
        assert_eq!(s.menu_bar_display, MenuBarDisplay::Count);
        assert_eq!(s.poll_seconds, 5);
        assert!(!s.waiting_enabled, "notifications are off until asked for");
        assert_eq!(s.waiting_after_minutes, 10);
        assert!(!s.include_background, "a bg session is not waiting on you");
        assert_eq!(s.preferred_terminal, "Terminal");

        // The eighteen added this milestone, each chosen so that upgrading
        // changes nothing a user already sees.
        assert_eq!(s.menu_bar_icon, MenuBarIcon::Bird);
        assert!(s.dim_when_stale);
        assert_eq!(s.stale_after_minutes, 5);
        assert!(s.show_waiting);
        assert!(s.show_working);
        assert!(s.show_recent);
        assert_eq!(s.recent_limit, 3, "today's RECENT_LIMIT");
        assert_eq!(s.row_density, RowDensity::Comfortable);
        assert!(!s.show_row_folder, "not shown today");
        assert!(s.show_row_usage, "shown today");
        assert_eq!(s.active_within_days, 7, "today's ACTIVE_WINDOW_MS");
        assert!(s.show_archived);
        assert_eq!(s.chart_days, 14, "today's SPARK_DAYS and CHART_DAYS");
        assert_eq!(s.top_projects_count, 8, "today's TOP_N");
        assert_eq!(s.top_projects_days, 30, "today's inline 30 * DAY_MS");
        assert_eq!(s.burn_rate, BurnRate::CostPerHour);
        assert!(s.show_cost, "shown today");
        assert!(s.sound, "the platform default for a delivered alert");
    }

    #[test]
    fn out_of_range_values_are_clamped_and_reported() {
        let s = Settings {
            poll_seconds: 0,
            waiting_after_minutes: 9999,
            ..Default::default()
        };
        let (s, notes) = s.validated();
        assert_eq!(
            s.poll_seconds, 1,
            "clamped to the floor, not reset to the default"
        );
        assert_eq!(s.waiting_after_minutes, 240);
        assert_eq!(
            notes.len(),
            2,
            "each change is reported so the edit is not silently obeyed"
        );
        assert!(notes.iter().any(|n| n.contains("poll_seconds")));
        assert!(notes.iter().any(|n| n.contains("waiting_after_minutes")));
    }

    #[test]
    fn every_bounded_field_clamps_and_reports() {
        let s = Settings {
            poll_seconds: 0,
            waiting_after_minutes: 9999,
            stale_after_minutes: 0,
            recent_limit: 999,
            active_within_days: 0,
            top_projects_count: 1,
            top_projects_days: 1,
            ..Default::default()
        };
        let (s, notes) = s.validated();
        assert_eq!(s.poll_seconds, 1);
        assert_eq!(s.waiting_after_minutes, 240);
        assert_eq!(s.stale_after_minutes, 1);
        assert_eq!(s.recent_limit, 20);
        assert_eq!(s.active_within_days, 1);
        assert_eq!(s.top_projects_count, 3);
        assert_eq!(s.top_projects_days, 7);
        assert_eq!(
            notes.len(),
            7,
            "every bound reports, so no edit is silently obeyed: {notes:?}"
        );
    }

    #[test]
    fn chart_days_snaps_to_the_nearest_offered_range_rather_than_clamping() {
        for (given, want) in [(1_u32, 7_u32), (13, 14), (20, 14), (23, 30), (999, 90)] {
            let (s, notes) = Settings {
                chart_days: given,
                ..Default::default()
            }
            .validated();
            assert_eq!(s.chart_days, want, "{given} should snap to {want}");
            assert_eq!(notes.len(), 1, "and say so: {notes:?}");
            assert!(notes[0].contains("chart_days"), "actionable: {}", notes[0]);
        }
    }

    #[test]
    fn an_offered_chart_range_is_left_alone() {
        for given in CHART_DAY_CHOICES {
            let (s, notes) = Settings {
                chart_days: given,
                ..Default::default()
            }
            .validated();
            assert_eq!(s.chart_days, given);
            assert!(notes.is_empty(), "{given} is offered: {notes:?}");
        }
    }

    #[test]
    fn the_new_enums_fall_back_rather_than_failing_the_whole_file() {
        let s: Settings = toml_edit::de::from_str(
            "[menu_bar]\nicon = \"nonsense\"\n\n[popover]\nrow_density = \"nonsense\"\n\n\
             [usage]\nburn_rate = \"nonsense\"\n",
        )
        .expect("one bad enum value must not cost the user every other setting");
        assert_eq!(s.menu_bar_icon, MenuBarIcon::Bird);
        assert_eq!(s.row_density, RowDensity::Comfortable);
        assert_eq!(s.burn_rate, BurnRate::CostPerHour);
    }

    #[test]
    fn the_new_enums_round_trip_through_their_wire_names() {
        for wire in MenuBarIcon::KNOWN_WIRE_VALUES {
            let s: Settings =
                toml_edit::de::from_str(&format!("[menu_bar]\nicon = \"{wire}\"\n")).unwrap();
            assert_eq!(s.menu_bar_icon.as_wire_str(), wire);
        }
        for wire in RowDensity::KNOWN_WIRE_VALUES {
            let s: Settings =
                toml_edit::de::from_str(&format!("[popover]\nrow_density = \"{wire}\"\n")).unwrap();
            assert_eq!(s.row_density.as_wire_str(), wire);
        }
        for wire in BurnRate::KNOWN_WIRE_VALUES {
            let s: Settings =
                toml_edit::de::from_str(&format!("[usage]\nburn_rate = \"{wire}\"\n")).unwrap();
            assert_eq!(s.burn_rate.as_wire_str(), wire);
        }
    }

    #[test]
    fn in_range_values_are_left_alone_and_report_nothing() {
        let s = Settings {
            poll_seconds: 30,
            waiting_after_minutes: 60,
            ..Default::default()
        };
        let (s, notes) = s.validated();
        assert_eq!(s.poll_seconds, 30);
        assert_eq!(s.waiting_after_minutes, 60);
        assert!(notes.is_empty());
    }

    #[test]
    fn a_claude_config_dir_that_is_not_a_directory_is_dropped_and_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let not_a_dir = tmp.path().join("regular-file");
        std::fs::write(&not_a_dir, b"i am a file, not a directory").unwrap();

        let s = Settings {
            claude_config_dir: not_a_dir.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let (s, notes) = s.validated();

        assert_eq!(
            s.claude_config_dir, "",
            "an unusable override must degrade to auto-detection, never be handed on \
             to a caller that would then refuse to start"
        );
        assert_eq!(notes.len(), 1, "and it is never dropped silently");
        assert!(
            notes[0].contains("claude_config_dir"),
            "actionable: {}",
            notes[0]
        );
    }

    #[test]
    fn a_claude_config_dir_that_is_a_real_directory_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Settings {
            claude_config_dir: tmp.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let (s, notes) = s.validated();

        assert_eq!(s.claude_config_dir, tmp.path().to_string_lossy());
        assert!(notes.is_empty());
    }

    #[test]
    fn a_missing_key_takes_its_default_rather_than_failing() {
        // A file written by an older Perch will not have every key.
        let partial = r#"
            version = 1
            [sessions]
            poll_seconds = 9
        "#;
        let s: Settings = toml_edit::de::from_str(partial).expect("partial file must parse");
        assert_eq!(s.poll_seconds, 9, "the key that is present is honoured");
        assert_eq!(
            s.waiting_after_minutes, 10,
            "the ones that are absent take defaults"
        );
    }

    #[test]
    fn menu_bar_display_round_trips_through_its_wire_names() {
        for (v, wire) in [
            (MenuBarDisplay::Icon, "icon"),
            (MenuBarDisplay::Count, "count"),
            (MenuBarDisplay::CountAndWaiting, "count-and-waiting"),
        ] {
            let doc = format!("[menu_bar]\ndisplay = \"{wire}\"\n");
            let s: Settings = toml_edit::de::from_str(&doc).unwrap();
            assert_eq!(s.menu_bar_display, v, "{wire} must map to {v:?}");
        }
    }

    #[test]
    fn an_unknown_display_value_falls_back_rather_than_failing_the_whole_file() {
        let s: Settings = toml_edit::de::from_str("[menu_bar]\ndisplay = \"nonsense\"\n")
            .expect("one bad enum value must not cost the user every other setting");
        assert_eq!(s.menu_bar_display, MenuBarDisplay::Count);
    }
}
