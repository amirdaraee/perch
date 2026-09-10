//! The settings window, described in Rust: which panes exist, what each row
//! is called, what it explains about itself, which control edits it, and
//! whether it is reachable at all given everything else the user has chosen.
//!
//! A shell renders this and does nothing else. It does not decide that the
//! waiting threshold is meaningless while alerts are off ([`SettingRow::enabled`]
//! carries that answer), it does not compose "every 5 seconds" from a number
//! ([`Control::Stepper`] carries the finished caption), and it does not keep
//! its own list of panes to search or reset (both are functions over what
//! [`build_schema`] returns). Every one of those, written in a shell, is a
//! second copy of a rule that can disagree with this one — and each has cost
//! this project a defect already.
//!
//! [`build_schema`] is a pure function of a [`Settings`] and a
//! [`SchemaContext`]. It reads no database, detects no terminals and touches
//! no disk: the context arrives carrying finished facts — counts, resolved
//! labels, an already-built list of terminal choices — so the schema depends
//! on none of the modules that produce them, and every string it composes is
//! testable from a struct literal.

use super::{
    BurnRate, MenuBarDisplay, MenuBarIcon, RowDensity, SettingKey, Settings, CHART_DAY_CHOICES,
};
use crate::ui::format::plural;

/// A pane's identity, used by the shell for selection and by `reset_pane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    General,
    MenuBar,
    Popover,
    Projects,
    Usage,
    Prices,
    Notifications,
    Diagnostics,
    Advanced,
}

/// A semantic icon, never an SF Symbol name. The shell maps this to whatever
/// glyph its platform draws; the mapping is the one macOS-specific fact in
/// the settings window, so it is the one thing that stays in Swift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconId {
    General,
    MenuBar,
    Popover,
    Projects,
    Usage,
    Prices,
    Notifications,
    Diagnostics,
    Advanced,
}

/// One pane of the settings window: a sidebar entry, and what it shows.
///
/// Three panes — [`PaneId::Prices`], [`PaneId::Diagnostics`] and
/// [`PaneId::Advanced`] — carry no groups on purpose. Their content is a
/// price table, a diagnostics report and a set of buttons, none of which is a
/// list of scalar options, so each is drawn bespoke. They are still here so
/// the sidebar, search and selection know they exist rather than learning
/// about them from a second hand-maintained list in the shell.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsPane {
    pub id: PaneId,
    pub title: String,
    pub icon: IconId,
    /// A short finished phrase shown beside the pane in the sidebar when the
    /// current configuration is costing the user something — "off · 2
    /// waiting", "3 ignored", "1 unpriced". `None` when there is nothing to
    /// say, which must be the common case: a badge that is always present is
    /// decoration and stops being read.
    pub attention: Option<String>,
    /// What this pane's settings currently produce, in the user's own data.
    pub preview: Option<PanePreview>,
    pub groups: Vec<SettingGroup>,
}

/// Finished strings, like every other model in this codebase. A shell lays
/// these out; it never computes them. The Menu Bar pane's mock status item is
/// the one exception, and it is drawn from values the model already carries.
#[derive(Debug, Clone, PartialEq)]
pub struct PanePreview {
    /// The headline: "12 Active · 19 Recent · 4 Archived".
    pub summary: String,
    /// Optional supporting lines beneath it.
    pub detail: Vec<String>,
}

/// A titled set of rows. The heading is an `Option` because the type allows
/// an untitled group; `every_group_is_titled` is what makes sure none is
/// ever built, since grouping is what turns a list of controls into a form.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingGroup {
    pub heading: Option<String>,
    pub rows: Vec<SettingRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingRow {
    /// `None` for a row that shows something rather than storing it — an
    /// [`Control::Info`] row, or an [`Control::Action`] button.
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

/// One option of a [`Control::Choice`].
///
/// `id` is the wire string the setting's own enum already publishes —
/// `"count-and-waiting"`, never `"Count and waiting"` — so a picker sends
/// back something [`Settings::set`] recognizes. The one exception is
/// [`SettingKey::PreferredTerminal`], whose value is free text (a terminal
/// Perch found, not a variant Perch defines): its `id` is the terminal's
/// name and a shell stores it with [`super::SettingValue::Text`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
}

/// One option of a [`Control::IntChoice`], carrying the number it stores and
/// the caption that number reads as.
#[derive(Debug, Clone, PartialEq)]
pub struct IntOption {
    pub value: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    Toggle {
        on: bool,
    },
    /// `value_label` is finished here — "5 seconds", "1 minute" — because a
    /// caption composed in a shell is how "Check every 1 seconds" reached a
    /// shipped build of this app twice.
    Stepper {
        value: i64,
        min: i64,
        max: i64,
        step: i64,
        value_label: String,
    },
    /// A picker over strings. `selected` is one of `options`' ids.
    Choice {
        selected: String,
        options: Vec<ChoiceOption>,
    },
    /// A picker over *numbers*. `chart_days` is the only one, and it is not a
    /// [`Control::Choice`] because it is stored as an integer: spelling 30 as
    /// `"30"` here would be the third spelling of that value, which is
    /// exactly what [`super::SettingValue`] exists to prevent. A shell sends
    /// the chosen `value` back as [`super::SettingValue::Int`].
    IntChoice {
        selected: i64,
        options: Vec<IntOption>,
    },
    Text {
        value: String,
        placeholder: String,
    },
    /// A folder the user may pick. `value` is what is stored — empty means
    /// "find it automatically" — and `resolved_label` is the folder that
    /// choice actually lands on, so an empty value never renders as a blank.
    Folder {
        value: String,
        resolved_label: String,
    },
    Action {
        button_label: String,
        destructive: bool,
    },
    /// A row that reports rather than edits: the resolved Claude Code folder,
    /// the command Resume will run.
    Info {
        value_label: String,
    },
}

/// How Claude Code's folder resolved, if it has resolved at all.
///
/// The distinction that matters is [`DirStatus::Unresolved`] versus
/// [`DirStatus::NotFound`]: nothing resolved yet is a first run, and is not a
/// problem to badge, while a configured folder that could not be used is.
#[derive(Debug, Clone, PartialEq)]
pub enum DirStatus {
    /// Nothing has been resolved yet — a genuine first run before detection,
    /// and what a unit test has.
    Unresolved,
    /// Perch is reading `label`; `how` says why it picked that one
    /// ("found automatically", "set in settings").
    Reading { label: String, how: String },
    /// The configured folder could not be used, so Perch is reading nothing.
    NotFound { label: String },
}

/// The facts the schema needs but does not own.
///
/// Everything here is a *finished value*: a count already taken, a label
/// already formatted, a choice list already built. The schema queries
/// nothing, which is what keeps [`build_schema`] pure, keeps it independent
/// of the modules that detect terminals or read the index, and makes every
/// string it composes testable from a struct literal. Whoever calls it — the
/// FFI, today — converts at that boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaContext {
    pub claude_dir: DirStatus,
    /// Days since each unarchived project was last worked on, one entry per
    /// project. Kept as ages rather than as a bucketed count because the
    /// buckets move with `active_within_days`: a count taken at some other
    /// threshold would make the Projects preview disagree with the setting
    /// sitting directly above it.
    pub project_last_used_days: Vec<u32>,
    pub archived_projects: u32,
    pub indexed_sessions: u32,
    pub waiting_sessions: u32,
    pub working_sessions: u32,
    pub recent_sessions: u32,
    pub priced_models: u32,
    /// Models with recorded turns and no price row — what the cost estimate
    /// is quietly missing.
    pub unpriced_models_in_use: u32,
    /// Session records the indexer rejected.
    pub rejected_records: u32,
    /// The terminals Perch found installed, already turned into choices.
    /// Deliberately not a terminal-detection type: the schema must not
    /// depend on the module that finds them.
    pub terminal_choices: Vec<ChoiceOption>,
    /// The command Resume would run, composed by whoever knows how.
    pub resume_command: Option<String>,
    /// The status item's current title, so every icon variant can be judged
    /// against the real text beside it.
    pub menu_bar_title: Option<String>,
    /// The burn-rate line the current mode produces, or `None` when the
    /// window holds too little data to project honestly.
    pub burn_rate_line: Option<String>,
}

impl SchemaContext {
    /// The context with nothing detected and nothing resolved.
    ///
    /// This is production code, not a test helper: it is what a genuine first
    /// run has before the index has been read and the terminals looked for,
    /// and the schema must be drawable then — with no counts, no previews it
    /// cannot honestly fill, and no attention badges, since nothing being
    /// known yet is not the same as something being wrong.
    pub fn empty() -> Self {
        SchemaContext {
            claude_dir: DirStatus::Unresolved,
            project_last_used_days: Vec::new(),
            archived_projects: 0,
            indexed_sessions: 0,
            waiting_sessions: 0,
            working_sessions: 0,
            recent_sessions: 0,
            priced_models: 0,
            unpriced_models_in_use: 0,
            rejected_records: 0,
            terminal_choices: Vec::new(),
            resume_command: None,
            menu_bar_title: None,
            burn_rate_line: None,
        }
    }
}

/// Every pane, in sidebar order.
///
/// One function per pane below, so no single one grows past reading length
/// and a change to the Usage pane cannot disturb the Popover pane's copy.
pub fn build_schema(s: &Settings, cx: &SchemaContext) -> Vec<SettingsPane> {
    vec![
        general_pane(s, cx),
        menu_bar_pane(s, cx),
        popover_pane(s, cx),
        projects_pane(s, cx),
        usage_pane(s, cx),
        prices_pane(cx),
        notifications_pane(s, cx),
        diagnostics_pane(cx),
        advanced_pane(),
    ]
}

/// Restore every setting `pane_id` shows to its factory value, leaving every
/// other pane's settings exactly as they are.
///
/// The keys come from the schema rather than from a list written beside it,
/// and each one is restored by reading [`Settings::default`] through
/// [`Settings::get`] and writing it back through [`Settings::set`] — never by
/// assigning a field. That is what keeps this correct when a twenty-seventh
/// setting is added: `set` is exhaustive over [`SettingKey`], and
/// `every_setting_key_appears_in_exactly_one_pane` guarantees the new key is
/// in some pane, so the new setting resets with its pane without anyone
/// remembering to come back here.
///
/// [`PaneId::Prices`], [`PaneId::Diagnostics`] and [`PaneId::Advanced`] carry
/// no rows, so they own no keys and this is a no-op over them. Reaching for
/// "everything" in that case would turn a reset of the price table into a
/// reset of the whole app.
pub fn reset_pane(s: &mut Settings, pane_id: PaneId) {
    // The context only decides captions, counts and enablement — never which
    // rows exist — so the empty one names the same keys as any other, and
    // keeps this function as free of the index and the terminal detector as
    // `build_schema` itself.
    let keys: Vec<SettingKey> = build_schema(s, &SchemaContext::empty())
        .iter()
        .filter(|p| p.id == pane_id)
        .flat_map(|p| p.groups.iter())
        .flat_map(|g| g.rows.iter())
        .filter_map(|r| r.key)
        .collect();

    let defaults = Settings::default();
    for key in keys {
        // `defaults.get(key)` is by construction the shape `set` wants for
        // that key, so this cannot fail; ignoring it keeps `reset_pane`
        // infallible rather than inventing an error no caller could act on.
        let _ = s.set(key, defaults.get(key));
    }
}

// --- The panes ------------------------------------------------------------

fn general_pane(s: &Settings, cx: &SchemaContext) -> SettingsPane {
    let groups = vec![
        group(
            "Startup",
            vec![row(
                s,
                SettingKey::LaunchAtLogin,
                "Open at login",
                "Start Perch when you log in, so it is already watching before you open a session.",
                Control::Toggle {
                    on: s.launch_at_login,
                },
            )
            .searchable(&["startup", "login item", "launch"])],
        ),
        group(
            "Claude Code",
            vec![
                row(
                    s,
                    SettingKey::ClaudeConfigDir,
                    "Claude Code folder",
                    "Where Claude Code keeps its sessions. Leave this blank to find it \
                     automatically, and set it only if Perch cannot see your work.",
                    Control::Folder {
                        value: s.claude_config_dir.clone(),
                        resolved_label: folder_label(cx),
                    },
                )
                .searchable(&["~/.claude", "path", "directory", "location"]),
                info_row("Currently reading", reading_label(cx)),
            ],
        ),
        group(
            "Refreshing",
            vec![row(
                s,
                SettingKey::PollSeconds,
                "Check for changes",
                "How often Perch re-reads Claude Code's session files. Lower is more \
                 responsive; higher is quieter on a busy machine.",
                counted_stepper(s.poll_seconds, 1, 60, "second", "seconds"),
            )
            .searchable(&["poll", "refresh", "interval", "update"])],
        ),
        group(
            "Terminal",
            vec![
                row(
                    s,
                    SettingKey::PreferredTerminal,
                    "Open sessions in",
                    "Which terminal Resume and Open launch. Only terminals Perch can find \
                     installed are offered.",
                    Control::Choice {
                        selected: s.preferred_terminal.clone(),
                        options: cx.terminal_choices.clone(),
                    },
                )
                .searchable(&["iTerm", "Ghostty", "Warp", "shell", "resume", "open"]),
                info_row("Resume runs", resume_label(cx)),
            ],
        ),
    ];

    SettingsPane {
        id: PaneId::General,
        title: "General".to_string(),
        icon: IconId::General,
        attention: match &cx.claude_dir {
            DirStatus::NotFound { .. } => Some("not found".to_string()),
            DirStatus::Unresolved | DirStatus::Reading { .. } => None,
        },
        preview: general_preview(cx),
        groups,
    }
}

fn menu_bar_pane(s: &Settings, cx: &SchemaContext) -> SettingsPane {
    let groups = vec![
        group(
            "Icon",
            vec![
                row(
                    s,
                    SettingKey::MenuBarIcon,
                    "Icon",
                    "The glyph Perch draws in the menu bar.",
                    Control::Choice {
                        selected: s.menu_bar_icon.as_wire_str().to_string(),
                        options: menu_bar_icon_options(),
                    },
                )
                .searchable(&["glyph", "symbol", "bird", "status item"]),
                row(
                    s,
                    SettingKey::MenuBarDisplay,
                    "Show alongside",
                    "What sits next to the icon. The hourglass appears only while something \
                     is actually waiting on you.",
                    Control::Choice {
                        selected: s.menu_bar_display.as_wire_str().to_string(),
                        options: menu_bar_display_options(),
                    },
                )
                .searchable(&["hourglass", "count", "badge", "number"]),
            ],
        ),
        group(
            "Stale data",
            vec![
                row(
                    s,
                    SettingKey::DimWhenStale,
                    "Fade when out of date",
                    "Fade the menu bar item when Perch has not managed to re-read your \
                     sessions recently, so an old number never looks like a current one.",
                    Control::Toggle {
                        on: s.dim_when_stale,
                    },
                )
                .searchable(&["dim", "grey", "stale", "old"]),
                row(
                    s,
                    SettingKey::StaleAfterMinutes,
                    "Count as out of date after",
                    "How long without a successful read before what you are looking at \
                     counts as old.",
                    counted_stepper(s.stale_after_minutes, 1, 120, "minute", "minutes"),
                )
                .depends_on(s.dim_when_stale)
                .searchable(&["stale", "threshold", "age"]),
            ],
        ),
    ];

    SettingsPane {
        id: PaneId::MenuBar,
        title: "Menu Bar".to_string(),
        icon: IconId::MenuBar,
        attention: None,
        preview: cx.menu_bar_title.as_ref().map(|title| PanePreview {
            summary: title.clone(),
            detail: vec!["This is what the menu bar shows right now.".to_string()],
        }),
        groups,
    }
}

fn popover_pane(s: &Settings, cx: &SchemaContext) -> SettingsPane {
    let groups = vec![
        group(
            "Sections",
            vec![
                row(
                    s,
                    SettingKey::ShowWaiting,
                    "Waiting on you",
                    "Show sessions that are blocked waiting for you to answer.",
                    Control::Toggle { on: s.show_waiting },
                )
                .searchable(&["blocked", "prompt", "section"]),
                row(
                    s,
                    SettingKey::ShowWorking,
                    "Working",
                    "Show sessions that are running right now.",
                    Control::Toggle { on: s.show_working },
                )
                .searchable(&["running", "busy", "section"]),
                row(
                    s,
                    SettingKey::ShowRecent,
                    "Recent",
                    "Show sessions that finished recently.",
                    Control::Toggle { on: s.show_recent },
                )
                .searchable(&["finished", "done", "section"]),
                row(
                    s,
                    SettingKey::RecentLimit,
                    "Recent sessions shown",
                    "How many finished sessions to list.",
                    counted_stepper(s.recent_limit, 1, 20, "session", "sessions"),
                )
                .depends_on(s.show_recent)
                .searchable(&["how many", "limit", "count"]),
            ],
        ),
        group(
            "Rows",
            vec![
                row(
                    s,
                    SettingKey::RowDensity,
                    "Row height",
                    "How much breathing room each row gets.",
                    Control::Choice {
                        selected: s.row_density.as_wire_str().to_string(),
                        options: row_density_options(),
                    },
                )
                .searchable(&["compact", "comfortable", "spacing", "density"]),
                row(
                    s,
                    SettingKey::ShowRowFolder,
                    "Show project folder",
                    "Show each session's folder underneath its name, which helps when two \
                     projects share a name.",
                    Control::Toggle {
                        on: s.show_row_folder,
                    },
                )
                .searchable(&["path", "directory", "cwd"]),
                row(
                    s,
                    SettingKey::ShowRowUsage,
                    "Show tokens and cost",
                    "Show each session's token count and estimated cost on its row.",
                    Control::Toggle {
                        on: s.show_row_usage,
                    },
                )
                .searchable(&["tokens", "cost", "dollars", "usage"]),
            ],
        ),
    ];

    SettingsPane {
        id: PaneId::Popover,
        title: "Popover".to_string(),
        icon: IconId::Popover,
        attention: None,
        preview: Some(popover_preview(cx)),
        groups,
    }
}

fn projects_pane(s: &Settings, cx: &SchemaContext) -> SettingsPane {
    let groups = vec![
        group(
            "Grouping",
            vec![
                row(
                    s,
                    SettingKey::ActiveWithinDays,
                    "Active means touched within",
                    "How recently you must have worked on a project for it to count as \
                     Active rather than Recent.",
                    counted_stepper(s.active_within_days, 1, 90, "day", "days"),
                )
                .searchable(&["recent", "grouping", "threshold"]),
                row(
                    s,
                    SettingKey::ShowArchived,
                    "Show archived projects",
                    "Show the Archived group in the sidebar. Archiving a project only hides \
                     it; nothing is ever deleted.",
                    Control::Toggle {
                        on: s.show_archived,
                    },
                )
                .searchable(&["archive", "hidden", "sidebar"]),
            ],
        ),
        group(
            "Charts",
            vec![row(
                s,
                SettingKey::ChartDays,
                "Days of history",
                "How many days the usage chart and each project's sparkline cover. Both \
                 read this one setting, so they can never disagree about their own date \
                 range.",
                Control::IntChoice {
                    selected: i64::from(s.chart_days),
                    options: chart_day_options(),
                },
            )
            .searchable(&["chart", "sparkline", "history", "range", "window"])],
        ),
    ];

    SettingsPane {
        id: PaneId::Projects,
        title: "Projects".to_string(),
        icon: IconId::Projects,
        attention: None,
        preview: Some(projects_preview(s, cx)),
        groups,
    }
}

fn usage_pane(s: &Settings, cx: &SchemaContext) -> SettingsPane {
    let groups = vec![
        group(
            "Top projects",
            vec![
                row(
                    s,
                    SettingKey::TopProjectsCount,
                    "Projects ranked",
                    "How many projects the Top Projects list shows.",
                    counted_stepper(s.top_projects_count, 3, 20, "project", "projects"),
                )
                .searchable(&["top", "leaderboard", "how many"]),
                row(
                    s,
                    SettingKey::TopProjectsDays,
                    "Ranked over",
                    "How far back the Top Projects ranking looks.",
                    counted_stepper(s.top_projects_days, 7, 180, "day", "days"),
                )
                .searchable(&["top", "window", "range"]),
            ],
        ),
        group(
            "Burn rate",
            vec![row(
                s,
                SettingKey::BurnRate,
                "Show burn rate as",
                "How to express your current rate of spend. Perch shows nothing at all \
                 until the window holds enough data to project honestly, whichever unit \
                 you pick.",
                Control::Choice {
                    selected: s.burn_rate.as_wire_str().to_string(),
                    options: burn_rate_options(),
                },
            )
            .searchable(&["spend", "per hour", "per day", "projection", "rate"])],
        ),
        group(
            "Cost",
            vec![row(
                s,
                SettingKey::ShowCost,
                "Show cost estimates",
                "Show dollar figures. Tokens are counted from your transcripts; dollars \
                 are inferred from a price table you can edit.",
                Control::Toggle { on: s.show_cost },
            )
            .searchable(&["dollars", "money", "price", "estimate"])],
        ),
    ];

    SettingsPane {
        id: PaneId::Usage,
        title: "Usage".to_string(),
        icon: IconId::Usage,
        attention: None,
        preview: Some(usage_preview(s, cx)),
        groups,
    }
}

/// Rowless on purpose: an editable price table is not a list of scalar
/// options, so its content is drawn bespoke. It is in the schema so the
/// sidebar and search know it exists.
fn prices_pane(cx: &SchemaContext) -> SettingsPane {
    SettingsPane {
        id: PaneId::Prices,
        title: "Prices".to_string(),
        icon: IconId::Prices,
        attention: (cx.unpriced_models_in_use > 0)
            .then(|| format!("{} unpriced", cx.unpriced_models_in_use)),
        preview: Some(prices_preview(cx)),
        groups: Vec::new(),
    }
}

fn notifications_pane(s: &Settings, cx: &SchemaContext) -> SettingsPane {
    let groups = vec![group(
        "Waiting on you",
        vec![
            row(
                s,
                SettingKey::WaitingEnabled,
                "Tell me when a session is waiting",
                "Notify you when a session has been sitting blocked on your answer. macOS \
                 will ask permission the first time you turn this on.",
                Control::Toggle {
                    on: s.waiting_enabled,
                },
            )
            .searchable(&["notification", "alert", "banner", "permission"]),
            row(
                s,
                SettingKey::WaitingAfterMinutes,
                "After",
                "How long a session must sit blocked before Perch says anything.",
                counted_stepper(s.waiting_after_minutes, 1, 240, "minute", "minutes"),
            )
            .depends_on(s.waiting_enabled)
            .searchable(&["delay", "threshold", "wait"]),
            row(
                s,
                SettingKey::IncludeBackground,
                "Include background sessions",
                "Also alert for background sessions. They are usually not waiting on you, \
                 which is why they are left out by default.",
                Control::Toggle {
                    on: s.include_background,
                },
            )
            .depends_on(s.waiting_enabled)
            .searchable(&["background", "headless", "agent"]),
            row(
                s,
                SettingKey::Sound,
                "Play a sound",
                "Play the system notification sound with the alert.",
                Control::Toggle { on: s.sound },
            )
            .depends_on(s.waiting_enabled)
            .searchable(&["sound", "chime", "audio", "silent"]),
        ],
    )];

    SettingsPane {
        id: PaneId::Notifications,
        title: "Notifications".to_string(),
        icon: IconId::Notifications,
        attention: (!s.waiting_enabled && cx.waiting_sessions > 0)
            .then(|| format!("off · {} waiting", cx.waiting_sessions)),
        preview: Some(notifications_preview(cx)),
        groups,
    }
}

/// Rowless on purpose, like [`prices_pane`]: a report about what Perch could
/// and could not read is not a list of options.
fn diagnostics_pane(cx: &SchemaContext) -> SettingsPane {
    SettingsPane {
        id: PaneId::Diagnostics,
        title: "Diagnostics".to_string(),
        icon: IconId::Diagnostics,
        attention: (cx.rejected_records > 0).then(|| format!("{} ignored", cx.rejected_records)),
        preview: None,
        groups: Vec::new(),
    }
}

/// Rowless on purpose: paths, index statistics, reindex, reset and About are
/// buttons and readouts rather than stored settings.
fn advanced_pane() -> SettingsPane {
    SettingsPane {
        id: PaneId::Advanced,
        title: "Advanced".to_string(),
        icon: IconId::Advanced,
        attention: None,
        preview: None,
        groups: Vec::new(),
    }
}

// --- Previews -------------------------------------------------------------
//
// Where there is no data yet, each says so plainly rather than showing a
// zero, which is the rule every other view-model in this crate follows.

fn general_preview(cx: &SchemaContext) -> Option<PanePreview> {
    let projects = cx.project_last_used_days.len() as u32 + cx.archived_projects;
    if projects == 0 && cx.indexed_sessions == 0 {
        return None;
    }
    let counts = format!(
        "{} and {}",
        plural(i64::from(projects), "project", "projects"),
        plural(i64::from(cx.indexed_sessions), "session", "sessions")
    );
    let summary = match &cx.claude_dir {
        DirStatus::Reading { label, .. } => format!("Reading {counts} from {label}."),
        DirStatus::NotFound { .. } | DirStatus::Unresolved => format!("Reading {counts}."),
    };
    Some(PanePreview {
        summary,
        detail: Vec::new(),
    })
}

fn popover_preview(cx: &SchemaContext) -> PanePreview {
    let summary = if cx.waiting_sessions + cx.working_sessions + cx.recent_sessions == 0 {
        "Nothing to show yet — no session has run since Perch started watching.".to_string()
    } else {
        format!(
            "{} waiting · {} working · {} recent",
            cx.waiting_sessions, cx.working_sessions, cx.recent_sessions
        )
    };
    PanePreview {
        summary,
        detail: Vec::new(),
    }
}

fn projects_preview(s: &Settings, cx: &SchemaContext) -> PanePreview {
    if cx.project_last_used_days.is_empty() && cx.archived_projects == 0 {
        return PanePreview {
            summary: "No projects indexed yet.".to_string(),
            detail: Vec::new(),
        };
    }
    let active = cx
        .project_last_used_days
        .iter()
        .filter(|days| **days <= s.active_within_days)
        .count();
    let recent = cx.project_last_used_days.len() - active;
    let mut summary = format!("{active} Active · {recent} Recent");
    if cx.archived_projects > 0 {
        summary.push_str(&format!(" · {} Archived", cx.archived_projects));
    }
    PanePreview {
        summary,
        detail: vec![format!(
            "Active means touched within the last {}.",
            plural(i64::from(s.active_within_days), "day", "days")
        )],
    }
}

fn usage_preview(s: &Settings, cx: &SchemaContext) -> PanePreview {
    let summary = match (s.burn_rate, cx.burn_rate_line.as_ref()) {
        (BurnRate::Off, _) => "The burn-rate line is off.".to_string(),
        (_, Some(line)) => line.clone(),
        (_, None) => "Not enough data in this window to project a rate.".to_string(),
    };
    PanePreview {
        summary,
        detail: Vec::new(),
    }
}

fn prices_preview(cx: &SchemaContext) -> PanePreview {
    if cx.priced_models == 0 {
        return PanePreview {
            summary: "No model prices recorded yet.".to_string(),
            detail: Vec::new(),
        };
    }
    let priced = plural(i64::from(cx.priced_models), "model priced", "models priced");
    let summary = match cx.unpriced_models_in_use {
        0 => format!("{priced} · every model you have used has one."),
        1 => format!("{priced} · 1 model in use has no price."),
        n => format!("{priced} · {n} models in use have no price."),
    };
    PanePreview {
        summary,
        detail: vec!["Tokens are truth; dollars are an estimate.".to_string()],
    }
}

fn notifications_preview(cx: &SchemaContext) -> PanePreview {
    let summary = match cx.waiting_sessions {
        0 => "Nothing is waiting on you.".to_string(),
        1 => "1 session is waiting on you right now.".to_string(),
        n => format!("{n} sessions are waiting on you right now."),
    };
    PanePreview {
        summary,
        detail: Vec::new(),
    }
}

// --- Row and control construction -----------------------------------------

fn group(heading: &str, rows: Vec<SettingRow>) -> SettingGroup {
    SettingGroup {
        heading: Some(heading.to_string()),
        rows,
    }
}

/// A row that stores something. `is_default` is answered here, once, by
/// asking the same `get` the shell would — never by a per-field comparison
/// each caller could get subtly wrong.
fn row(s: &Settings, key: SettingKey, label: &str, help: &str, control: Control) -> SettingRow {
    SettingRow {
        key: Some(key),
        label: label.to_string(),
        help: Some(help.to_string()),
        control,
        enabled: true,
        is_default: s.get(key) == Settings::default().get(key),
        indent: false,
        search_terms: Vec::new(),
    }
}

/// A row that reports rather than stores, so it has no key and nothing to
/// reset. Its value is already a finished string.
fn info_row(label: &str, value_label: String) -> SettingRow {
    SettingRow {
        key: None,
        label: label.to_string(),
        help: None,
        control: Control::Info { value_label },
        enabled: true,
        is_default: true,
        indent: false,
        search_terms: Vec::new(),
    }
}

impl SettingRow {
    /// This row is only reachable while its master is on, and renders
    /// indented beneath it. Resolved here so the shell reads a bool and the
    /// rule exists in one language.
    fn depends_on(mut self, master_is_on: bool) -> Self {
        self.enabled = master_is_on;
        self.indent = true;
        self
    }

    fn searchable(mut self, terms: &[&str]) -> Self {
        self.search_terms = terms.iter().map(|t| (*t).to_string()).collect();
        self
    }
}

/// A stepper over a counted `u32`, with its caption composed here — the one
/// place in the codebase that decides what "1 minute" reads like.
fn counted_stepper(value: u32, min: i64, max: i64, one: &str, many: &str) -> Control {
    let value = i64::from(value);
    Control::Stepper {
        value,
        min,
        max,
        step: 1,
        value_label: plural(value, one, many),
    }
}

// Each option list is built from its enum's own `KNOWN_WIRE_VALUES`, so an
// id cannot drift from what `Settings::set` accepts, and labelled by an
// exhaustive match, so a new variant cannot slip through unlabelled.

fn menu_bar_icon_options() -> Vec<ChoiceOption> {
    MenuBarIcon::KNOWN_WIRE_VALUES
        .iter()
        .map(|wire| {
            let label = match MenuBarIcon::from_wire_str(wire) {
                MenuBarIcon::Bird => "Bird",
                MenuBarIcon::Binoculars => "Binoculars",
                MenuBarIcon::Dot => "Dot",
                MenuBarIcon::Bars => "Bars",
            };
            ChoiceOption {
                id: (*wire).to_string(),
                label: label.to_string(),
            }
        })
        .collect()
}

fn menu_bar_display_options() -> Vec<ChoiceOption> {
    MenuBarDisplay::KNOWN_WIRE_VALUES
        .iter()
        .map(|wire| {
            let label = match MenuBarDisplay::from_wire_str(wire) {
                MenuBarDisplay::Icon => "Nothing — just the icon",
                MenuBarDisplay::Count => "How many sessions are running",
                MenuBarDisplay::CountAndWaiting => "The count, and an hourglass when waiting",
            };
            ChoiceOption {
                id: (*wire).to_string(),
                label: label.to_string(),
            }
        })
        .collect()
}

fn row_density_options() -> Vec<ChoiceOption> {
    RowDensity::KNOWN_WIRE_VALUES
        .iter()
        .map(|wire| {
            let label = match RowDensity::from_wire_str(wire) {
                RowDensity::Comfortable => "Comfortable",
                RowDensity::Compact => "Compact",
            };
            ChoiceOption {
                id: (*wire).to_string(),
                label: label.to_string(),
            }
        })
        .collect()
}

fn burn_rate_options() -> Vec<ChoiceOption> {
    BurnRate::KNOWN_WIRE_VALUES
        .iter()
        .map(|wire| {
            let label = match BurnRate::from_wire_str(wire) {
                BurnRate::Off => "Nothing",
                BurnRate::TokensPerHour => "Tokens per hour",
                BurnRate::CostPerHour => "Dollars per hour",
                BurnRate::CostPerDay => "Dollars per day",
                BurnRate::ProjectedWindow => "Projected total for this window",
            };
            ChoiceOption {
                id: (*wire).to_string(),
                label: label.to_string(),
            }
        })
        .collect()
}

/// The four ranges `chart_days` offers, captioned here rather than left as
/// bare numbers for a shell to append "days" to.
fn chart_day_options() -> Vec<IntOption> {
    CHART_DAY_CHOICES
        .iter()
        .map(|days| IntOption {
            value: i64::from(*days),
            label: plural(i64::from(*days), "day", "days"),
        })
        .collect()
}

/// The folder a shell shows beside the picker: the one Perch would actually
/// read, so "" never renders as a blank field.
fn folder_label(cx: &SchemaContext) -> String {
    match &cx.claude_dir {
        DirStatus::Reading { label, .. } | DirStatus::NotFound { label } => label.clone(),
        DirStatus::Unresolved => "Chosen automatically".to_string(),
    }
}

fn reading_label(cx: &SchemaContext) -> String {
    match &cx.claude_dir {
        DirStatus::Reading { label, how } => format!("{label} — {how}"),
        DirStatus::NotFound { label } => {
            format!("{label} — not found, so Perch is reading nothing")
        }
        DirStatus::Unresolved => "Perch has not looked for Claude Code's folder yet".to_string(),
    }
}

fn resume_label(cx: &SchemaContext) -> String {
    cx.resume_command
        .clone()
        .unwrap_or_else(|| "Nothing yet — no terminal has been found to run it in".to_string())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Every row of every pane, flattened.
    pub(crate) fn all_rows(panes: &[SettingsPane]) -> Vec<&SettingRow> {
        panes
            .iter()
            .flat_map(|p| p.groups.iter())
            .flat_map(|g| g.rows.iter())
            .collect()
    }

    /// The row that reaches `key`. Panics if there is none, which
    /// `every_setting_key_appears_in_exactly_one_pane` guarantees cannot
    /// happen.
    pub(crate) fn find_row(panes: &[SettingsPane], key: SettingKey) -> &SettingRow {
        all_rows(panes)
            .into_iter()
            .find(|r| r.key == Some(key))
            .unwrap_or_else(|| panic!("{key:?} has no row"))
    }

    /// How many rows survived a filter.
    pub(crate) fn count_rows(panes: &[SettingsPane]) -> usize {
        all_rows(panes).len()
    }

    /// The first row of the first pane that has one. Panics if there is
    /// none, which is what a search test asserting a hit wants.
    pub(crate) fn first_row(panes: &[SettingsPane]) -> &SettingRow {
        all_rows(panes)
            .into_iter()
            .next()
            .expect("no pane kept a row")
    }

    pub(crate) fn pane(panes: &[SettingsPane], id: PaneId) -> &SettingsPane {
        panes
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{id:?} is not in the schema"))
    }

    /// That pane's preview, which it must have.
    pub(crate) fn preview(panes: &[SettingsPane], id: PaneId) -> &PanePreview {
        pane(panes, id)
            .preview
            .as_ref()
            .unwrap_or_else(|| panic!("{id:?} has no preview"))
    }

    /// A context in which nothing is wrong: the folder resolved, every model
    /// in use has a price, no record was rejected, nothing is waiting.
    pub(crate) fn healthy_context() -> SchemaContext {
        SchemaContext {
            claude_dir: DirStatus::Reading {
                label: "~/.claude".to_string(),
                how: "found automatically".to_string(),
            },
            project_last_used_days: vec![1, 2, 9],
            archived_projects: 0,
            indexed_sessions: 190,
            waiting_sessions: 0,
            working_sessions: 1,
            recent_sessions: 2,
            priced_models: 4,
            unpriced_models_in_use: 0,
            rejected_records: 0,
            terminal_choices: vec![ChoiceOption {
                id: "Terminal".to_string(),
                label: "Terminal".to_string(),
            }],
            resume_command: Some("claude --resume <session>".to_string()),
            menu_bar_title: Some("3".to_string()),
            burn_rate_line: Some("$1.20 per hour".to_string()),
        }
    }

    pub(crate) fn context_with_waiting_sessions(n: u32) -> SchemaContext {
        SchemaContext {
            waiting_sessions: n,
            ..healthy_context()
        }
    }

    pub(crate) fn context_with_projects_last_used_days_ago(days: &[u32]) -> SchemaContext {
        SchemaContext {
            project_last_used_days: days.to_vec(),
            ..healthy_context()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::settings::SettingValue;

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
        assert_eq!(
            before,
            SettingKey::ALL.len(),
            "a key has no row to reach it from"
        );
    }

    #[test]
    fn a_dependent_row_is_disabled_when_its_master_is_off() {
        let mut s = Settings {
            waiting_enabled: false,
            ..Settings::default()
        };
        let panes = build_schema(&s, &SchemaContext::empty());
        let row = find_row(&panes, SettingKey::WaitingAfterMinutes);
        assert!(
            !row.enabled,
            "the threshold means nothing while alerts are off"
        );

        s.waiting_enabled = true;
        let panes = build_schema(&s, &SchemaContext::empty());
        assert!(find_row(&panes, SettingKey::WaitingAfterMinutes).enabled);
    }

    #[test]
    fn a_row_knows_whether_it_still_holds_its_default() {
        let mut s = Settings::default();
        assert!(
            find_row(
                &build_schema(&s, &SchemaContext::empty()),
                SettingKey::PollSeconds
            )
            .is_default
        );
        s.poll_seconds = 30;
        assert!(
            !find_row(
                &build_schema(&s, &SchemaContext::empty()),
                SettingKey::PollSeconds
            )
            .is_default
        );
    }

    #[test]
    fn every_stored_setting_explains_itself() {
        // The first settings window was rejected as "very blank". The single
        // biggest reason a pane reads as substantial is that each row says
        // what it does underneath its label -- a sentence, not a fragment. A
        // row that stores something and explains nothing is the defect this
        // catches.
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
    fn attention_is_absent_when_nothing_is_wrong() {
        // A badge that is always there is decoration, and stops being read.
        let panes = build_schema(&Settings::default(), &healthy_context());
        assert!(panes.iter().all(|p| p.attention.is_none()));
    }

    #[test]
    fn notifications_says_so_when_it_is_off_while_sessions_wait() {
        let cx = context_with_waiting_sessions(2);
        let mut s = Settings {
            waiting_enabled: false,
            ..Settings::default()
        };
        let panes = build_schema(&s, &cx);
        let note = pane(&panes, PaneId::Notifications)
            .attention
            .as_deref()
            .unwrap_or("");
        assert!(
            note.contains('2'),
            "it must say how many, not merely that something is off"
        );

        s.waiting_enabled = true;
        let panes = build_schema(&s, &cx);
        assert!(pane(&panes, PaneId::Notifications).attention.is_none());
    }

    #[test]
    fn the_projects_pane_counts_the_users_own_projects_at_the_current_threshold() {
        let cx = context_with_projects_last_used_days_ago(&[1, 3, 20, 40]);
        let mut s = Settings {
            active_within_days: 7,
            ..Settings::default()
        };
        let summary = preview(&build_schema(&s, &cx), PaneId::Projects)
            .summary
            .clone();
        assert!(summary.contains("2 Active"), "got {summary:?}");

        s.active_within_days = 30;
        let summary = preview(&build_schema(&s, &cx), PaneId::Projects)
            .summary
            .clone();
        assert!(
            summary.contains("3 Active"),
            "the preview must follow the setting"
        );
    }

    #[test]
    fn every_group_is_titled() {
        // Grouping is what turns a list into a form. A group with no heading
        // is an ungrouped list wearing a card.
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        for pane in &panes {
            for group in &pane.groups {
                assert!(
                    group.heading.is_some(),
                    "{:?} has an untitled group",
                    pane.id
                );
            }
        }
    }

    #[test]
    fn every_stepper_reads_correctly_at_one() {
        // The sibling above catches the caption that was never composed;
        // this one catches the caption composed by interpolation. Every
        // counted setting is driven to 1 at once -- `set` stores what it is
        // given, bounds being `validated`'s question -- so a new stepper
        // that spells "1 days" cannot reach a build.
        let mut s = Settings::default();
        for key in SettingKey::ALL {
            if matches!(s.get(key), SettingValue::Int(_)) {
                s.set(key, SettingValue::Int(1)).expect("1 is a number");
            }
        }
        for row in all_rows(&build_schema(&s, &SchemaContext::empty())) {
            if let Control::Stepper { value_label, .. } = &row.control {
                assert!(
                    value_label.starts_with("1 ") && !value_label.ends_with('s'),
                    "{:?} reads {value_label:?} at one",
                    row.key
                );
            }
        }
    }

    #[test]
    fn resetting_a_pane_restores_only_its_own_keys() {
        let mut s = Settings {
            poll_seconds: 30,          // General
            waiting_after_minutes: 99, // Notifications
            ..Settings::default()
        };

        reset_pane(&mut s, PaneId::General);

        assert_eq!(
            s.poll_seconds,
            Settings::default().poll_seconds,
            "its own key resets"
        );
        assert_eq!(
            s.waiting_after_minutes, 99,
            "another pane's key is untouched"
        );
    }

    #[test]
    fn resetting_a_rowless_pane_changes_nothing() {
        // Prices, Diagnostics and Advanced hold no rows, so they own no keys.
        // A reset over one of them must be a no-op rather than reaching for
        // "everything", which is the shape this mistake takes.
        let mut s = Settings {
            poll_seconds: 30,
            waiting_after_minutes: 99,
            ..Settings::default()
        };
        let before = s.clone();

        for id in [PaneId::Prices, PaneId::Diagnostics, PaneId::Advanced] {
            reset_pane(&mut s, id);
        }

        assert_eq!(s, before, "a rowless pane owns no keys to reset");
    }

    #[test]
    fn every_stepper_carries_a_finished_caption() {
        let panes = build_schema(&Settings::default(), &SchemaContext::empty());
        for row in all_rows(&panes) {
            if let Control::Stepper { value_label, .. } = &row.control {
                assert!(
                    !value_label.is_empty(),
                    "{:?} left its caption to the shell",
                    row.key
                );
                assert!(
                    !value_label.ends_with(" 1 minutes"),
                    "caption is not pluralized"
                );
            }
        }
    }
}
