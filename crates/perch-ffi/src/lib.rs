//! UniFFI surface over perch-core. Records mirror `perch_core::ui::model` exactly;
//! the shell renders them and nothing else.

use perch_core::db::Db;
use perch_core::platform::RealProcessProbe;
use perch_core::ui::{
    diagnostics as core_diagnostics, main_window, model as core_model,
    settings as core_ui_settings, usage as core_usage, watcher,
};
use perch_core::{config, db, index, live, notify, pricing, query, settings};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

uniffi::setup_scaffolding!();

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Status {
    Working,
    Idle,
    Waiting,
    Background,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Stats {
    pub window_tokens: String,
    pub week_tokens: String,
    pub day_tokens: String,
    pub day_cost: String,
    pub estimated: bool,
    pub has_data: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionRow {
    pub id: String,
    pub pid: i32,
    pub name: String,
    pub project: String,
    pub kind: String,
    pub version: String,
    pub status: Status,
    pub status_label: String,
    pub elapsed: String,
    pub tokens: String,
    pub cost: String,
    pub folder: String,
    pub detail_line: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RecentRow {
    pub id: String,
    pub name: String,
    pub project: String,
    pub ended_ago: String,
    pub tokens: String,
    pub ended_line: String,
}

/// Mirrors `perch_core::ui::model::Staleness`. A finished sentence, not a
/// duration for a shell to phrase: the shell dims and draws, nothing more.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Staleness {
    pub label: String,
}

/// Mirrors `perch_core::settings::RowDensity`. Crosses on the *model* rather
/// than on the `Settings` record below, because it is the model that tells a
/// shell how to draw — a shell reading the preference for itself is the
/// divergence this mirror exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RowDensity {
    Comfortable,
    Compact,
}

impl From<settings::RowDensity> for RowDensity {
    fn from(d: settings::RowDensity) -> Self {
        match d {
            settings::RowDensity::Comfortable => RowDensity::Comfortable,
            settings::RowDensity::Compact => RowDensity::Compact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PopoverModel {
    pub stats: Stats,
    pub live: Vec<SessionRow>,
    pub recent: Vec<RecentRow>,
    pub tray_title: String,
    pub error: Option<String>,
    pub waiting_banner: Option<String>,
    pub staleness: Option<Staleness>,
    pub show_waiting: bool,
    pub show_working: bool,
    pub show_recent: bool,
    pub row_density: RowDensity,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PerchError {
    #[error("no Claude Code config directory at {path}")]
    NoConfigDir { path: String },
    #[error("database error: {message}")]
    Database { message: String },
    #[error("io error: {message}")]
    Io { message: String },
}

impl From<core_model::Status> for Status {
    fn from(s: core_model::Status) -> Self {
        match s {
            core_model::Status::Working => Status::Working,
            core_model::Status::Idle => Status::Idle,
            core_model::Status::Waiting => Status::Waiting,
            core_model::Status::Background => Status::Background,
        }
    }
}

// Every `From` impl below destructures the core value (rather than reading
// named fields off it) so that adding a field to a `perch_core::ui::model`
// record is a compile error here, not a silent gap: without the destructure,
// a new core field would compile fine while never reaching any shell.

impl From<core_model::Stats> for Stats {
    fn from(s: core_model::Stats) -> Self {
        let core_model::Stats {
            window_tokens,
            week_tokens,
            day_tokens,
            day_cost,
            estimated,
            has_data,
        } = s;
        Stats {
            window_tokens,
            week_tokens,
            day_tokens,
            day_cost,
            estimated,
            has_data,
        }
    }
}

impl From<core_model::SessionRow> for SessionRow {
    fn from(r: core_model::SessionRow) -> Self {
        let core_model::SessionRow {
            id,
            pid,
            name,
            project,
            kind,
            version,
            status,
            status_label,
            elapsed,
            tokens,
            cost,
            folder,
            detail_line,
        } = r;
        SessionRow {
            id,
            pid,
            name,
            project,
            kind,
            version,
            status: status.into(),
            status_label,
            elapsed,
            tokens,
            cost,
            folder,
            detail_line,
        }
    }
}

impl From<core_model::RecentRow> for RecentRow {
    fn from(r: core_model::RecentRow) -> Self {
        let core_model::RecentRow {
            id,
            name,
            project,
            ended_ago,
            tokens,
            ended_line,
        } = r;
        RecentRow {
            id,
            name,
            project,
            ended_ago,
            tokens,
            ended_line,
        }
    }
}

impl From<core_model::PopoverModel> for PopoverModel {
    fn from(m: core_model::PopoverModel) -> Self {
        let core_model::PopoverModel {
            stats,
            live,
            recent,
            tray_title,
            error,
            waiting_banner,
            staleness,
            show_waiting,
            show_working,
            show_recent,
            row_density,
        } = m;
        PopoverModel {
            stats: stats.into(),
            live: live.into_iter().map(Into::into).collect(),
            recent: recent.into_iter().map(Into::into).collect(),
            tray_title,
            error,
            waiting_banner,
            staleness: staleness.map(|s| Staleness { label: s.label }),
            show_waiting,
            show_working,
            show_recent,
            row_density: row_density.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProjectGroup {
    Pinned,
    Active,
    Recent,
    Archived,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectRow {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub group: ProjectGroup,
    pub session_count: String,
    pub tokens: String,
    pub cost: String,
    pub last_active: String,
    pub live_session_count: u32,
    pub subtitle: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SparkPoint {
    pub day_index: i32,
    pub tokens: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionHistoryRow {
    pub id: String,
    pub name: String,
    pub started: String,
    pub duration: String,
    pub tokens: String,
    pub cost: String,
    pub branch: Option<String>,
    pub is_live: bool,
    pub detail_line: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectDetail {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub note: String,
    pub tokens: String,
    pub cost: String,
    pub session_count: String,
    pub sparkline: Vec<SparkPoint>,
    pub sessions: Vec<SessionHistoryRow>,
    pub pinned: bool,
    pub archived: bool,
    pub path_exists: bool,
    /// This project's own notification override — not the global setting.
    pub notify: NotifyOverride,
    /// What `NotifyOverride::Default` currently means, spelled out (see
    /// `main_window::default_notify_label`) — always present, regardless of
    /// `notify`'s own value. For display only: never parse it.
    pub notify_default_label: String,
    /// The number behind that label — the global "waiting on you" threshold
    /// in minutes. What a shell seeds its "Custom" minute control from, so
    /// the starting value is Rust's product decision rather than a literal
    /// in the shell that drifts when the default moves.
    pub notify_default_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MainWindowModel {
    pub now: PopoverModel,
    pub projects: Vec<ProjectRow>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HeroStat {
    pub label: String,
    pub value: String,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DailyBar {
    pub day_index: i32,
    pub label: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub thinking: u64,
    pub total_label: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RankedProject {
    pub name: String,
    pub tokens: u64,
    pub tokens_label: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: u64,
    pub tokens_label: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UsageModel {
    pub hero: Vec<HeroStat>,
    pub daily: Vec<DailyBar>,
    pub top_projects: Vec<RankedProject>,
    pub by_model: Vec<ModelUsage>,
    pub burn_rate: Option<String>,
    pub has_data: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TerminalCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub shell_line: String,
}

impl From<main_window::ProjectGroup> for ProjectGroup {
    fn from(g: main_window::ProjectGroup) -> Self {
        match g {
            main_window::ProjectGroup::Pinned => ProjectGroup::Pinned,
            main_window::ProjectGroup::Active => ProjectGroup::Active,
            main_window::ProjectGroup::Recent => ProjectGroup::Recent,
            main_window::ProjectGroup::Archived => ProjectGroup::Archived,
        }
    }
}

impl From<main_window::ProjectRow> for ProjectRow {
    fn from(r: main_window::ProjectRow) -> Self {
        let main_window::ProjectRow {
            id,
            name,
            path,
            group,
            session_count,
            tokens,
            cost,
            last_active,
            live_session_count,
            subtitle,
        } = r;
        ProjectRow {
            id,
            name,
            path,
            group: group.into(),
            session_count,
            tokens,
            cost,
            last_active,
            live_session_count,
            subtitle,
        }
    }
}

impl From<main_window::SparkPoint> for SparkPoint {
    fn from(p: main_window::SparkPoint) -> Self {
        let main_window::SparkPoint {
            day_index,
            tokens,
            label,
        } = p;
        SparkPoint {
            day_index,
            tokens,
            label,
        }
    }
}

impl From<main_window::SessionHistoryRow> for SessionHistoryRow {
    fn from(r: main_window::SessionHistoryRow) -> Self {
        let main_window::SessionHistoryRow {
            id,
            name,
            started,
            duration,
            tokens,
            cost,
            branch,
            is_live,
            detail_line,
        } = r;
        SessionHistoryRow {
            id,
            name,
            started,
            duration,
            tokens,
            cost,
            branch,
            is_live,
            detail_line,
        }
    }
}

impl From<main_window::ProjectDetail> for ProjectDetail {
    fn from(d: main_window::ProjectDetail) -> Self {
        let main_window::ProjectDetail {
            id,
            name,
            path,
            note,
            tokens,
            cost,
            session_count,
            sparkline,
            sessions,
            pinned,
            archived,
            path_exists,
            notify,
            notify_default_label,
            notify_default_minutes,
        } = d;
        ProjectDetail {
            id,
            name,
            path,
            note,
            tokens,
            cost,
            session_count,
            sparkline: sparkline.into_iter().map(Into::into).collect(),
            sessions: sessions.into_iter().map(Into::into).collect(),
            notify: notify.into(),
            notify_default_label,
            notify_default_minutes,
            pinned,
            archived,
            path_exists,
        }
    }
}

impl From<main_window::MainWindowModel> for MainWindowModel {
    fn from(m: main_window::MainWindowModel) -> Self {
        let main_window::MainWindowModel {
            now,
            projects,
            error,
        } = m;
        MainWindowModel {
            now: now.into(),
            projects: projects.into_iter().map(Into::into).collect(),
            error,
        }
    }
}

impl From<core_usage::HeroStat> for HeroStat {
    fn from(h: core_usage::HeroStat) -> Self {
        let core_usage::HeroStat {
            label,
            value,
            caption,
        } = h;
        HeroStat {
            label,
            value,
            caption,
        }
    }
}

impl From<core_usage::DailyBar> for DailyBar {
    fn from(b: core_usage::DailyBar) -> Self {
        let core_usage::DailyBar {
            day_index,
            label,
            input,
            output,
            cache_read,
            cache_write,
            thinking,
            total_label,
        } = b;
        DailyBar {
            day_index,
            label,
            input,
            output,
            cache_read,
            cache_write,
            thinking,
            total_label,
        }
    }
}

impl From<core_usage::RankedProject> for RankedProject {
    fn from(p: core_usage::RankedProject) -> Self {
        let core_usage::RankedProject {
            name,
            tokens,
            tokens_label,
            cost,
        } = p;
        RankedProject {
            name,
            tokens,
            tokens_label,
            cost,
        }
    }
}

impl From<core_usage::ModelUsage> for ModelUsage {
    fn from(m: core_usage::ModelUsage) -> Self {
        let core_usage::ModelUsage {
            model,
            tokens,
            tokens_label,
            cost,
        } = m;
        ModelUsage {
            model,
            tokens,
            tokens_label,
            cost,
        }
    }
}

impl From<core_usage::UsageModel> for UsageModel {
    fn from(m: core_usage::UsageModel) -> Self {
        let core_usage::UsageModel {
            hero,
            daily,
            top_projects,
            by_model,
            burn_rate,
            has_data,
        } = m;
        UsageModel {
            hero: hero.into_iter().map(Into::into).collect(),
            daily: daily.into_iter().map(Into::into).collect(),
            top_projects: top_projects.into_iter().map(Into::into).collect(),
            by_model: by_model.into_iter().map(Into::into).collect(),
            burn_rate,
            has_data,
        }
    }
}

impl From<perch_core::actions::TerminalCommand> for TerminalCommand {
    fn from(c: perch_core::actions::TerminalCommand) -> Self {
        let shell_line = c.shell_line();
        let perch_core::actions::TerminalCommand { program, args, cwd } = c;
        TerminalCommand {
            program,
            args,
            cwd,
            shell_line,
        }
    }
}

// --- Settings, diagnostics, the per-project override, and notifications ---
// Everything below mirrors `perch_core::settings`, `perch_core::ui::settings`,
// `perch_core::ui::diagnostics`, `perch_core::db::NotifyOverride` and
// `perch_core::notify::Notification`. `Settings` and `NotifyOverride` cross
// the boundary in *both* directions (a shell both reads and writes them), so
// each gets a `From` impl each way; both directions destructure their source
// for the same reason every other mirror in this file does — so a field
// added on either side becomes a compile error here, not a silent gap.

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MenuBarDisplay {
    Icon,
    Count,
    CountAndWaiting,
}

impl From<settings::MenuBarDisplay> for MenuBarDisplay {
    fn from(d: settings::MenuBarDisplay) -> Self {
        match d {
            settings::MenuBarDisplay::Icon => MenuBarDisplay::Icon,
            settings::MenuBarDisplay::Count => MenuBarDisplay::Count,
            settings::MenuBarDisplay::CountAndWaiting => MenuBarDisplay::CountAndWaiting,
        }
    }
}

impl From<MenuBarDisplay> for settings::MenuBarDisplay {
    fn from(d: MenuBarDisplay) -> Self {
        match d {
            MenuBarDisplay::Icon => settings::MenuBarDisplay::Icon,
            MenuBarDisplay::Count => settings::MenuBarDisplay::Count,
            MenuBarDisplay::CountAndWaiting => settings::MenuBarDisplay::CountAndWaiting,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Settings {
    pub launch_at_login: bool,
    pub claude_config_dir: String,
    pub menu_bar_display: MenuBarDisplay,
    pub poll_seconds: u32,
    pub waiting_enabled: bool,
    pub waiting_after_minutes: u32,
    pub include_background: bool,
    pub preferred_terminal: String,
}

/// `perch_core`'s `Settings` now carries eighteen keys this record does not
/// yet mirror — the settings window that reaches them is built on the schema
/// surface, not on this flat record, so growing it here would be a second
/// spelling of the same twenty-six values.
///
/// Every one of those eighteen is still destructured below, bound to `_`
/// rather than swept up by `..`, so the house guarantee holds unchanged: a
/// *new* field added to `perch_core::settings::Settings` is a compile error
/// here, not a silent omission.
impl From<settings::Settings> for Settings {
    fn from(s: settings::Settings) -> Self {
        let settings::Settings {
            launch_at_login,
            claude_config_dir,
            menu_bar_display,
            menu_bar_icon: _,
            dim_when_stale: _,
            stale_after_minutes: _,
            poll_seconds,
            preferred_terminal,
            show_waiting: _,
            show_working: _,
            show_recent: _,
            recent_limit: _,
            row_density: _,
            show_row_folder: _,
            show_row_usage: _,
            active_within_days: _,
            show_archived: _,
            chart_days: _,
            top_projects_count: _,
            top_projects_days: _,
            burn_rate: _,
            show_cost: _,
            waiting_enabled,
            waiting_after_minutes,
            include_background,
            sound: _,
        } = s;
        Settings {
            launch_at_login,
            claude_config_dir,
            menu_bar_display: menu_bar_display.into(),
            poll_seconds,
            waiting_enabled,
            waiting_after_minutes,
            include_background,
            preferred_terminal,
        }
    }
}

impl Settings {
    /// Lay this record's eight values over `base`, leaving every key it does
    /// not mirror as `base` has it.
    ///
    /// Deliberately not `From<Settings> for settings::Settings`: a `From`
    /// could only fill the other eighteen from `Settings::default()`, which
    /// would make saving one toggle in the settings window silently reset
    /// every hand-edited value in `config.toml` that this record cannot see.
    /// Merging onto what is already on disk is the only honest conversion
    /// while the two shapes differ.
    fn merged_onto(self, base: settings::Settings) -> settings::Settings {
        let Settings {
            launch_at_login,
            claude_config_dir,
            menu_bar_display,
            poll_seconds,
            waiting_enabled,
            waiting_after_minutes,
            include_background,
            preferred_terminal,
        } = self;
        settings::Settings {
            launch_at_login,
            claude_config_dir,
            menu_bar_display: menu_bar_display.into(),
            poll_seconds,
            waiting_enabled,
            waiting_after_minutes,
            include_background,
            preferred_terminal,
            ..base
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SettingsModel {
    pub settings: Settings,
    pub config_path: String,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

impl From<core_ui_settings::SettingsModel> for SettingsModel {
    fn from(m: core_ui_settings::SettingsModel) -> Self {
        let core_ui_settings::SettingsModel {
            settings,
            config_path,
            notes,
            error,
        } = m;
        SettingsModel {
            settings: settings.into(),
            config_path,
            notes,
            error,
        }
    }
}

/// Per-project override of the global notification setting (see
/// `db::NotifyOverride`). Crosses the boundary both ways: `set_notify_override`
/// takes one as input, and `ProjectDetail.notify` reports a project's current
/// one back — so, like `Settings`, it gets a `From` each way, both
/// destructuring for the same reason as every other conversion here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NotifyOverride {
    Default,
    Off,
    Custom { after_minutes: u32 },
}

impl From<NotifyOverride> for db::NotifyOverride {
    fn from(o: NotifyOverride) -> Self {
        match o {
            NotifyOverride::Default => db::NotifyOverride::Default,
            NotifyOverride::Off => db::NotifyOverride::Off,
            NotifyOverride::Custom { after_minutes } => {
                db::NotifyOverride::Custom { after_minutes }
            }
        }
    }
}

impl From<db::NotifyOverride> for NotifyOverride {
    fn from(o: db::NotifyOverride) -> Self {
        match o {
            db::NotifyOverride::Default => NotifyOverride::Default,
            db::NotifyOverride::Off => NotifyOverride::Off,
            db::NotifyOverride::Custom { after_minutes } => {
                NotifyOverride::Custom { after_minutes }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum RecordVerdict {
    Accepted,
    NoSuchProcess,
    NotClaude { actual: String },
    Unparsable,
}

impl From<core_diagnostics::RecordVerdict> for RecordVerdict {
    fn from(v: core_diagnostics::RecordVerdict) -> Self {
        match v {
            core_diagnostics::RecordVerdict::Accepted => RecordVerdict::Accepted,
            core_diagnostics::RecordVerdict::NoSuchProcess => RecordVerdict::NoSuchProcess,
            core_diagnostics::RecordVerdict::NotClaude { actual } => {
                RecordVerdict::NotClaude { actual }
            }
            core_diagnostics::RecordVerdict::Unparsable => RecordVerdict::Unparsable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RecordRow {
    pub file: String,
    pub pid: i32,
    pub session_id: String,
    pub verdict: RecordVerdict,
    pub verdict_label: String,
}

impl From<core_diagnostics::RecordRow> for RecordRow {
    fn from(r: core_diagnostics::RecordRow) -> Self {
        let core_diagnostics::RecordRow {
            file,
            pid,
            session_id,
            verdict,
            verdict_label,
        } = r;
        RecordRow {
            file,
            pid,
            session_id,
            verdict: verdict.into(),
            verdict_label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiagnosticsModel {
    pub config_dir: String,
    pub config_dir_source: String,
    pub sessions_dir: String,
    pub records: Vec<RecordRow>,
    pub settings_path: String,
    pub settings_loaded: bool,
    pub index_path: String,
    pub index_sessions: String,
    pub index_turns: String,
    pub last_indexed: String,
}

impl From<core_diagnostics::DiagnosticsModel> for DiagnosticsModel {
    fn from(m: core_diagnostics::DiagnosticsModel) -> Self {
        let core_diagnostics::DiagnosticsModel {
            config_dir,
            config_dir_source,
            sessions_dir,
            records,
            settings_path,
            settings_loaded,
            index_path,
            index_sessions,
            index_turns,
            last_indexed,
        } = m;
        DiagnosticsModel {
            config_dir,
            config_dir_source,
            sessions_dir,
            records: records.into_iter().map(Into::into).collect(),
            settings_path,
            settings_loaded,
            index_path,
            index_sessions,
            index_turns,
            last_indexed,
        }
    }
}

/// Mirrors `perch_core::notify::Notification`, named differently on this side
/// of the boundary: `Notification` is also `Foundation.Notification` in every
/// Swift file that imports both `PerchFFI` and `Foundation`/`AppKit` (which is
/// most of them) — the two would collide as soon as either was written
/// unqualified, so this one is spelled out instead.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WaitingNotification {
    pub session_id: String,
    /// What a click on this alert routes to: the id of the indexed project
    /// the waiting session is running in — unique, and the same key every
    /// other model on this boundary is addressed by. `project` below is only
    /// the directory name Perch displays, and two projects can share one
    /// (`~/work/api` and `~/personal/api`), so it must never be used to
    /// identify a project.
    ///
    /// `None` when the index has never seen that directory. A shell that
    /// gets `None` has nothing to select and must select nothing — guessing
    /// from `project` is exactly the wrong answer this field exists to
    /// prevent.
    pub project_id: Option<i64>,
    pub project: String,
    pub title: String,
    pub body: String,
}

impl From<notify::Notification> for WaitingNotification {
    fn from(n: notify::Notification) -> Self {
        let notify::Notification {
            session_id,
            project_id,
            project,
            title,
            body,
        } = n;
        WaitingNotification {
            session_id,
            project_id,
            project,
            title,
            body,
        }
    }
}

/// Implemented by the shell. Called on the watcher thread; the shell hops to its UI thread.
#[uniffi::export(with_foreign)]
pub trait PerchListener: Send + Sync {
    fn on_model(&self, model: PopoverModel);
    /// Edge-triggered "waiting on you" alerts decided by `notify::decide` for
    /// this tick — never called with an empty vector; a tick with nothing new
    /// to say simply doesn't call this at all.
    fn on_notifications(&self, items: Vec<WaitingNotification>);
}

#[derive(uniffi::Object)]
pub struct Perch {
    /// Mutable at runtime: a settings-file hand-edit (or a `save_settings`
    /// call) that changes `claude_config_dir` swaps this in place rather
    /// than requiring the shell to tear down and reconstruct `Perch` itself
    /// — see `on_settings_file_changed`.
    config_dir: Arc<Mutex<PathBuf>>,
    db_path: PathBuf,
    config_path: PathBuf,
    reindex_error: Arc<Mutex<Option<String>>>,
    /// Shared with every `ThisPerch` this engine hands out — see that
    /// struct's own field for what it holds and why it lives here.
    last_read_ms: Arc<Mutex<Option<i64>>>,
    handle: Mutex<Option<watcher::WatcherHandle>>,
    /// The live watch on `config.toml` itself, started by `start()`. `None`
    /// before `start()` runs (or after `stop()`).
    settings_watch: Mutex<Option<settings::store::WatchHandle>>,
    /// Stashed by `start()` so `on_settings_file_changed` can rebuild the
    /// session watcher against the same listener when `poll_seconds` or
    /// `claude_config_dir` changes underneath a running engine.
    listener: Mutex<Option<Arc<dyn PerchListener>>>,
    /// The settings this engine last saw — compared against on every
    /// settings-file watch tick so an unrelated poll (the watch fires
    /// `on_change` on every tick of its own backstop, not only on an actual
    /// change — see `settings::store::watch`'s own doc comment) is a no-op
    /// rather than an unconditional restart.
    last_settings: Mutex<settings::Settings>,
    /// Episodes already notified for (see `notify::decide`), shared across every
    /// watcher tick so a still-waiting session is never notified twice. Lives
    /// here, not in the shell: the shell only ever sees the finished
    /// `WaitingNotification`s a tick produces, never the bookkeeping behind them.
    notified: Arc<Mutex<HashSet<notify::Episode>>>,
    /// Lets a callback that must outlive any single method call (the
    /// settings watch's `on_change`) reach back into this object without
    /// holding a strong reference that would keep it alive forever — see
    /// `start()`. Never upgraded from inside `Perch` itself, only handed to
    /// that one callback.
    self_ref: Weak<Perch>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Fold any core error into the one typed database variant the shell sees.
/// Used by every read/edit method below so a failure is always surfaced,
/// never `.ok()`-discarded.
fn db_err(e: impl std::fmt::Display) -> PerchError {
    PerchError::Database {
        message: e.to_string(),
    }
}

/// Perch's own database. Never inside the Claude Code config dir.
///
/// Honours `PERCH_DATA_DIR` (the directory to hold `index.db`) when set, so
/// tests never touch the user's real index; falls back to the platform
/// default app-data directory otherwise. The resolution itself now lives in
/// `perch_core::settings::store::app_data_dir` (moved there in the settings
/// task, which needed the identical directory for `config.toml`) — this
/// just appends this file's own name to it.
fn app_data_db() -> Result<PathBuf, PerchError> {
    let dir = settings::store::app_data_dir().map_err(|e| PerchError::Io {
        message: e.to_string(),
    })?;
    Ok(dir.join("index.db"))
}

/// Where `config.toml` lives: `PERCH_CONFIG` if set, else `config.toml`
/// inside the same app-data directory `app_data_db` resolves `index.db`
/// against (see `settings::store::config_path`).
fn config_toml_path() -> Result<PathBuf, PerchError> {
    settings::store::config_path().map_err(|e| PerchError::Io {
        message: e.to_string(),
    })
}

/// The Claude Code directory an empty (or absent) `Perch::new` constructor
/// override resolves to: the settings file's own `claude_config_dir` when
/// it names one, else the ordinary environment-based resolution
/// (`config::config_dir`) every other caller falls through to. This mirrors
/// `ui::diagnostics::config_dir_source`'s own precedence, which already
/// reports the settings override as beating every env-derived source — the
/// two must never disagree about which one wins.
fn resolve_config_dir_override(claude_config_dir: &str) -> Option<PathBuf> {
    let trimmed = claude_config_dir.trim();
    if !trimmed.is_empty() {
        return Some(PathBuf::from(trimmed));
    }
    config::config_dir()
}

/// The parts of `Perch` the watcher closure needs, without holding an
/// `Arc<Perch>` (which would keep the object alive past the shell's last
/// reference). `reindex_error` is a shared handle to the *last re-index
/// outcome* only — not a back-reference to `Perch` itself.
#[derive(Clone)]
struct ThisPerch {
    config_dir: Arc<Mutex<PathBuf>>,
    db_path: PathBuf,
    config_path: PathBuf,
    reindex_error: Arc<Mutex<Option<String>>>,
    notified: Arc<Mutex<HashSet<notify::Episode>>>,
    /// When the index was last opened and read successfully — `None` until
    /// that has happened once. This is the *only* place that knows, because
    /// this is the only place that reads, which is why `build_model` takes it
    /// as an argument instead of reaching for a clock of its own.
    last_read_ms: Arc<Mutex<Option<i64>>>,
}

impl ThisPerch {
    fn config_dir(&self) -> PathBuf {
        self.config_dir.lock().unwrap().clone()
    }

    fn model_for(&self, sessions: Vec<live::LiveSession>) -> PopoverModel {
        self.core_model_for(sessions).into()
    }

    /// Same as `model_for`, but stops short of converting to the FFI mirror.
    /// `main_window()` needs exactly this core-typed value (`build_main_window`
    /// takes `perch_core::ui::model::PopoverModel`, not the FFI one) — building
    /// it directly here, rather than adding a one-call-site FFI-to-core
    /// conversion, is the choice Task 6's brief asked to be recorded.
    fn core_model_for(&self, sessions: Vec<live::LiveSession>) -> core_model::PopoverModel {
        // Captured (not `.ok()`-discarded): this same file is also opened by
        // the retained Tauri app, so a concurrent writer can make this fail
        // with `SQLITE_BUSY` on an otherwise-healthy index — and this path
        // runs on every watcher tick, not just around a re-index, so it is
        // the only place that ever sees that failure.
        let db_result = db::open(&self.db_path);
        // Loaded fresh, independently of `notifications_for`'s own load
        // (same duplication that method already carries, and cheap for the
        // same reason: parsing one small file): a hand-edited
        // `menu_bar_display` — or `recent_limit`, which this model also reads
        // now — must be reflected the moment the *next* tick's model goes
        // out, not only after some separate settings-watch machinery reacts.
        let loaded_settings = settings::store::load(&self.config_path).settings;
        // This tick's read is the newest successful one when the index
        // opened; when it did not, the last one that did still stands, and
        // the gap between it and now is exactly the staleness the user asked
        // to be dimmed for.
        let now = now_ms();
        let last_read = {
            let mut last = self.last_read_ms.lock().unwrap();
            if db_result.is_ok() {
                *last = Some(now);
            }
            *last
        };
        let mut model = core_model::build_model(
            db_result.as_ref().ok(),
            &sessions,
            now,
            last_read,
            &loaded_settings,
        );
        // Neither fold may clobber a more specific error `build_model` itself
        // already produced (e.g. a broken index schema): this tick's open
        // failure is more specific than a possibly-stale re-index failure
        // from an earlier tick, so it is folded first.
        if let Err(e) = &db_result {
            model
                .error
                .get_or_insert_with(|| format!("could not open index: {e}"));
        }
        if let Some(err) = self.reindex_error.lock().unwrap().clone() {
            model.error.get_or_insert(err);
        }
        model
    }

    /// Re-index the database, recording (rather than discarding) any
    /// failure. The design's rule is that database trouble must be visible;
    /// `PopoverModel::error` exists for exactly this, so a failure here is
    /// folded into every model emitted afterwards (via `model_for` above)
    /// until a later re-index succeeds and clears it — there is no separate
    /// notification path, so nothing downstream can silently miss it.
    fn reindex(&self) {
        let outcome = (|| -> Result<(), String> {
            let database =
                db::open(&self.db_path).map_err(|e| format!("could not open index: {e}"))?;
            pricing::seed_default_prices(&database)
                .map_err(|e| format!("could not seed prices: {e}"))?;
            index::index_all(&database, &config::projects_dir(&self.config_dir()))
                .map_err(|e| format!("could not index sessions: {e}"))?;
            Ok(())
        })();
        *self.reindex_error.lock().unwrap() = outcome.err();
    }

    /// This tick's id and `NotifyOverride` for every project the index knows,
    /// keyed by the project's `real_path` — the same string `LiveSession::cwd`
    /// carries (see `main_window`'s identical `l.cwd == s.real_path` match).
    /// One database round trip per tick, not one per session: both of
    /// `decide`'s lookups below are pure map lookups over this.
    ///
    /// The id rides along with the override rather than costing a second
    /// query because `project_summaries` already carries it — it is what a
    /// finished notification points at, so a click routes to the project the
    /// alert was about rather than to whichever indexed project happens to
    /// share its directory name.
    ///
    /// A database that won't open, or a summary query that fails, yields an
    /// empty map — every project then falls through to `NotifyOverride::Default`
    /// (and to no id) for this tick. That is the same graceful-degradation the
    /// rest of this file gives a watcher tick (see `core_model_for`'s own
    /// `db::open`): the tick's `PopoverModel.error` already carries an open
    /// failure when there is one, and `Vec<WaitingNotification>` has no error
    /// channel of its own to carry a second, redundant report of the identical
    /// trouble.
    fn projects_by_cwd(&self) -> HashMap<String, (i64, db::NotifyOverride)> {
        let Ok(database) = db::open(&self.db_path) else {
            return HashMap::new();
        };
        // One `project_meta` query per *project*, not per session (that's the
        // shape the brief asked to avoid: an N+1 against live session count).
        // This is still an N+1 against project count, run once per tick —
        // a deliberate choice at today's scale, not an oversight: `Db` has no
        // "every project's meta in one query" method yet, and adding one
        // purely for this cache would be new surface for a cost that isn't
        // showing up. Revisit if this is ever measured against hundreds of
        // projects.
        query::project_summaries(&database)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                database
                    .project_meta(s.id)
                    .ok()
                    .map(|m| (s.real_path, (s.id, m.notify)))
            })
            .collect()
    }

    /// This tick's edge-triggered notifications (see `notify::decide`),
    /// updating the shared episode memory in place so the next tick picks up
    /// exactly where this one left off.
    fn notifications_for(&self, sessions: &[live::LiveSession]) -> Vec<WaitingNotification> {
        let loaded_settings = settings::store::load(&self.config_path).settings;
        let projects = self.projects_by_cwd();
        let override_for = |cwd: &str| {
            projects
                .get(cwd)
                .map(|(_, notify)| *notify)
                .unwrap_or(db::NotifyOverride::Default)
        };
        // A `cwd` the index has never seen resolves to no id at all — the
        // notification then names no project rather than a wrong one.
        let project_id_for = |cwd: &str| projects.get(cwd).map(|(id, _)| *id);

        let mut notified = self.notified.lock().unwrap();
        let (items, remembered) = notify::decide(
            sessions,
            &loaded_settings,
            &override_for,
            &project_id_for,
            &notified,
            now_ms(),
        );
        *notified = remembered;
        items.into_iter().map(Into::into).collect()
    }

    /// One watcher tick, start to finish: the model and the notifications it
    /// earns, both delivered to `listener`. Factored out of `start`'s closure
    /// so it is directly testable against hand-built `LiveSession`s, without
    /// a live watcher thread or `RealProcessProbe` needing an actual `claude`
    /// process to find.
    fn tick(&self, listener: &dyn PerchListener, sessions: Vec<live::LiveSession>) {
        let notifications = self.notifications_for(&sessions);
        listener.on_model(self.model_for(sessions));
        if !notifications.is_empty() {
            listener.on_notifications(notifications);
        }
    }
}

/// Composes the "After N minutes" label for a notification override a shell's
/// stepper is *currently* showing. Free-standing rather than a `Perch` method
/// because it reads no state: it exists purely so the pluralization happens
/// here, under the rule that a shell never composes a sentence from a number.
#[uniffi::export]
pub fn custom_notify_label(minutes: u32) -> String {
    perch_core::ui::main_window::custom_notify_label(minutes)
}

/// The settings window's two stepper captions, free-standing for the same
/// reason `custom_notify_label` is: they read no state, and they exist so the
/// number a stepper is showing is pluralized here rather than interpolated
/// into a sentence by the shell.
#[uniffi::export]
pub fn poll_seconds_label(seconds: u32) -> String {
    core_ui_settings::poll_seconds_label(seconds)
}

#[uniffi::export]
pub fn waiting_after_minutes_label(minutes: u32) -> String {
    core_ui_settings::waiting_after_minutes_label(minutes)
}

/// The settings a first run starts from. Exported so a shell showing a
/// control before its model has loaded can seed it from Rust's own default
/// instead of re-declaring one of its own — a literal in the shell is a
/// second source of truth that drifts the moment this one changes.
#[uniffi::export]
pub fn default_settings() -> Settings {
    perch_core::settings::Settings::default().into()
}

#[uniffi::export]
impl Perch {
    /// `config_dir: None` resolves per spec §3 (CLAUDE_CONFIG_DIR → XDG →
    /// ~/.claude) — *unless* the settings file itself names a
    /// `claude_config_dir` override, which beats every one of those (see
    /// `resolve_config_dir_override`, and `ui::diagnostics::config_dir_source`,
    /// which already reports exactly this precedence). An explicit `Some(p)`
    /// (tests, and any future caller with its own reason to bypass settings
    /// entirely) still beats both.
    ///
    /// A settings override that names something unusable never reaches the
    /// failure below: `Settings::validated` has already dropped it back to
    /// auto-detection and earned a note the settings window shows. That
    /// matters here more than anywhere else — every control in that window
    /// is disabled while the engine is down, so a constructor that failed on
    /// a bad `claude_config_dir` would leave the user with no way to edit or
    /// clear the very field that caused it.
    #[uniffi::constructor]
    pub fn new(config_dir: Option<String>) -> Result<Arc<Self>, PerchError> {
        let config_path = config_toml_path()?;
        let dir = match config_dir {
            Some(p) => PathBuf::from(p),
            None => {
                let claude_config_dir = settings::store::load(&config_path)
                    .settings
                    .claude_config_dir;
                // `.filter(is_dir).or_else(...)` is defence in depth, not the
                // mechanism: as the code stands `claude_config_dir` is either
                // empty or a real directory by the time `load` returns it, so
                // the filter never fires. It is here because this constructor
                // is the one place where getting this wrong costs the user
                // the only UI that could fix it — so it never trusts an
                // override far enough to refuse to start over one, whatever
                // some future loader might hand it.
                resolve_config_dir_override(&claude_config_dir)
                    .filter(|d| d.is_dir())
                    .or_else(config::config_dir)
                    .ok_or(PerchError::NoConfigDir {
                        path: "<unresolved>".into(),
                    })?
            }
        };
        if !dir.is_dir() {
            return Err(PerchError::NoConfigDir {
                path: dir.to_string_lossy().into_owned(),
            });
        }
        let db_path = app_data_db()?;
        Ok(Arc::new_cyclic(|weak| Self {
            config_dir: Arc::new(Mutex::new(dir)),
            db_path,
            config_path,
            reindex_error: Arc::new(Mutex::new(None)),
            last_read_ms: Arc::new(Mutex::new(None)),
            handle: Mutex::new(None),
            settings_watch: Mutex::new(None),
            listener: Mutex::new(None),
            last_settings: Mutex::new(settings::Settings::default()),
            notified: Arc::new(Mutex::new(HashSet::new())),
            self_ref: weak.clone(),
        }))
    }

    /// Synchronous snapshot: sessions from disk, stats from the index if it opens.
    pub fn current(&self) -> PopoverModel {
        let sessions =
            live::live_sessions(&config::sessions_dir(&self.config_dir()), &RealProcessProbe);
        self.model_for(sessions)
    }

    /// Emit the current sessions immediately (from whatever the index
    /// already holds), then re-index in the background and emit again, then
    /// keep emitting on every change and at least every `poll_seconds`
    /// (today's settings value, read once here — see `spawn_watcher` and
    /// `on_settings_file_changed` for how a later change to it takes effect
    /// without this method being called again).
    ///
    /// Also starts a watch on `config.toml` itself, so a hand-edited
    /// `poll_seconds` or `claude_config_dir` reaches a *running* engine —
    /// see `on_settings_file_changed`.
    ///
    /// Idempotent: a second call while already running is a no-op. Re-index
    /// work never runs on the caller's thread (AppKit's main thread calls
    /// this from `applicationDidFinishLaunching`) — see `refresh()`.
    pub fn start(&self, listener: Arc<dyn PerchListener>) {
        let loaded = settings::store::load(&self.config_path).settings;

        // Checking "already running?" and claiming the slot are one critical
        // section, the same way `refresh()` and `stop()` each decide inside a
        // single `self.handle.lock()`. Released and retaken, two callers can
        // both read `None` and both spawn a watcher — and only the second
        // ends up in `handle`, so `stop()` can never reach the first.
        //
        // Nothing else is locked while this guard is held: `build_watcher`
        // touches no other field of `self`, and the assignments below wait
        // until it is dropped. `on_settings_file_changed` takes `listener`
        // and *then* `handle`, so taking them the other way round here would
        // be a lock-order inversion between this thread and that one.
        {
            let mut handle = self.handle.lock().unwrap();
            if handle.is_some() {
                return;
            }
            *handle = Some(self.build_watcher(listener.clone(), loaded.poll_seconds));
        }
        *self.last_settings.lock().unwrap() = loaded;
        *self.listener.lock().unwrap() = Some(listener);

        // `self_ref` upgraded only transiently, inside the callback, each
        // time it actually fires — never held onto — so this watch thread
        // can never keep `Perch` alive past the shell's last reference (see
        // `self_ref`'s own doc comment).
        let weak = self.self_ref.clone();
        let watch = settings::store::watch(self.config_path.clone(), move || {
            if let Some(me) = weak.upgrade() {
                me.on_settings_file_changed();
            }
        });
        *self.settings_watch.lock().unwrap() = Some(watch);
    }

    /// Emit now. Called when the menu opens.
    ///
    /// While the watcher is running, this is a pure signal — the re-index it
    /// triggers runs on the watcher thread, not here, so a slow index never
    /// blocks the caller (AppKit calls this from `menuWillOpen`, while it is
    /// preparing to display the menu). With no watcher running yet, there is
    /// no thread to hand the work to, so it runs here instead.
    pub fn refresh(&self) {
        match self.handle.lock().unwrap().as_ref() {
            Some(h) => h.refresh(),
            None => self.reindex(),
        }
    }

    /// Stop the session watcher and the settings-file watch. Safe to call
    /// twice: a second call finds neither handle and is a no-op.
    pub fn stop(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            h.stop();
        }
        if let Some(w) = self.settings_watch.lock().unwrap().take() {
            w.stop();
        }
    }

    /// The whole window: `Now` plus every project the index knows. `now` is
    /// the *core* `PopoverModel` (see `core_model_for`'s doc comment for why),
    /// and a database that fails to open still yields a window — with the
    /// reason attached to `error` — rather than nothing.
    pub fn main_window(&self) -> MainWindowModel {
        let sessions =
            live::live_sessions(&config::sessions_dir(&self.config_dir()), &RealProcessProbe);
        let now = self.core_model_for(sessions.clone());
        let db = db::open(&self.db_path).ok();
        // The Active/Recent boundary is `active_within_days`; loaded here for
        // the same reason `core_model_for` loads its own copy.
        let loaded_settings = settings::store::load(&self.config_path).settings;
        main_window::build_main_window(db.as_ref(), now, &sessions, now_ms(), &loaded_settings)
            .into()
    }

    /// One project in full: note, totals, sparkline, and every session.
    pub fn project_detail(&self, project_id: i64) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        self.detail(&database, project_id)
    }

    /// The Usage tab's view-model.
    pub fn usage(&self) -> Result<UsageModel, PerchError> {
        let database = self.open_db()?;
        let loaded_settings = settings::store::load(&self.config_path).settings;
        core_usage::build_usage(&database, now_ms(), &loaded_settings)
            .map(Into::into)
            .map_err(db_err)
    }

    /// Overwrite a project's note, returning the refreshed detail so the
    /// shell never has to re-fetch or guess what changed.
    pub fn set_note(&self, project_id: i64, note: String) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        database.set_note(project_id, &note).map_err(db_err)?;
        self.detail(&database, project_id)
    }

    /// Pin or unpin a project.
    pub fn set_pinned(&self, project_id: i64, pinned: bool) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        database.set_pinned(project_id, pinned).map_err(db_err)?;
        self.detail(&database, project_id)
    }

    /// Archive or unarchive a project.
    pub fn set_archived(
        &self,
        project_id: i64,
        archived: bool,
    ) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        database
            .set_archived(project_id, archived)
            .map_err(db_err)?;
        self.detail(&database, project_id)
    }

    /// Set a project's display name.
    pub fn rename_project(
        &self,
        project_id: i64,
        name: String,
    ) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        database
            .set_display_name(project_id, &name)
            .map_err(db_err)?;
        self.detail(&database, project_id)
    }

    /// `claude --resume <session_id>` in `cwd`, as a ready-to-run shell line.
    pub fn resume_command(&self, session_id: String, cwd: String) -> TerminalCommand {
        perch_core::actions::TerminalCommand::resume(&session_id, &cwd).into()
    }

    /// A fresh `claude` session in `cwd`, as a ready-to-run shell line.
    pub fn open_command(&self, cwd: String) -> TerminalCommand {
        perch_core::actions::TerminalCommand::open(&cwd).into()
    }

    /// Today's settings, the file they came from, and anything wrong with
    /// that file. Never fails outright — see `settings::store::load`.
    pub fn settings(&self) -> SettingsModel {
        core_ui_settings::build_settings(&self.config_path).into()
    }

    /// Save `s` to disk, format-preserving, then read it straight back —
    /// so the returned model is exactly what a fresh `settings()` call
    /// would see, including any clamp `load` applies and the note it earns.
    pub fn save_settings(&self, s: Settings) -> Result<SettingsModel, PerchError> {
        // Merge over what is on disk rather than over defaults: this record
        // mirrors eight of the file's twenty-six keys, and the other
        // eighteen must survive a save that never mentioned them.
        let on_disk = settings::store::load(&self.config_path).settings;
        settings::store::save(&self.config_path, &s.merged_onto(on_disk)).map_err(|e| {
            PerchError::Io {
                message: e.to_string(),
            }
        })?;
        Ok(self.settings())
    }

    /// The diagnostics pane's view-model: where Perch thinks the Claude Code
    /// directory is and why, every session record found there with a
    /// spelled-out verdict, and the state of Perch's own settings file and
    /// index. A database that won't open still yields a full picture — with
    /// dashes for the index-derived fields alone (see
    /// `ui::diagnostics::build_diagnostics`) — never nothing.
    pub fn diagnostics(&self) -> DiagnosticsModel {
        let db = db::open(&self.db_path).ok();
        core_diagnostics::build_diagnostics(
            db.as_ref(),
            &self.config_dir(),
            &self.config_path,
            &RealProcessProbe,
            now_ms(),
        )
        .into()
    }

    /// Set a project's notification override, returning the refreshed detail
    /// so the shell never has to re-fetch or guess what changed.
    pub fn set_notify_override(
        &self,
        project_id: i64,
        o: NotifyOverride,
    ) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        database
            .set_notify_override(project_id, &o.into())
            .map_err(db_err)?;
        self.detail(&database, project_id)
    }
}

impl Perch {
    fn config_dir(&self) -> PathBuf {
        self.config_dir.lock().unwrap().clone()
    }

    fn this(&self) -> ThisPerch {
        ThisPerch {
            config_dir: self.config_dir.clone(),
            db_path: self.db_path.clone(),
            config_path: self.config_path.clone(),
            reindex_error: self.reindex_error.clone(),
            notified: self.notified.clone(),
            last_read_ms: self.last_read_ms.clone(),
        }
    }

    fn model_for(&self, sessions: Vec<live::LiveSession>) -> PopoverModel {
        self.this().model_for(sessions)
    }

    /// See `ThisPerch::core_model_for`.
    fn core_model_for(&self, sessions: Vec<live::LiveSession>) -> core_model::PopoverModel {
        self.this().core_model_for(sessions)
    }

    /// Synchronous fallback for `refresh()` when no watcher is running (see
    /// `ThisPerch::reindex` for the real work and its error-visibility rule).
    fn reindex(&self) {
        self.this().reindex();
    }

    /// (Re)builds the session watcher against the current config dir and
    /// poll interval, storing it as the running handle and replacing
    /// whatever was there (if any — the caller is responsible for having
    /// stopped it first when this is a restart rather than the initial
    /// `start()`).
    ///
    /// Only for a restart, where the handle is taken and replaced in two
    /// steps under a lock this does not hold. `start()` calls
    /// `build_watcher` directly instead, because it must decide *and* claim
    /// inside one critical section — see its own comment.
    fn spawn_watcher(&self, listener: Arc<dyn PerchListener>, poll_seconds: u32) {
        *self.handle.lock().unwrap() = Some(self.build_watcher(listener, poll_seconds));
    }

    /// The watcher itself, built and started but not stored anywhere, so the
    /// caller decides under which lock it becomes *the* running watcher.
    /// Reads `config_dir` and nothing else of `self`'s locked state, which
    /// is what makes it safe to call with `handle` held. Emits once
    /// immediately (`h.refresh()`) so a start or restart is never silently
    /// quiet until the next tick.
    fn build_watcher(
        &self,
        listener: Arc<dyn PerchListener>,
        poll_seconds: u32,
    ) -> watcher::WatcherHandle {
        let me = self.this();
        let me_for_refresh = me.clone();
        let cfg = watcher::WatcherConfig::for_dir(
            config::sessions_dir(&self.config_dir()),
            Duration::from_secs(u64::from(poll_seconds)),
        );
        let h = watcher::spawn(
            cfg,
            Arc::new(RealProcessProbe),
            move |sessions| me.tick(listener.as_ref(), sessions),
            move || me_for_refresh.reindex(),
        );
        h.refresh();
        h
    }

    /// Called (debounced, trailing-edge, at least once per `config.toml`
    /// poll tick) by the settings-file watch `start()` sets up. Most ticks
    /// are not an actual change — `settings::store::watch` fires `on_change`
    /// on every tick of its own backstop poll, not only when the file
    /// genuinely changed (documented on that function) — so the very first
    /// thing this does is compare against `last_settings` and return if
    /// nothing this engine cares about actually moved. That comparison, not
    /// a debounce of its own, is what stops an idle engine from restarting
    /// its watcher every few seconds forever.
    ///
    /// Only two fields ever earn a restart here:
    /// - `claude_config_dir`: re-resolved and swapped into the shared
    ///   `config_dir` (so every read — `current`, `main_window`,
    ///   `diagnostics`, the session watcher's own `sessions_dir` — sees it
    ///   immediately), then a re-index against the new directory.
    /// - `poll_seconds`: the session watcher's own backstop interval, which
    ///   only `WatcherConfig` at spawn time can set (see `spawn_watcher`) —
    ///   there is no live "change this running thread's timer" knob, so the
    ///   only way to apply a new value is to stop the old watcher and spawn
    ///   a fresh one.
    ///
    /// Every other setting (`menu_bar_display`, `waiting_enabled`,
    /// `waiting_after_minutes`, `include_background`, `preferred_terminal`)
    /// is read fresh from disk by whoever needs it on its own next use
    /// (`core_model_for`, `notifications_for`, or the shell reading
    /// `settings()` at the moment it acts) — none of them need the watcher
    /// itself to restart, so none of them are compared here.
    fn on_settings_file_changed(&self) {
        let loaded = settings::store::load(&self.config_path).settings;
        let previous = {
            let mut last = self.last_settings.lock().unwrap();
            if *last == loaded {
                return;
            }
            std::mem::replace(&mut *last, loaded.clone())
        };

        let dir_changed = previous.claude_config_dir != loaded.claude_config_dir;
        let poll_changed = previous.poll_seconds != loaded.poll_seconds;

        // Only an actual, successful swap of `config_dir` earns a restart on
        // its own account — a `claude_config_dir` edit that fails to resolve
        // changes nothing about what the watcher should be doing, so it must
        // not restart the watcher over a no-op.
        //
        // A hand-edit naming something that isn't a directory no longer
        // arrives here as itself: `Settings::validated` drops it back to ""
        // on the way out of `load`, with a note the settings window shows —
        // so what can still fail below is only the *auto-detected* path
        // having gone away, which no setting can repair and which stderr is
        // the right (and only) place for.
        let mut dir_actually_moved = false;
        if dir_changed {
            match resolve_config_dir_override(&loaded.claude_config_dir) {
                Some(resolved) if resolved.is_dir() => {
                    *self.config_dir.lock().unwrap() = resolved;
                    self.reindex();
                    dir_actually_moved = true;
                }
                Some(resolved) => eprintln!(
                    "perch: settings: claude_config_dir {} is not a directory; keeping the previous one",
                    resolved.display()
                ),
                None => eprintln!(
                    "perch: settings: claude_config_dir cleared, but no directory could be \
                     auto-detected either; keeping the previous one"
                ),
            }
        }

        if dir_actually_moved || poll_changed {
            if let Some(listener) = self.listener.lock().unwrap().clone() {
                if let Some(old) = self.handle.lock().unwrap().take() {
                    old.stop();
                }
                self.spawn_watcher(listener, loaded.poll_seconds);
            }
        }
    }

    /// Open Perch's own index database, mapping any failure to the one typed
    /// variant every read/edit method surfaces — never `.ok()`-discarded.
    fn open_db(&self) -> Result<Db, PerchError> {
        db::open(&self.db_path).map_err(db_err)
    }

    /// The refreshed detail for `project_id`, built against an already-open
    /// `database` — shared by every edit method so each returns the updated
    /// state rather than making the shell re-fetch or guess what changed.
    fn detail(&self, database: &Db, project_id: i64) -> Result<ProjectDetail, PerchError> {
        let sessions =
            live::live_sessions(&config::sessions_dir(&self.config_dir()), &RealProcessProbe);
        // `build_project_detail` needs the global `waiting_after_minutes` to
        // compose `notify_default_label` (what "Default" currently means for
        // this project) — `&Settings` rather than the bare minute count,
        // matching `notify::decide`'s own calling convention elsewhere in
        // this file, and self-documenting at this call site.
        let loaded_settings = settings::store::load(&self.config_path).settings;
        main_window::build_project_detail(
            database,
            project_id,
            &sessions,
            now_ms(),
            &loaded_settings,
        )
        .map(Into::into)
        .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Capture {
        models: Mutex<Vec<PopoverModel>>,
        notifications: Mutex<Vec<Vec<WaitingNotification>>>,
    }
    impl PerchListener for Capture {
        fn on_model(&self, model: PopoverModel) {
            self.models.lock().unwrap().push(model);
        }
        fn on_notifications(&self, items: Vec<WaitingNotification>) {
            self.notifications.lock().unwrap().push(items);
        }
    }

    /// `cargo test` runs a crate's tests in parallel threads within one
    /// process, and `PERCH_DATA_DIR` is process-global — so any two tests
    /// that both call `std::env::set_var` on it race, genuinely, not just
    /// in theory. This lock serializes every test that touches the var;
    /// `DataDirGuard` (below) holds it for the guard's lifetime and clears
    /// the var on drop, so a panicking test still leaves it unset for
    /// whichever test runs next.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct DataDirGuard<'a> {
        _lock: MutexGuard<'a, ()>,
    }

    impl<'a> DataDirGuard<'a> {
        fn set(dir: &std::path::Path) -> Self {
            // A prior test panicking while holding the lock poisons it;
            // recover the guard rather than let that cascade into every
            // later test in this file.
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("PERCH_DATA_DIR", dir);
            Self { _lock: lock }
        }
    }

    impl Drop for DataDirGuard<'_> {
        fn drop(&mut self) {
            std::env::remove_var("PERCH_DATA_DIR");
        }
    }

    /// `CLAUDE_CONFIG_DIR` for the life of the guard, restored to whatever
    /// was there before on drop. Only ever taken while a `DataDirGuard` is
    /// held — that guard owns `ENV_LOCK`, which is what serializes this
    /// process-global write against every other env-touching test here.
    struct ClaudeDirGuard {
        prior: Option<String>,
    }

    impl ClaudeDirGuard {
        fn set(dir: &std::path::Path) -> Self {
            let prior = std::env::var("CLAUDE_CONFIG_DIR").ok();
            std::env::set_var("CLAUDE_CONFIG_DIR", dir);
            Self { prior }
        }
    }

    impl Drop for ClaudeDirGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }

    /// `start` is documented idempotent: "a second call while already
    /// running is a no-op". Two threads arriving together must not both come
    /// away believing they were the first — that leaves two watcher threads
    /// polling the same directory, only one of which `handle` still names,
    /// so `stop()` can never reach the other.
    ///
    /// Each thread brings its own listener, which is what makes the failure
    /// visible rather than inferred: exactly one watcher exists iff at most
    /// one of the two listeners is ever driven. Rounds, because the window
    /// between the check and the claim is short and a single round can get
    /// lucky — this is a race, so the test drives at it repeatedly rather
    /// than pretending one attempt proves anything.
    #[test]
    fn two_threads_calling_start_at_once_only_ever_start_one_watcher() {
        let data = tempfile::tempdir().unwrap();
        let _env = DataDirGuard::set(data.path());
        let tmp = tempfile::tempdir().unwrap();

        for round in 0..12 {
            let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
            let a = Arc::new(Capture::default());
            let b = Arc::new(Capture::default());
            let gate = Arc::new(std::sync::Barrier::new(2));

            let threads: Vec<_> = [a.clone(), b.clone()]
                .into_iter()
                .map(|cap| {
                    let perch = perch.clone();
                    let gate = gate.clone();
                    std::thread::spawn(move || {
                        gate.wait();
                        perch.start(cap);
                    })
                })
                .collect();
            for t in threads {
                t.join().unwrap();
            }

            // Long enough for a spawned watcher's immediate emit to land.
            std::thread::sleep(Duration::from_millis(200));
            perch.stop();

            let a_emits = a.models.lock().unwrap().len();
            let b_emits = b.models.lock().unwrap().len();
            assert!(
                a_emits == 0 || b_emits == 0,
                "round {round}: both listeners were driven, so both calls to \
                 start() spawned a watcher (a={a_emits} emits, b={b_emits} emits) \
                 — one of those watchers is now unreachable from `handle` and \
                 stop() will never reach it"
            );
            assert!(
                a_emits > 0 || b_emits > 0,
                "round {round}: neither listener was driven — the test is not \
                 observing a running watcher at all"
            );
        }
    }

    #[test]
    fn constructs_against_an_empty_config_dir_and_emits_a_model() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let _env = DataDirGuard::set(data_dir.path());
        // Perch's own DB must not land inside the (fake) config dir either.
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let snap = perch.current();
        assert!(!snap.stats.has_data);
        assert_eq!(snap.stats.window_tokens, "—");
        assert!(snap.live.is_empty());

        let cap = Arc::new(Capture::default());
        perch.start(cap.clone());
        std::thread::sleep(Duration::from_millis(500));
        perch.stop();
        assert!(
            !cap.models.lock().unwrap().is_empty(),
            "start must emit at least the initial model"
        );
        assert!(
            std::fs::read_dir(tmp.path()).unwrap().next().is_none(),
            "nothing written into the config dir"
        );
    }

    #[test]
    fn missing_config_dir_is_a_typed_error() {
        // No PERCH_DATA_DIR needed: `Perch::new` rejects a non-existent
        // config dir before it ever resolves the app-data path, so this
        // test has no reason to touch that (process-global) env var.
        let err = Perch::new(Some("/definitely/not/here".into()))
            .err()
            .expect("must fail");
        assert!(matches!(err, PerchError::NoConfigDir { .. }));
    }

    #[test]
    fn reindex_failure_surfaces_in_the_model_until_a_later_reindex_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tempfile::tempdir().unwrap();
        // A plain file where `app_data_db()`'s directory is expected: inside
        // `reindex()`, `db::open`'s `create_dir_all(parent)` then fails
        // because a regular file already occupies that path, so `reindex()`
        // cannot even open the database — the cleanest lever to force the
        // "could not open index" branch without touching perch-core.
        let blocker = data_root.path().join("blocked");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let _env = DataDirGuard::set(&blocker);

        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        perch.refresh(); // reindex() runs synchronously; no watcher needed.
        let snap = perch.current();
        assert!(
            snap.error.is_some(),
            "a re-index failure must surface in the model, not vanish silently"
        );

        // Clear the obstruction and let the next re-index succeed: the
        // model must reflect that the trouble is gone, not keep repeating
        // a stale error forever.
        std::fs::remove_file(&blocker).unwrap();
        perch.refresh();
        let snap = perch.current();
        assert!(
            snap.error.is_none(),
            "a later successful re-index must clear the earlier error"
        );
    }

    #[test]
    fn ffi_level_open_failure_surfaces_in_model_error_even_without_a_reindex() {
        // Same blocker technique as above, but this test never calls
        // `start()` or `refresh()`, so `reindex_error` stays `None` — the
        // only thing that can put an error on the model is `ThisPerch::model_for`'s
        // own `db::open` call, exactly the path the watcher's 5-second ticks
        // (and a concurrent-writer `SQLITE_BUSY`) go through. Before this fix
        // that call was `.ok()`-discarded and `snap.error` stayed `None`.
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tempfile::tempdir().unwrap();
        let blocker = data_root.path().join("blocked");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let _env = DataDirGuard::set(&blocker);

        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let snap = perch.current();
        assert!(
            snap.error.is_some(),
            "an unopenable index must surface in model.error on every tick, \
             not just around an explicit re-index"
        );
    }

    #[test]
    fn main_window_and_usage_are_reachable_against_an_empty_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let w = perch.main_window();
        assert!(w.projects.is_empty(), "an empty config dir has no projects");
        let u = perch.usage().unwrap();
        assert!(!u.has_data);
        assert_eq!(
            u.daily.len(),
            14,
            "the chart window is drawn even when empty"
        );
    }

    #[test]
    fn editing_an_unknown_project_is_a_typed_error_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        assert!(perch.set_pinned(4242, true).is_err());
        assert!(perch.project_detail(4242).is_err());
        assert!(perch
            .set_notify_override(4242, NotifyOverride::Off)
            .is_err());
    }

    #[test]
    fn commands_cross_the_boundary_with_their_shell_line_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let c = perch.resume_command("abc".into(), "/a/b".into());
        assert_eq!(c.shell_line, "cd '/a/b' && claude --resume 'abc'");
        let o = perch.open_command("/a/b".into());
        assert_eq!(o.shell_line, "cd '/a/b' && claude");
    }

    #[test]
    fn settings_is_reachable_and_starts_at_the_documented_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let model = perch.settings();
        assert_eq!(
            model.settings.poll_seconds, 5,
            "a first run has no file yet"
        );
        assert!(model.notes.is_empty());
        assert!(model.error.is_none());
        assert!(model.config_path.ends_with("config.toml"));
    }

    #[test]
    fn save_settings_round_trips_through_a_fresh_settings_call() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let mut s = perch.settings().settings;
        s.poll_seconds = 30;
        s.waiting_enabled = true;
        s.waiting_after_minutes = 20;
        s.menu_bar_display = MenuBarDisplay::CountAndWaiting;

        let saved = perch.save_settings(s.clone()).unwrap();
        assert_eq!(
            saved.settings, s,
            "save_settings returns exactly what a fresh read sees"
        );

        let reread = perch.settings();
        assert_eq!(
            reread.settings, s,
            "a later, independent settings() call must see what was saved"
        );
    }

    #[test]
    fn save_settings_clamps_out_of_range_values_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let mut s = perch.settings().settings;
        s.poll_seconds = 0;
        let saved = perch.save_settings(s).unwrap();
        assert_eq!(saved.settings.poll_seconds, 1, "clamped, not rejected");
        assert_eq!(saved.notes.len(), 1);
    }

    #[test]
    fn diagnostics_is_reachable_against_an_empty_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let d = perch.diagnostics();
        assert!(d.records.is_empty());
        assert_eq!(d.config_dir, tmp.path().display().to_string());
        assert!(d.settings_loaded, "no settings file yet is a first run");
        assert_eq!(
            d.index_sessions, "0",
            "a fresh index opens fine and is honestly empty"
        );
    }

    #[test]
    fn set_notify_override_persists_and_returns_the_refreshed_detail() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());

        // Seed a project directly against the same `index.db` `Perch::new`
        // will resolve (see `app_data_db`), so `set_notify_override` below
        // has a real row to update.
        let db_path = data.path().join("index.db");
        let project_id = {
            let database = db::open(&db_path).unwrap();
            database.upsert_project("slug", "/a/proj", false).unwrap()
        };

        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let detail = perch
            .set_notify_override(project_id, NotifyOverride::Custom { after_minutes: 45 })
            .unwrap();
        assert_eq!(detail.id, project_id);
        assert_eq!(
            detail.notify,
            NotifyOverride::Custom { after_minutes: 45 },
            "the refreshed detail must show the override it just wrote, not just accept it"
        );
        assert!(
            detail.notify_default_label.contains("10 minutes"),
            "a first run's global default (10 minutes) must still be reported \
             even though this project itself is on Custom: {}",
            detail.notify_default_label
        );

        let database = db::open(&db_path).unwrap();
        assert_eq!(
            database.project_meta(project_id).unwrap().notify,
            db::NotifyOverride::Custom { after_minutes: 45 },
            "the override must actually persist, not just echo back in the reply"
        );
    }

    #[test]
    fn a_project_never_given_its_own_override_reports_default_and_the_current_global_label() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());

        let db_path = data.path().join("index.db");
        let project_id = {
            let database = db::open(&db_path).unwrap();
            database.upsert_project("slug", "/a/proj", false).unwrap()
        };

        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let mut s = perch.settings().settings;
        s.waiting_after_minutes = 25;
        perch.save_settings(s).unwrap();

        let detail = perch.project_detail(project_id).unwrap();
        assert_eq!(
            detail.notify,
            NotifyOverride::Default,
            "a project never given its own override reads as Default"
        );
        assert!(
            detail.notify_default_label.contains("25 minutes"),
            "the label must reflect the saved global setting, not a stale default: {}",
            detail.notify_default_label
        );
        assert_eq!(
            detail.notify_default_minutes, 25,
            "the number behind that label has to cross the boundary too — it \
             is what the shell's Custom minute control seeds from, and the \
             shell must never parse the label to get it"
        );
    }

    #[test]
    fn a_waiting_session_past_threshold_pushes_a_notification_through_the_listener() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let mut s = perch.settings().settings;
        s.waiting_enabled = true;
        s.waiting_after_minutes = 1; // the shortest allowed threshold
        perch.save_settings(s).unwrap();

        let since_ms = now_ms() - 2 * 60_000; // 2 minutes ago: past the 1-minute threshold
        let session = live::LiveSession {
            pid: 1,
            session_id: "s1".into(),
            cwd: "/tmp/proj".into(),
            name: "s1".into(),
            kind: "interactive".into(),
            status: live::SessionStatus::Waiting {
                reason: None,
                since_ms,
            },
            started_at: since_ms,
            status_updated_at: since_ms,
            cc_version: None,
            socket_path: None,
        };

        let cap = Arc::new(Capture::default());
        perch.this().tick(cap.as_ref(), vec![session.clone()]);

        let pushed = cap.notifications.lock().unwrap().clone();
        assert_eq!(
            pushed.len(),
            1,
            "the push must reach the shell through on_notifications, not just on_model"
        );
        assert_eq!(pushed[0].len(), 1);
        assert_eq!(pushed[0][0].session_id, "s1");
        assert_eq!(pushed[0][0].project, "proj");

        // Edge-triggered: the same still-waiting episode must not refire on
        // a later tick — this is what the episode memory living inside
        // `Perch` (via `this()`, shared through `notified`) buys.
        perch.this().tick(cap.as_ref(), vec![session]);
        assert_eq!(
            cap.notifications.lock().unwrap().len(),
            1,
            "no repeat push for the same episode on a later tick"
        );
    }

    /// Two indexed projects whose directories share a last path component
    /// (`~/work/api` and `~/personal/api`) must be tellable apart from the
    /// notification payload alone. `project` — the directory name — is the
    /// same string for both, so a shell routing a click on it can only
    /// guess; `project_id` is what makes the click land on the project the
    /// alert was actually about.
    #[test]
    fn notifications_carry_the_project_id_so_a_shared_directory_name_still_routes() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());

        let db_path = data.path().join("index.db");
        let (work_api, personal_api) = {
            let database = db::open(&db_path).unwrap();
            (
                database
                    .upsert_project("-work-api", "/work/api", false)
                    .unwrap(),
                database
                    .upsert_project("-personal-api", "/personal/api", false)
                    .unwrap(),
            )
        };
        assert_ne!(work_api, personal_api);

        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let mut s = perch.settings().settings;
        s.waiting_enabled = true;
        s.waiting_after_minutes = 1;
        perch.save_settings(s).unwrap();

        let since_ms = now_ms() - 2 * 60_000;
        let waiting = |id: &str, cwd: &str| live::LiveSession {
            pid: 1,
            session_id: id.into(),
            cwd: cwd.into(),
            name: id.into(),
            kind: "interactive".into(),
            status: live::SessionStatus::Waiting {
                reason: None,
                since_ms,
            },
            started_at: since_ms,
            status_updated_at: since_ms,
            cc_version: None,
            socket_path: None,
        };

        let cap = Arc::new(Capture::default());
        perch.this().tick(
            cap.as_ref(),
            vec![
                waiting("s-work", "/work/api"),
                waiting("s-personal", "/personal/api"),
            ],
        );

        let pushed = cap.notifications.lock().unwrap().clone();
        assert_eq!(pushed.len(), 1);
        let mut routed: Vec<(String, Option<i64>, String)> = pushed[0]
            .iter()
            .map(|n| (n.session_id.clone(), n.project_id, n.project.clone()))
            .collect();
        routed.sort();
        assert_eq!(
            routed,
            vec![
                ("s-personal".to_string(), Some(personal_api), "api".into()),
                ("s-work".to_string(), Some(work_api), "api".into()),
            ],
            "both alerts say \"api\"; only the id tells the two projects apart"
        );
    }

    /// A waiting session in a directory the index has never seen has no
    /// project row to point at. The payload says so with `None` rather than
    /// naming some other project — a click that selects nothing is right,
    /// a click that silently selects the wrong project is not.
    #[test]
    fn an_unindexed_directory_notifies_with_no_project_id_rather_than_a_wrong_one() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let mut s = perch.settings().settings;
        s.waiting_enabled = true;
        s.waiting_after_minutes = 1;
        perch.save_settings(s).unwrap();

        let since_ms = now_ms() - 2 * 60_000;
        let session = live::LiveSession {
            pid: 1,
            session_id: "s1".into(),
            cwd: "/tmp/never-indexed".into(),
            name: "s1".into(),
            kind: "interactive".into(),
            status: live::SessionStatus::Waiting {
                reason: None,
                since_ms,
            },
            started_at: since_ms,
            status_updated_at: since_ms,
            cc_version: None,
            socket_path: None,
        };

        let cap = Arc::new(Capture::default());
        perch.this().tick(cap.as_ref(), vec![session]);
        let pushed = cap.notifications.lock().unwrap().clone();
        assert_eq!(pushed[0][0].project_id, None);
    }

    #[test]
    fn a_constructor_override_beats_the_settings_files_claude_config_dir() {
        let dir_a = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());

        let seed = settings::Settings {
            claude_config_dir: "/somewhere/settings/points/that/does/not/exist".into(),
            ..Default::default()
        };
        settings::store::save(&data.path().join("config.toml"), &seed).unwrap();

        // An explicit constructor argument still wins over the settings file
        // — this is what every other test in this module already relies on
        // to run against a throwaway `tempdir()` regardless of whatever a
        // stray real `config.toml` says.
        let perch = Perch::new(Some(dir_a.path().to_string_lossy().into_owned())).unwrap();
        assert_eq!(perch.config_dir(), dir_a.path());
    }

    #[test]
    fn a_blank_settings_override_resolves_via_the_ordinary_environment_precedence() {
        assert_eq!(resolve_config_dir_override("   "), config::config_dir());
    }

    #[test]
    fn a_nonblank_settings_override_beats_the_environment() {
        assert_eq!(
            resolve_config_dir_override("/custom/claude"),
            Some(PathBuf::from("/custom/claude"))
        );
    }

    #[test]
    fn constructing_with_no_override_honours_the_settings_files_claude_config_dir() {
        let dir_a = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());

        let seed = settings::Settings {
            claude_config_dir: dir_a.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        settings::store::save(&data.path().join("config.toml"), &seed).unwrap();

        let perch = Perch::new(None).unwrap();
        assert_eq!(
            perch.config_dir(),
            dir_a.path(),
            "the settings file's override must win over env-based auto-detection"
        );
    }

    /// A `claude_config_dir` naming something that is not a directory used to
    /// fail `Perch::new` outright, and every control in the settings window is
    /// disabled while the engine is down — so the one field that caused the
    /// failure could not be edited or cleared from the app at all. The value
    /// must degrade to auto-detection and say so, never to a window that
    /// cannot be used to fix it.
    #[test]
    fn a_claude_config_dir_that_is_not_a_directory_falls_back_instead_of_bricking_the_app() {
        let fallback = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let _claude = ClaudeDirGuard::set(fallback.path());

        let not_a_dir = data.path().join("regular-file");
        std::fs::write(&not_a_dir, b"i am a file, not a directory").unwrap();
        let seed = settings::Settings {
            claude_config_dir: not_a_dir.to_string_lossy().into_owned(),
            ..Default::default()
        };
        settings::store::save(&data.path().join("config.toml"), &seed).unwrap();

        let perch =
            Perch::new(None).expect("a bad claude_config_dir must never stop Perch from opening");
        assert_eq!(
            perch.config_dir(),
            fallback.path(),
            "the ordinary resolution order must take over, not the unusable override"
        );

        let notes = perch.settings().notes;
        assert!(
            notes.iter().any(|n| n.contains("claude_config_dir")),
            "and the settings window must be told why: {notes:?}"
        );
    }

    #[test]
    fn a_hand_edited_claude_config_dir_moves_a_running_engines_config_dir_without_restarting_perch()
    {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let config_path = data.path().join("config.toml");

        let seed = settings::Settings {
            claude_config_dir: dir_a.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        settings::store::save(&config_path, &seed).unwrap();

        let perch = Perch::new(None).unwrap();
        assert_eq!(perch.config_dir(), dir_a.path());

        let cap = Arc::new(Capture::default());
        perch.start(cap.clone());

        // Simulate a hand-edit made outside Perch entirely: load-mutate-save
        // through the exact same `settings::store` functions a text editor's
        // save would leave behind on disk, never touching `perch` itself.
        let mut edited = settings::store::load(&config_path).settings;
        edited.claude_config_dir = dir_b.path().to_string_lossy().into_owned();
        settings::store::save(&config_path, &edited).unwrap();

        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline && perch.config_dir() != dir_b.path() {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            perch.config_dir(),
            dir_b.path(),
            "a hand-edited claude_config_dir must reach a running engine on its own, \
             without the app needing to be restarted"
        );

        perch.stop();
    }

    #[test]
    fn a_hand_edited_poll_seconds_restarts_the_session_watcher_with_the_new_interval() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let _guard = DataDirGuard::set(data.path());
        let config_path = data.path().join("config.toml");

        // Start slow (30s) -- far longer than this test's own deadline below
        // -- so any tick this test observes after the edit can only be
        // explained by the watcher having actually restarted at the new,
        // much shorter interval, not by the original 30s poll happening to
        // land inside the window.
        let seed = settings::Settings {
            poll_seconds: 30,
            ..Default::default()
        };
        settings::store::save(&config_path, &seed).unwrap();

        let perch = Perch::new(Some(tmp.path().to_string_lossy().into_owned())).unwrap();
        let cap = Arc::new(Capture::default());
        perch.start(cap.clone());

        // Drain whatever the initial start-up emitted so the counting below
        // only reflects ticks from after the interval actually changes.
        std::thread::sleep(Duration::from_millis(300));
        let before = cap.models.lock().unwrap().len();

        let mut edited = settings::store::load(&config_path).settings;
        edited.poll_seconds = 1;
        settings::store::save(&config_path, &edited).unwrap();

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut seen_new_tick = false;
        while Instant::now() < deadline {
            if cap.models.lock().unwrap().len() > before {
                seen_new_tick = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            seen_new_tick,
            "a hand-edited poll_seconds must restart the watcher at the new interval, \
             well inside the old 30s poll"
        );

        perch.stop();
    }
}
