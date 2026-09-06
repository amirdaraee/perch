//! Perch's own settings. Every value here replaces something that used to be a
//! constant, and every one of them is validated in Rust so a hand-edited file
//! behaves the same way whichever shell is reading it.
//!
//! On disk the file is four TOML tables — `[general]`, `[menu_bar]`,
//! `[sessions]`, `[notifications]` — plus a top-level `version`. In Rust it is
//! one flat [`Settings`] struct: the section tables are an artifact of the
//! wire format, not something callers should have to know about, so the
//! (de)serialization is written by hand against a private nested
//! [`SettingsFile`] rather than derived directly on the flat struct.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod store;

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
    pub(crate) fn as_wire_str(self) -> &'static str {
        match self {
            MenuBarDisplay::Icon => "icon",
            MenuBarDisplay::Count => "count",
            MenuBarDisplay::CountAndWaiting => "count-and-waiting",
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
        Ok(match raw.as_str() {
            "icon" => MenuBarDisplay::Icon,
            "count-and-waiting" => MenuBarDisplay::CountAndWaiting,
            // "count" and anything unrecognised both land on the default.
            _ => MenuBarDisplay::Count,
        })
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
    /// 1..=60.
    pub poll_seconds: u32,
    pub waiting_enabled: bool,
    /// 1..=240.
    pub waiting_after_minutes: u32,
    pub include_background: bool,
    /// "Terminal" | "iTerm2" | ...
    pub preferred_terminal: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            launch_at_login: false,
            claude_config_dir: String::new(),
            menu_bar_display: MenuBarDisplay::Count,
            poll_seconds: 5,
            waiting_enabled: false,
            waiting_after_minutes: 10,
            include_background: false,
            preferred_terminal: "Terminal".to_string(),
        }
    }
}

impl Settings {
    /// Clamps every out-of-range value to its bound and reports what it
    /// changed, so a hand-edited file is never silently ignored *or* silently
    /// obeyed: an out-of-range value still runs (clamped), and the user is
    /// told it happened.
    pub fn validated(self) -> (Settings, Vec<String>) {
        let mut settings = self;
        let mut notes = Vec::new();

        let clamped_poll = settings.poll_seconds.clamp(1, 60);
        if clamped_poll != settings.poll_seconds {
            notes.push(format!(
                "poll_seconds was {}; clamped to {clamped_poll}",
                settings.poll_seconds
            ));
            settings.poll_seconds = clamped_poll;
        }

        let clamped_wait = settings.waiting_after_minutes.clamp(1, 240);
        if clamped_wait != settings.waiting_after_minutes {
            notes.push(format!(
                "waiting_after_minutes was {}; clamped to {clamped_wait}",
                settings.waiting_after_minutes
            ));
            settings.waiting_after_minutes = clamped_wait;
        }

        (settings, notes)
    }
}

// --- Wire format: four sectioned tables, each defaulted independently so a
// file missing a whole section (or missing individual keys within one) still
// parses. `Settings`'s own Serialize/Deserialize are written against this
// private struct rather than derived, since the public struct is flat and
// the file on disk is not.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct GeneralSection {
    launch_at_login: bool,
    claude_config_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct MenuBarSection {
    display: MenuBarDisplay,
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
struct NotificationsSection {
    waiting_enabled: bool,
    waiting_after_minutes: u32,
}

impl Default for NotificationsSection {
    fn default() -> Self {
        let d = Settings::default();
        NotificationsSection {
            waiting_enabled: d.waiting_enabled,
            waiting_after_minutes: d.waiting_after_minutes,
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
            },
            sessions: SessionsSection {
                poll_seconds: s.poll_seconds,
                include_background: s.include_background,
                preferred_terminal: s.preferred_terminal,
            },
            notifications: NotificationsSection {
                waiting_enabled: s.waiting_enabled,
                waiting_after_minutes: s.waiting_after_minutes,
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
            poll_seconds: f.sessions.poll_seconds,
            waiting_enabled: f.notifications.waiting_enabled,
            waiting_after_minutes: f.notifications.waiting_after_minutes,
            include_background: f.sessions.include_background,
            preferred_terminal: f.sessions.preferred_terminal,
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
