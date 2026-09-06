//! UniFFI surface over perch-core. Records mirror `perch_core::ui::model` exactly;
//! the shell renders them and nothing else.

use perch_core::db::Db;
use perch_core::platform::RealProcessProbe;
use perch_core::ui::{main_window, model as core_model, usage as core_usage, watcher};
use perch_core::{config, db, index, live, pricing};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PopoverModel {
    pub stats: Stats,
    pub live: Vec<SessionRow>,
    pub recent: Vec<RecentRow>,
    pub tray_title: String,
    pub error: Option<String>,
    pub waiting_banner: Option<String>,
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
        } = m;
        PopoverModel {
            stats: stats.into(),
            live: live.into_iter().map(Into::into).collect(),
            recent: recent.into_iter().map(Into::into).collect(),
            tray_title,
            error,
            waiting_banner,
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

/// Implemented by the shell. Called on the watcher thread; the shell hops to its UI thread.
#[uniffi::export(with_foreign)]
pub trait PerchListener: Send + Sync {
    fn on_model(&self, model: PopoverModel);
}

#[derive(uniffi::Object)]
pub struct Perch {
    config_dir: PathBuf,
    db_path: PathBuf,
    reindex_error: Arc<Mutex<Option<String>>>,
    handle: Mutex<Option<watcher::WatcherHandle>>,
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
    let dir = perch_core::settings::store::app_data_dir().map_err(|e| PerchError::Io {
        message: e.to_string(),
    })?;
    Ok(dir.join("index.db"))
}

/// The parts of `Perch` the watcher closure needs, without holding an
/// `Arc<Perch>` (which would keep the object alive past the shell's last
/// reference). `reindex_error` is a shared handle to the *last re-index
/// outcome* only — not a back-reference to `Perch` itself.
#[derive(Clone)]
struct ThisPerch {
    config_dir: PathBuf,
    db_path: PathBuf,
    reindex_error: Arc<Mutex<Option<String>>>,
}

impl ThisPerch {
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
        let mut model = core_model::build_model(db_result.as_ref().ok(), &sessions, now_ms());
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
            index::index_all(&database, &config::projects_dir(&self.config_dir))
                .map_err(|e| format!("could not index sessions: {e}"))?;
            Ok(())
        })();
        *self.reindex_error.lock().unwrap() = outcome.err();
    }
}

#[uniffi::export]
impl Perch {
    /// `config_dir: None` resolves per spec §3 (CLAUDE_CONFIG_DIR → XDG → ~/.claude).
    #[uniffi::constructor]
    pub fn new(config_dir: Option<String>) -> Result<Arc<Self>, PerchError> {
        let dir = match config_dir {
            Some(p) => PathBuf::from(p),
            None => config::config_dir().ok_or(PerchError::NoConfigDir {
                path: "<unresolved>".into(),
            })?,
        };
        if !dir.is_dir() {
            return Err(PerchError::NoConfigDir {
                path: dir.to_string_lossy().into_owned(),
            });
        }
        Ok(Arc::new(Self {
            config_dir: dir,
            db_path: app_data_db()?,
            reindex_error: Arc::new(Mutex::new(None)),
            handle: Mutex::new(None),
        }))
    }

    /// Synchronous snapshot: sessions from disk, stats from the index if it opens.
    pub fn current(&self) -> PopoverModel {
        let sessions =
            live::live_sessions(&config::sessions_dir(&self.config_dir), &RealProcessProbe);
        self.model_for(sessions)
    }

    /// Emit the current sessions immediately (from whatever the index
    /// already holds), then re-index in the background and emit again, then
    /// keep emitting on every change and at least every 5 s.
    ///
    /// Idempotent: a second call while already running is a no-op. Re-index
    /// work never runs on the caller's thread (AppKit's main thread calls
    /// this from `applicationDidFinishLaunching`) — see `refresh()`.
    pub fn start(&self, listener: Arc<dyn PerchListener>) {
        let mut guard = self.handle.lock().unwrap();
        if guard.is_some() {
            return;
        }
        let me = ThisPerch {
            config_dir: self.config_dir.clone(),
            db_path: self.db_path.clone(),
            reindex_error: self.reindex_error.clone(),
        };
        let me_for_refresh = me.clone();
        let cfg = watcher::WatcherConfig::for_dir(config::sessions_dir(&self.config_dir));
        let h = watcher::spawn(
            cfg,
            Arc::new(RealProcessProbe),
            move |sessions| listener.on_model(me.model_for(sessions)),
            move || me_for_refresh.reindex(),
        );
        // The watcher's own initial emit (above) is prompt but may be built
        // from a stale or empty index; ask it to re-index and emit again
        // right away, on its own thread, rather than blocking here for it.
        h.refresh();
        *guard = Some(h);
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

    /// Stop the watcher. Safe to call twice: a second call finds no handle
    /// and is a no-op.
    pub fn stop(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            h.stop();
        }
    }

    /// The whole window: `Now` plus every project the index knows. `now` is
    /// the *core* `PopoverModel` (see `core_model_for`'s doc comment for why),
    /// and a database that fails to open still yields a window — with the
    /// reason attached to `error` — rather than nothing.
    pub fn main_window(&self) -> MainWindowModel {
        let sessions =
            live::live_sessions(&config::sessions_dir(&self.config_dir), &RealProcessProbe);
        let now = self.core_model_for(sessions.clone());
        let db = db::open(&self.db_path).ok();
        main_window::build_main_window(db.as_ref(), now, &sessions, now_ms()).into()
    }

    /// One project in full: note, totals, sparkline, and every session.
    pub fn project_detail(&self, project_id: i64) -> Result<ProjectDetail, PerchError> {
        let database = self.open_db()?;
        self.detail(&database, project_id)
    }

    /// The Usage tab's view-model.
    pub fn usage(&self) -> Result<UsageModel, PerchError> {
        let database = self.open_db()?;
        core_usage::build_usage(&database, now_ms())
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
}

impl Perch {
    fn this(&self) -> ThisPerch {
        ThisPerch {
            config_dir: self.config_dir.clone(),
            db_path: self.db_path.clone(),
            reindex_error: self.reindex_error.clone(),
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
            live::live_sessions(&config::sessions_dir(&self.config_dir), &RealProcessProbe);
        main_window::build_project_detail(database, project_id, &sessions, now_ms())
            .map(Into::into)
            .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::Duration;

    struct Capture(Mutex<Vec<PopoverModel>>);
    impl PerchListener for Capture {
        fn on_model(&self, model: PopoverModel) {
            self.0.lock().unwrap().push(model);
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

        let cap = Arc::new(Capture(Mutex::new(Vec::new())));
        perch.start(cap.clone());
        std::thread::sleep(Duration::from_millis(500));
        perch.stop();
        assert!(
            !cap.0.lock().unwrap().is_empty(),
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
}
