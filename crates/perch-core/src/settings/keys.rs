//! Every setting addressed as data rather than as a field.
//!
//! [`Settings`] is a flat struct of twenty-seven differently-typed fields, and
//! almost nothing outside this module wants to know that: the schema, search,
//! per-pane reset and the FFI all want to say "this setting, that value".
//! [`SettingKey`] is that address, and [`Settings::get`]/[`Settings::set`]
//! are the only two places that translate between an address and a field.
//!
//! Both are single exhaustive `match`es with no catch-all arm, and that is
//! the point of the module: a twenty-eighth setting cannot be added to
//! `Settings` without the compiler demanding a key for it here, and once it
//! has a key every consumer that walks [`SettingKey::ALL`] picks it up for
//! free. The alternative — a hand-maintained list per consumer — is how a
//! setting ends up unsearchable, or missed by a reset, with nothing failing.

use super::{BurnRate, MenuBarDisplay, MenuBarIcon, RowDensity, Settings};

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
    ResumeBypassPermissions,
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
    /// Every key, once, in the order the fields are declared in [`Settings`].
    /// Consumers walk this instead of keeping their own list, which is what
    /// makes "every key has a row", "every key is searchable" and "every key
    /// resets" testable as one assertion each rather than twenty-seven.
    pub const ALL: [SettingKey; 27] = [
        SettingKey::LaunchAtLogin,
        SettingKey::ClaudeConfigDir,
        SettingKey::MenuBarDisplay,
        SettingKey::MenuBarIcon,
        SettingKey::DimWhenStale,
        SettingKey::StaleAfterMinutes,
        SettingKey::PollSeconds,
        SettingKey::PreferredTerminal,
        SettingKey::ResumeBypassPermissions,
        SettingKey::ShowWaiting,
        SettingKey::ShowWorking,
        SettingKey::ShowRecent,
        SettingKey::RecentLimit,
        SettingKey::RowDensity,
        SettingKey::ShowRowFolder,
        SettingKey::ShowRowUsage,
        SettingKey::ActiveWithinDays,
        SettingKey::ShowArchived,
        SettingKey::ChartDays,
        SettingKey::TopProjectsCount,
        SettingKey::TopProjectsDays,
        SettingKey::BurnRate,
        SettingKey::ShowCost,
        SettingKey::WaitingEnabled,
        SettingKey::WaitingAfterMinutes,
        SettingKey::IncludeBackground,
        SettingKey::Sound,
    ];
}

/// The four shapes a setting's value can take at the schema boundary. A
/// [`SettingValue::Choice`] carries the wire string its enum already defines,
/// so no third spelling of these values exists: the file on disk, the schema
/// and the shell all say `"count-and-waiting"`.
///
/// `Int` is `i64` rather than the `u32` every counted setting actually
/// stores, because this shape has to survive a trip through an FFI whose
/// integers are signed. A negative or oversized `Int` is therefore possible
/// to *send* and is refused by `set`; it is not clamped into range, which is
/// a different question with a different answer (see below).
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Text(String),
    Choice(String),
}

impl SettingValue {
    /// What this value is, for an error message that says what arrived.
    fn shape_name(&self) -> &'static str {
        match self {
            SettingValue::Bool(_) => "a boolean",
            SettingValue::Int(_) => "a number",
            SettingValue::Text(_) => "text",
            SettingValue::Choice(_) => "a choice",
        }
    }
}

/// Why a [`Settings::set`] could not be honoured. Always a mismatch between
/// the value's shape and the key's — never an out-of-range number, which is
/// a legal write that [`Settings::validated`] clamps and reports on the way
/// to disk.
///
/// This is a report about a caller's mistake, not user-facing copy: anything
/// may arrive across the FFI, and a shell that sends a boolean where a count
/// belongs must be told so rather than crash the app it is drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct SetError {
    pub key_label: String,
    pub message: String,
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key_label, self.message)
    }
}

impl std::error::Error for SetError {}

fn wrong_shape(key: SettingKey, wanted: &str, got: &SettingValue) -> SetError {
    SetError {
        key_label: format!("{key:?}"),
        message: format!("expected {wanted}, got {}", got.shape_name()),
    }
}

fn want_bool(key: SettingKey, value: SettingValue) -> Result<bool, SetError> {
    match value {
        SettingValue::Bool(b) => Ok(b),
        other => Err(wrong_shape(key, "a boolean", &other)),
    }
}

/// A counted setting: a whole number that fits the `u32` the field stores.
/// Range checking stops there. Whether 9999 seconds is a sensible poll
/// interval is [`Settings::validated`]'s question, asked once on the way to
/// disk so exactly one place decides bounds; asking it here as well would
/// give a value two chances to be silently altered, in two different places,
/// with only one of them able to tell the user it happened.
fn want_count(key: SettingKey, value: SettingValue) -> Result<u32, SetError> {
    match value {
        SettingValue::Int(n) => u32::try_from(n).map_err(|_| SetError {
            key_label: format!("{key:?}"),
            message: format!("expected a whole number of at most {}, got {n}", u32::MAX),
        }),
        other => Err(wrong_shape(key, "a number", &other)),
    }
}

fn want_text(key: SettingKey, value: SettingValue) -> Result<String, SetError> {
    match value {
        SettingValue::Text(t) => Ok(t),
        other => Err(wrong_shape(key, "text", &other)),
    }
}

/// A choice, checked against the wire values its enum publishes. An
/// unrecognized string is refused rather than degraded to the default the
/// way the file reader degrades it: a typo in a hand-edited file must cost
/// the user one setting instead of the whole file, but a shell picking from
/// a list this same schema handed it has made a mistake, and swallowing that
/// would leave a picker that appears to select something and stores another.
fn want_choice(key: SettingKey, value: SettingValue, known: &[&str]) -> Result<String, SetError> {
    match value {
        SettingValue::Choice(c) if known.contains(&c.as_str()) => Ok(c),
        SettingValue::Choice(c) => Err(SetError {
            key_label: format!("{key:?}"),
            message: format!("{c:?} is not one of {}", known.join(", ")),
        }),
        other => Err(wrong_shape(key, "a choice", &other)),
    }
}

impl Settings {
    /// This setting's current value, in the shape the schema and the shell
    /// speak. Exhaustive over [`SettingKey`] with no catch-all arm.
    pub fn get(&self, key: SettingKey) -> SettingValue {
        match key {
            SettingKey::LaunchAtLogin => SettingValue::Bool(self.launch_at_login),
            SettingKey::ClaudeConfigDir => SettingValue::Text(self.claude_config_dir.clone()),
            SettingKey::MenuBarDisplay => {
                SettingValue::Choice(self.menu_bar_display.as_wire_str().to_string())
            }
            SettingKey::MenuBarIcon => {
                SettingValue::Choice(self.menu_bar_icon.as_wire_str().to_string())
            }
            SettingKey::DimWhenStale => SettingValue::Bool(self.dim_when_stale),
            SettingKey::StaleAfterMinutes => SettingValue::Int(i64::from(self.stale_after_minutes)),
            SettingKey::PollSeconds => SettingValue::Int(i64::from(self.poll_seconds)),
            SettingKey::PreferredTerminal => SettingValue::Text(self.preferred_terminal.clone()),
            SettingKey::ResumeBypassPermissions => {
                SettingValue::Bool(self.resume_bypass_permissions)
            }
            SettingKey::ShowWaiting => SettingValue::Bool(self.show_waiting),
            SettingKey::ShowWorking => SettingValue::Bool(self.show_working),
            SettingKey::ShowRecent => SettingValue::Bool(self.show_recent),
            SettingKey::RecentLimit => SettingValue::Int(i64::from(self.recent_limit)),
            SettingKey::RowDensity => {
                SettingValue::Choice(self.row_density.as_wire_str().to_string())
            }
            SettingKey::ShowRowFolder => SettingValue::Bool(self.show_row_folder),
            SettingKey::ShowRowUsage => SettingValue::Bool(self.show_row_usage),
            SettingKey::ActiveWithinDays => SettingValue::Int(i64::from(self.active_within_days)),
            SettingKey::ShowArchived => SettingValue::Bool(self.show_archived),
            // A number, not a `Choice`: `chart_days` is a `u32` whose offered
            // values are `CHART_DAY_CHOICES`, and the schema presents it as a
            // picker over those. Spelling 30 as `"30"` here would be the
            // third spelling `SettingValue` exists to prevent.
            SettingKey::ChartDays => SettingValue::Int(i64::from(self.chart_days)),
            SettingKey::TopProjectsCount => SettingValue::Int(i64::from(self.top_projects_count)),
            SettingKey::TopProjectsDays => SettingValue::Int(i64::from(self.top_projects_days)),
            SettingKey::BurnRate => SettingValue::Choice(self.burn_rate.as_wire_str().to_string()),
            SettingKey::ShowCost => SettingValue::Bool(self.show_cost),
            SettingKey::WaitingEnabled => SettingValue::Bool(self.waiting_enabled),
            SettingKey::WaitingAfterMinutes => {
                SettingValue::Int(i64::from(self.waiting_after_minutes))
            }
            SettingKey::IncludeBackground => SettingValue::Bool(self.include_background),
            SettingKey::Sound => SettingValue::Bool(self.sound),
        }
    }

    /// Store `value` under `key`, or say why its shape does not fit.
    ///
    /// Exhaustive over [`SettingKey`] with no catch-all arm, and it **does
    /// not clamp**: bounds live in [`Settings::validated`], which runs once
    /// on the way to disk and reports every change it makes. A value stored
    /// here is exactly the value that arrived.
    pub fn set(&mut self, key: SettingKey, value: SettingValue) -> Result<(), SetError> {
        match key {
            SettingKey::LaunchAtLogin => self.launch_at_login = want_bool(key, value)?,
            SettingKey::ClaudeConfigDir => self.claude_config_dir = want_text(key, value)?,
            SettingKey::MenuBarDisplay => {
                let raw = want_choice(key, value, &MenuBarDisplay::KNOWN_WIRE_VALUES)?;
                self.menu_bar_display = MenuBarDisplay::from_wire_str(&raw);
            }
            SettingKey::MenuBarIcon => {
                let raw = want_choice(key, value, &MenuBarIcon::KNOWN_WIRE_VALUES)?;
                self.menu_bar_icon = MenuBarIcon::from_wire_str(&raw);
            }
            SettingKey::DimWhenStale => self.dim_when_stale = want_bool(key, value)?,
            SettingKey::StaleAfterMinutes => self.stale_after_minutes = want_count(key, value)?,
            SettingKey::PollSeconds => self.poll_seconds = want_count(key, value)?,
            SettingKey::PreferredTerminal => self.preferred_terminal = want_text(key, value)?,
            SettingKey::ResumeBypassPermissions => {
                self.resume_bypass_permissions = want_bool(key, value)?
            }
            SettingKey::ShowWaiting => self.show_waiting = want_bool(key, value)?,
            SettingKey::ShowWorking => self.show_working = want_bool(key, value)?,
            SettingKey::ShowRecent => self.show_recent = want_bool(key, value)?,
            SettingKey::RecentLimit => self.recent_limit = want_count(key, value)?,
            SettingKey::RowDensity => {
                let raw = want_choice(key, value, &RowDensity::KNOWN_WIRE_VALUES)?;
                self.row_density = RowDensity::from_wire_str(&raw);
            }
            SettingKey::ShowRowFolder => self.show_row_folder = want_bool(key, value)?,
            SettingKey::ShowRowUsage => self.show_row_usage = want_bool(key, value)?,
            SettingKey::ActiveWithinDays => self.active_within_days = want_count(key, value)?,
            SettingKey::ShowArchived => self.show_archived = want_bool(key, value)?,
            SettingKey::ChartDays => self.chart_days = want_count(key, value)?,
            SettingKey::TopProjectsCount => self.top_projects_count = want_count(key, value)?,
            SettingKey::TopProjectsDays => self.top_projects_days = want_count(key, value)?,
            SettingKey::BurnRate => {
                let raw = want_choice(key, value, &BurnRate::KNOWN_WIRE_VALUES)?;
                self.burn_rate = BurnRate::from_wire_str(&raw);
            }
            SettingKey::ShowCost => self.show_cost = want_bool(key, value)?,
            SettingKey::WaitingEnabled => self.waiting_enabled = want_bool(key, value)?,
            SettingKey::WaitingAfterMinutes => self.waiting_after_minutes = want_count(key, value)?,
            SettingKey::IncludeBackground => self.include_background = want_bool(key, value)?,
            SettingKey::Sound => self.sound = want_bool(key, value)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{BurnRate, MenuBarDisplay, MenuBarIcon, RowDensity, Settings};

    /// Every wire-value domain a [`SettingValue::Choice`] can be drawn from.
    /// `flip` needs to know which enum a choice string belongs to before it
    /// can name a *different* member of the same enum, and this is the only
    /// place that mapping is needed.
    const CHOICE_DOMAINS: [&[&str]; 4] = [
        &MenuBarDisplay::KNOWN_WIRE_VALUES,
        &MenuBarIcon::KNOWN_WIRE_VALUES,
        &RowDensity::KNOWN_WIRE_VALUES,
        &BurnRate::KNOWN_WIRE_VALUES,
    ];

    /// A different value of the same shape. For a choice that means a
    /// different *valid* member of the same enum, never an invented string:
    /// the round-trip test below would otherwise be asserting that `set`
    /// stores garbage.
    fn flip(value: &SettingValue) -> SettingValue {
        match value {
            SettingValue::Bool(b) => SettingValue::Bool(!b),
            SettingValue::Int(n) => SettingValue::Int(n + 1),
            SettingValue::Text(t) => SettingValue::Text(format!("{t}x")),
            SettingValue::Choice(c) => {
                let domain = CHOICE_DOMAINS
                    .into_iter()
                    .find(|d| d.contains(&c.as_str()))
                    .unwrap_or_else(|| panic!("{c:?} is not a wire value of any settings enum"));
                let other = domain
                    .iter()
                    .find(|o| **o != c.as_str())
                    .unwrap_or_else(|| panic!("{domain:?} has no second value to flip to"));
                SettingValue::Choice((*other).to_string())
            }
        }
    }

    #[test]
    fn flip_offers_a_different_valid_value_for_every_choice_enum() {
        // The round-trip test's `assert_ne!` is only meaningful if this holds
        // for every enum, including the four- and five-variant ones. A
        // one-variant enum would make it impossible; none exists, and this is
        // what keeps that true.
        for domain in CHOICE_DOMAINS {
            assert!(
                domain.len() >= 2,
                "{domain:?} cannot offer a different value"
            );
            for value in domain {
                let flipped = flip(&SettingValue::Choice((*value).to_string()));
                let SettingValue::Choice(got) = &flipped else {
                    panic!("flip changed the shape of a choice");
                };
                assert_ne!(got, value, "flip returned the value it was given");
                assert!(
                    domain.contains(&got.as_str()),
                    "{got:?} is not a value of {domain:?}"
                );
            }
        }
    }

    #[test]
    fn every_key_round_trips_through_get_and_set() {
        for key in SettingKey::ALL {
            let mut s = Settings::default();
            let original = s.get(key);
            let changed = flip(&original);
            s.set(key, changed.clone())
                .expect("set accepts its own shape");
            assert_eq!(
                s.get(key),
                changed,
                "{key:?} did not store what it was given"
            );
            assert_ne!(s.get(key), original, "{key:?} ignored the write");
        }
    }

    #[test]
    fn setting_a_key_to_the_wrong_shape_is_an_error_not_a_panic() {
        let mut s = Settings::default();
        let err = s.set(SettingKey::PollSeconds, SettingValue::Bool(true));
        assert!(err.is_err(), "a bool is not a poll interval");
    }

    #[test]
    fn all_lists_every_key_exactly_once() {
        let mut seen: Vec<SettingKey> = SettingKey::ALL.to_vec();
        let before = seen.len();
        seen.sort_by_key(|k| format!("{k:?}"));
        seen.dedup();
        assert_eq!(seen.len(), before, "ALL contains a duplicate");
        assert_eq!(
            before, 27,
            "ALL is missing a key, or gained one without this count"
        );
    }

    #[test]
    fn a_choice_carries_the_wire_string_the_enum_already_defines() {
        // Not "Count", not "count_and_waiting": the same spelling the file on
        // disk uses, so a value never acquires a second form in transit.
        let s = Settings::default();
        assert_eq!(
            s.get(SettingKey::MenuBarDisplay),
            SettingValue::Choice("count".to_string())
        );
        assert_eq!(
            s.get(SettingKey::BurnRate),
            SettingValue::Choice("cost-per-hour".to_string())
        );
    }

    #[test]
    fn set_stores_an_out_of_range_number_without_clamping_it() {
        // The bound is `validated`'s to apply, once, on the way to disk --
        // and to report. A `set` that quietly clamped as well would give the
        // same value two chances to change without the user hearing about it.
        let mut s = Settings::default();
        s.set(SettingKey::PollSeconds, SettingValue::Int(9999))
            .expect("a number is a number");
        assert_eq!(s.poll_seconds, 9999, "set stores what it was given");

        let (validated, notes) = s.validated();
        assert_eq!(validated.poll_seconds, 60, "the bound still applies");
        assert!(
            notes.iter().any(|n| n.contains("clamped")),
            "and it is still reported: {notes:?}"
        );
    }

    #[test]
    fn a_number_that_cannot_be_stored_is_an_error_not_a_wrap() {
        let mut s = Settings::default();
        assert!(s
            .set(SettingKey::RecentLimit, SettingValue::Int(-1))
            .is_err());
        assert!(s
            .set(SettingKey::RecentLimit, SettingValue::Int(i64::MAX))
            .is_err());
        assert_eq!(
            s.recent_limit,
            Settings::default().recent_limit,
            "a refused write changes nothing"
        );
    }

    #[test]
    fn a_choice_outside_its_own_enum_is_refused() {
        // The file reader degrades an unknown word to the default so one typo
        // does not cost every other setting. A caller picking from a list this
        // schema handed it has no such excuse, and a silent degrade there
        // would show a picker selecting one thing and storing another.
        let mut s = Settings::default();
        let err = s
            .set(
                SettingKey::MenuBarIcon,
                SettingValue::Choice("owl".to_string()),
            )
            .expect_err("owl is not a glyph Perch draws");
        assert!(err.message.contains("owl"), "got {err}");
        assert_eq!(s.menu_bar_icon, Settings::default().menu_bar_icon);
    }

    #[test]
    fn every_key_refuses_a_shape_that_is_not_its_own() {
        // Anything at all can arrive across the FFI. Nothing may panic.
        let shapes = [
            SettingValue::Bool(true),
            SettingValue::Int(1),
            SettingValue::Text("x".to_string()),
            SettingValue::Choice("x".to_string()),
        ];
        for key in SettingKey::ALL {
            let mut s = Settings::default();
            let mine = s.get(key);
            for shape in &shapes {
                if std::mem::discriminant(shape) == std::mem::discriminant(&mine) {
                    continue;
                }
                assert!(
                    s.set(key, shape.clone()).is_err(),
                    "{key:?} accepted {shape:?}"
                );
            }
            assert_eq!(s.get(key), mine, "{key:?} changed on a refused write");
        }
    }
}
