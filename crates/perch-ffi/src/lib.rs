//! UniFFI surface over perch-core. Records mirror `perch_core::ui::model` exactly;
//! the shell renders them and nothing else.

use perch_core::platform::RealProcessProbe;
use perch_core::ui::{model as core_model, watcher};
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
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RecentRow {
    pub id: String,
    pub name: String,
    pub project: String,
    pub ended_ago: String,
    pub tokens: String,
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

impl From<core_model::Stats> for Stats {
    fn from(s: core_model::Stats) -> Self {
        Stats {
            window_tokens: s.window_tokens,
            week_tokens: s.week_tokens,
            day_tokens: s.day_tokens,
            day_cost: s.day_cost,
            estimated: s.estimated,
            has_data: s.has_data,
        }
    }
}

impl From<core_model::SessionRow> for SessionRow {
    fn from(r: core_model::SessionRow) -> Self {
        SessionRow {
            id: r.id,
            pid: r.pid,
            name: r.name,
            project: r.project,
            kind: r.kind,
            version: r.version,
            status: r.status.into(),
            status_label: r.status_label,
            elapsed: r.elapsed,
            tokens: r.tokens,
            cost: r.cost,
        }
    }
}

impl From<core_model::RecentRow> for RecentRow {
    fn from(r: core_model::RecentRow) -> Self {
        RecentRow {
            id: r.id,
            name: r.name,
            project: r.project,
            ended_ago: r.ended_ago,
            tokens: r.tokens,
        }
    }
}

impl From<core_model::PopoverModel> for PopoverModel {
    fn from(m: core_model::PopoverModel) -> Self {
        PopoverModel {
            stats: m.stats.into(),
            live: m.live.into_iter().map(Into::into).collect(),
            recent: m.recent.into_iter().map(Into::into).collect(),
            tray_title: m.tray_title,
            error: m.error,
            waiting_banner: m.waiting_banner,
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

/// Perch's own database. Never inside the Claude Code config dir.
///
/// Honours `PERCH_DATA_DIR` (the directory to hold `index.db`) when set, so
/// tests never touch the user's real index; falls back to the platform
/// default app-data directory otherwise.
fn app_data_db() -> Result<PathBuf, PerchError> {
    if let Ok(dir) = std::env::var("PERCH_DATA_DIR") {
        return Ok(PathBuf::from(dir).join("index.db"));
    }
    let home = std::env::var("HOME").map_err(|_| PerchError::Io {
        message: "HOME is not set".into(),
    })?;
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Perch");
    #[cfg(not(target_os = "macos"))]
    let base = PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("perch");
    Ok(base.join("index.db"))
}

/// The parts of `Perch` the watcher closure needs, without holding an
/// `Arc<Perch>` (which would keep the object alive past the shell's last
/// reference). `reindex_error` is a shared handle to the *last re-index
/// outcome* only — not a back-reference to `Perch` itself.
#[derive(Clone)]
struct ThisPerch {
    db_path: PathBuf,
    reindex_error: Arc<Mutex<Option<String>>>,
}

impl ThisPerch {
    fn model_for(&self, sessions: Vec<live::LiveSession>) -> PopoverModel {
        let db = db::open(&self.db_path).ok();
        let mut model: PopoverModel =
            core_model::build_model(db.as_ref(), &sessions, now_ms()).into();
        // A re-index failure must stay visible until a later re-index
        // succeeds, but it must not clobber a more specific error already
        // produced by `build_model` itself (e.g. a broken index schema).
        if let Some(err) = self.reindex_error.lock().unwrap().clone() {
            model.error.get_or_insert(err);
        }
        model
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

    /// Re-index, emit, then keep emitting on every change and at least every 5 s.
    ///
    /// Idempotent: a second call while already running is a no-op.
    pub fn start(&self, listener: Arc<dyn PerchListener>) {
        let mut guard = self.handle.lock().unwrap();
        if guard.is_some() {
            return;
        }
        self.reindex();
        let me = ThisPerch {
            db_path: self.db_path.clone(),
            reindex_error: self.reindex_error.clone(),
        };
        let cfg = watcher::WatcherConfig::for_dir(config::sessions_dir(&self.config_dir));
        let h = watcher::spawn(cfg, Arc::new(RealProcessProbe), move |sessions| {
            listener.on_model(me.model_for(sessions));
        });
        *guard = Some(h);
    }

    /// Re-index and emit now. Called when the menu opens.
    pub fn refresh(&self) {
        self.reindex();
        if let Some(h) = self.handle.lock().unwrap().as_ref() {
            h.refresh();
        }
    }

    /// Stop the watcher. Safe to call twice: a second call finds no handle
    /// and is a no-op.
    pub fn stop(&self) {
        if let Some(h) = self.handle.lock().unwrap().take() {
            h.stop();
        }
    }
}

impl Perch {
    fn model_for(&self, sessions: Vec<live::LiveSession>) -> PopoverModel {
        ThisPerch {
            db_path: self.db_path.clone(),
            reindex_error: self.reindex_error.clone(),
        }
        .model_for(sessions)
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
}
