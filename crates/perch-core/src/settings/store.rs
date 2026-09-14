//! Where [`Settings`](super::Settings) lives on disk: path resolution,
//! loading (which never fails the caller), format-preserving saving, and
//! watching for hand edits.
//!
//! `save` is the one legitimate filesystem write in `perch-core` outside of
//! `db.rs` (Perch's own index database) -- this file, `config.toml`, is
//! Perch's own, never anything inside the Claude Code config directory.

use super::{MenuBarDisplay, Settings, SETTINGS_VERSION};
use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use toml_edit::{value, DocumentMut, Item, Table};

/// Perch's own application-support directory -- never inside the Claude
/// Code config directory. This used to be resolved only by `perch-ffi`, for
/// `index.db`; it now lives here so [`config_path`] can share the exact same
/// resolution rather than growing a second copy of it, and `perch-ffi`'s own
/// `app_data_db` calls this and appends its own filename.
///
/// Honours `PERCH_DATA_DIR` when set -- tests, and anyone who wants Perch's
/// data elsewhere -- the same precedence habit `CLAUDE_CONFIG_DIR` already
/// sets for the *Claude Code* directory in `config::config_dir`. The
/// `#[cfg]` split below is the one OS-conditional seam this module needs;
/// everything else in `perch-core` compiles the same on Linux as on macOS.
pub fn app_data_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("PERCH_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
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
    Ok(base)
}

/// `PERCH_CONFIG` names the settings file directly; otherwise it lives at
/// `config.toml` inside [`app_data_dir`].
pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("PERCH_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(app_data_dir()?.join("config.toml"))
}

/// The result of [`load`]: settings the app can run with right now, any
/// notes a hand-edited file's clamped values earned, and an error string
/// when the file itself could not be understood. `load` never fails outright
/// -- see its own doc comment for why.
pub struct Loaded {
    pub settings: Settings,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

fn defaults_loaded(error: Option<String>) -> Loaded {
    let (settings, notes) = Settings::default().validated();
    Loaded {
        settings,
        notes,
        error,
    }
}

/// Load settings from `path`. This never returns an error to the caller --
/// Perch must always start -- so every kind of trouble degrades to
/// something the app can run with instead:
///
/// - a missing file is a first run, not a failure: defaults, no error.
/// - a file that does not parse as TOML (or does not match the shape
///   `Settings` expects) is defaults, plus an error string naming the path
///   so the problem is actionable.
/// - an in-range-but-invalid value (out of bounds) is clamped by
///   [`Settings::validated`], which reports what it changed as a note.
pub fn load(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        // Absence is the one kind of trouble this function stays silent
        // about -- a first run has no settings file, and that is not an
        // error. Anything else that stops the file from being read (wrong
        // permissions, a directory sitting where the file should be, ...)
        // must say so instead of taking the identical silent path: a user
        // whose hand-edits quietly stopped taking effect deserves to know
        // why, exactly like a parse failure below.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return defaults_loaded(None),
        Err(e) => {
            return defaults_loaded(Some(format!(
                "{}: could not read settings ({e}); using defaults",
                path.display()
            )))
        }
    };

    let mut doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            return defaults_loaded(Some(format!(
                "{}: could not parse settings ({e}); using defaults",
                path.display()
            )))
        }
    };

    let existing_version = doc.get("version").and_then(Item::as_integer).unwrap_or(0);
    migrate(&mut doc, existing_version);

    // `MenuBarDisplay`'s hand-written `Deserialize` (see settings::mod)
    // already degrades an unrecognized `menu_bar.display` to the default by
    // the time `toml_edit::de::from_str` below returns -- it has no path
    // back to a caller-visible note, only that silent default. This is the
    // one place left that can still see the raw string on the way in, so it
    // is the one place that can report it.
    let mut extra_notes = Vec::new();
    if let Some(display) = doc
        .get("menu_bar")
        .and_then(Item::as_table)
        .and_then(|t| t.get("display"))
        .and_then(Item::as_str)
    {
        if !MenuBarDisplay::KNOWN_WIRE_VALUES.contains(&display) {
            extra_notes.push(format!(
                "menu_bar.display was {display:?}, which Perch does not recognize; using the default (\"count\")"
            ));
        }
    }

    // Deserialize from the (possibly migrated) document's own text rather
    // than the original `text`, so a future migration step that rewrites
    // `doc` is picked up automatically, without `load` needing to change.
    match toml_edit::de::from_str::<Settings>(&doc.to_string()) {
        Ok(parsed) => {
            let (settings, mut notes) = parsed.validated();
            notes.extend(extra_notes);
            Loaded {
                settings,
                notes,
                error: None,
            }
        }
        Err(e) => defaults_loaded(Some(format!(
            "{}: could not parse settings ({e}); using defaults",
            path.display()
        ))),
    }
}

/// Migrate a parsed settings document whose stored `version` predates
/// `SETTINGS_VERSION`. Mirrors `db::migrate`'s shape: read the version, then
/// run whichever one-shot upgrade(s) bring the document current, never
/// destroying a value the user owns.
///
/// Runs on the `DocumentMut` rather than on a deserialized `Settings`,
/// which is the whole point: a key that has moved section is, to the
/// deserializer, simply absent, and would be replaced by its default before
/// anything got the chance to notice it was ever there. Only the document
/// still knows where the value actually is.
///
/// Ordering assumption a future upgrade step must preserve: this runs on
/// `doc` *before* `load` deserializes it into `Settings` via
/// `toml_edit::de::from_str`. Whatever an upgrade rewrites, it must leave
/// behind a document that deserialization can still parse into `Settings`
/// (every key present or defaultable, every value the expected type) --
/// otherwise a migrated file fails the *next* line down as if it were
/// simply malformed, which is indistinguishable to the user from the
/// migration never having run at all.
///
/// `save` runs this too, on the document it read back from disk. `load`
/// only ever migrates the copy in memory, so without that the *file* would
/// keep its v1 shape forever: the moved key's dead original would sit there
/// looking editable, and a later hand-edit of it would appear to work while
/// doing nothing at all.
fn migrate(doc: &mut DocumentMut, existing_version: i64) {
    // v1 -> v2: `include_background` moves from `[sessions]` to
    // `[notifications]`. It has only ever been read by the notification
    // engine; filing it under sessions was a mistake. Every other key this
    // version of Perch added is *new*, and a new key needs no step here --
    // it is simply missing from an older file and `#[serde(default)]`
    // supplies it. A key that moves is the case that would otherwise lose a
    // value the user chose, in silence.
    if existing_version < 2 {
        move_key(doc, "sessions", "notifications", "include_background");
    }
}

/// Move one key from one top-level section to another, carrying whatever
/// comment sits above it, and leaving nothing behind. Silently does nothing
/// if the source section or the key is absent -- a migration step that has
/// already run, or a file that never held the key, is not an error.
///
/// Removing the source key is the half that is easy to skip and expensive to
/// omit: a dead `[sessions].include_background` left in the file reads as a
/// live setting, so editing it looks like it works and changes nothing.
fn move_key(doc: &mut DocumentMut, from: &str, to: &str, key: &str) {
    // A section may be spelled inline (`sessions = { ... }`) -- the file's
    // header invites hand-editing and both `load` and `ensure_table` already
    // accept that spelling, so the migration has to read one too. What it
    // must not do is carry an inline entry's formatting into a real
    // `[table]`: inline decor is whitespace around a comma, which would come
    // out as a stray indent, and an inline table cannot hold a comment in
    // the first place, so there is nothing there worth preserving.
    let from_was_a_real_table = doc.get(from).is_some_and(Item::is_table);

    let Some(section) = doc.get_mut(from).and_then(Item::as_table_like_mut) else {
        return;
    };
    // Cloned before the removal, because `TableLike::remove` hands back only
    // the value -- the key, and with it the comment written above it, would
    // otherwise be dropped on the floor.
    let formatted_key = from_was_a_real_table
        .then(|| section.get_key_value(key).map(|(k, _)| k.clone()))
        .flatten();
    let Some(mut item) = section.remove(key) else {
        return;
    };

    if !from_was_a_real_table {
        // Let the encoder apply a real table's own spacing rather than the
        // inline one this value was written with.
        if let Some(v) = item.as_value_mut() {
            v.decor_mut().clear();
        }
    }

    let dest = ensure_table(doc, to);
    match formatted_key {
        Some(k) => dest.insert_formatted(&k, item),
        None => dest.insert(key, item),
    };
}

/// A fresh config file: every key at its default, with a header comment so
/// the file documents itself to whoever opens it by hand. `save` starts
/// from this exact text on a first save, then sets every key on top of it
/// the same way it would for an existing file -- a first save is not a
/// special case either.
const TEMPLATE: &str = r#"# Perch's own settings. Perch reads this file on start and watches it for
# changes made by hand -- edit it directly and Perch will pick up the
# change on its own. Comments, key order, and any key Perch itself does not
# recognize (for instance one written by a newer version of Perch) all
# survive Perch saving over this file.

version = 2

[general]
# Start Perch when you log in.
launch_at_login = false
# "" auto-detects Claude Code's directory; set a path to override it.
claude_config_dir = ""

[menu_bar]
# "icon" | "count" | "count-and-waiting"
display = "count"
# "bird" | "binoculars" | "dot" | "bars"
icon = "bird"
# Dim the menu-bar item when the index has not refreshed recently.
dim_when_stale = true
# Minutes without a refresh before the item counts as stale. 1-120.
stale_after_minutes = 5

[sessions]
# How often Perch re-reads Claude Code's directory, in seconds. 1-60.
poll_seconds = 5
# Which terminal "Resume" opens.
preferred_terminal = "Terminal"
# Launch "Resume" with --dangerously-skip-permissions, so Claude Code never
# asks before running a command or editing a file.
resume_bypass_permissions = false

[popover]
# Which sections the menu-bar popover shows.
show_waiting = true
show_working = true
show_recent = true
# How many ended sessions the Recent section lists. 1-20.
recent_limit = 3
# "comfortable" | "compact"
row_density = "comfortable"
# What each session row shows beneath its name.
show_row_folder = false
show_row_usage = true

[projects]
# A project is Active if it was used within this many days. 1-90.
active_within_days = 7
# Show the Archived group.
show_archived = true
# Days covered by the usage chart and every project sparkline. 7, 14, 30 or 90.
chart_days = 14

[usage]
# How many projects the Top Projects list shows. 3-20.
top_projects_count = 8
# Days the Top Projects list covers. 7-180.
top_projects_days = 30
# "off" | "tokens-per-hour" | "cost-per-hour" | "cost-per-day" | "projected-window"
burn_rate = "cost-per-hour"
# Show dollar estimates alongside token counts.
show_cost = true

[notifications]
# Notify when a session has been waiting on you.
waiting_enabled = false
# How long it must have waited first, in minutes. 1-240.
waiting_after_minutes = 10
# Count sessions Claude Code is running in the background as waiting on you.
include_background = false
# Play the default alert sound with the notification.
sound = true
"#;

/// Get (creating if absent) the named top-level section as a real `[name]`
/// table, never an inline one -- `Table`'s own `IndexMut` would otherwise
/// turn a wholly-missing section into `name = { ... }` inline syntax the
/// first time a key inside it is assigned.
fn ensure_table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    let entry = doc.entry(name).or_insert_with(|| Item::Table(Table::new()));

    // The file invites hand-editing, and TOML lets a section be spelled
    // inline (`general = { launch_at_login = true }`). `load` accepts that,
    // so `save` must too: promote it to a real `[general]` table, keeping
    // every key it holds. Anything that is neither -- a scalar left by a
    // broken edit, which `load` will already have reported as an error --
    // becomes an empty table, since there is no key in it to preserve.
    if !entry.is_table() {
        let promoted = std::mem::replace(entry, Item::None)
            .into_table()
            .unwrap_or_default();
        *entry = Item::Table(promoted);
    }

    entry
        .as_table_mut()
        .expect("the branch above just made this a table")
}

/// Format-preserving save. Parses the existing file at `path` (or, if there
/// is none yet, [`TEMPLATE`]) into a `toml_edit::DocumentMut` and assigns
/// only the keys Perch owns, in place -- so a user's comments, their key
/// order, and any key a newer Perch wrote all survive exactly.
///
/// If `path` exists but is not valid TOML, this refuses to save rather than
/// silently replacing it with the template: a format-preserving edit cannot
/// be applied to a document it cannot parse, and guessing would risk
/// throwing away whatever the user has there, broken or not.
pub fn save(path: &Path, s: &Settings) -> Result<()> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(text) => text.parse::<DocumentMut>().with_context(|| {
            format!(
                "{} exists but is not valid TOML; refusing to overwrite it blindly",
                path.display()
            )
        })?,
        // Absent is the ordinary first run: write a fresh file from the
        // template. Anything else — unreadable permissions, an I/O error —
        // gets the same refusal as unparseable TOML above, and for the same
        // reason: replacing the file with the template would discard the
        // user's comments and any key a newer Perch wrote, which is the one
        // thing this function exists to avoid.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TEMPLATE
            .parse::<DocumentMut>()
            .expect("TEMPLATE is valid TOML"),
        Err(e) => {
            return Err(anyhow::anyhow!(e).context(format!(
                "{} could not be read; refusing to overwrite it blindly",
                path.display()
            )))
        }
    };

    // Bring the document current before writing over it. `load` migrates
    // only the copy it deserializes from, so this is the one place a moved
    // key's dead original is actually removed from the file.
    let existing_version = doc.get("version").and_then(Item::as_integer).unwrap_or(0);
    migrate(&mut doc, existing_version);

    doc["version"] = value(SETTINGS_VERSION);

    let general = ensure_table(&mut doc, "general");
    general["launch_at_login"] = value(s.launch_at_login);
    general["claude_config_dir"] = value(s.claude_config_dir.clone());

    let menu_bar = ensure_table(&mut doc, "menu_bar");
    menu_bar["display"] = value(s.menu_bar_display.as_wire_str().to_string());
    menu_bar["icon"] = value(s.menu_bar_icon.as_wire_str().to_string());
    menu_bar["dim_when_stale"] = value(s.dim_when_stale);
    menu_bar["stale_after_minutes"] = value(i64::from(s.stale_after_minutes));

    let sessions = ensure_table(&mut doc, "sessions");
    sessions["poll_seconds"] = value(i64::from(s.poll_seconds));
    sessions["preferred_terminal"] = value(s.preferred_terminal.clone());
    sessions["resume_bypass_permissions"] = value(s.resume_bypass_permissions);

    let popover = ensure_table(&mut doc, "popover");
    popover["show_waiting"] = value(s.show_waiting);
    popover["show_working"] = value(s.show_working);
    popover["show_recent"] = value(s.show_recent);
    popover["recent_limit"] = value(i64::from(s.recent_limit));
    popover["row_density"] = value(s.row_density.as_wire_str().to_string());
    popover["show_row_folder"] = value(s.show_row_folder);
    popover["show_row_usage"] = value(s.show_row_usage);

    let projects = ensure_table(&mut doc, "projects");
    projects["active_within_days"] = value(i64::from(s.active_within_days));
    projects["show_archived"] = value(s.show_archived);
    projects["chart_days"] = value(i64::from(s.chart_days));

    let usage = ensure_table(&mut doc, "usage");
    usage["top_projects_count"] = value(i64::from(s.top_projects_count));
    usage["top_projects_days"] = value(i64::from(s.top_projects_days));
    usage["burn_rate"] = value(s.burn_rate.as_wire_str().to_string());
    usage["show_cost"] = value(s.show_cost);

    let notifications = ensure_table(&mut doc, "notifications");
    notifications["waiting_enabled"] = value(s.waiting_enabled);
    notifications["waiting_after_minutes"] = value(i64::from(s.waiting_after_minutes));
    notifications["include_background"] = value(s.include_background);
    notifications["sound"] = value(s.sound);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, doc.to_string()).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// --- Watching ----------------------------------------------------------

const DEBOUNCE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_secs(5);

enum Cmd {
    Fs,
    Stop,
}

/// A live watch on the settings file, started by [`watch`].
pub struct WatchHandle {
    cmds: mpsc::Sender<Cmd>,
    thread: Option<std::thread::JoinHandle<()>>,
    thread_id: std::thread::ThreadId,
}

impl WatchHandle {
    /// Stop the watch thread and block until it has exited.
    ///
    /// Mirrors `ui::watcher::WatcherHandle::stop`'s self-join guard: if a
    /// caller stashes this handle somewhere its own `on_change` callback can
    /// reach and calls `stop()` from inside it, joining would otherwise be
    /// this thread waiting on itself. The `Stop` command is still sent (and
    /// processed once the callback returns), but the join is skipped in that
    /// case; called from any other thread, `stop()` blocks until the watch
    /// thread has actually exited.
    pub fn stop(mut self) {
        let _ = self.cmds.send(Cmd::Stop);
        if std::thread::current().id() == self.thread_id {
            return;
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WatchHandle {
    /// Safety net for a handle dropped without calling `stop()`: the run
    /// loop holds its own clone of this sender (for the filesystem-watch
    /// callback), so the channel never disconnects on its own -- without
    /// this, the thread would leak forever. `Drop` must not block, so this
    /// only asks the thread to exit; it does not join.
    fn drop(&mut self) {
        let _ = self.cmds.send(Cmd::Stop);
    }
}

/// Watch `path` for changes and call `on_change` (debounced, trailing edge)
/// whenever it does, plus at least once per poll interval as a backstop.
///
/// This watches `path`'s *parent directory*, filtered down to `path`'s own
/// file name, rather than watching `path` itself: many editors (and some
/// filesystems' handling of `save` above) replace a file rather than write
/// into it in place -- write a new file, then rename it over the old one --
/// which drops an inode-level watch on the old file the instant the rename
/// lands. Watching the directory sees the replacement either way, because
/// the rename is itself an event in that directory. Like `ui::watcher`, this
/// never creates the directory it watches: if it is absent, `watch` falls
/// back to polling and keeps retrying every tick.
pub fn watch<F>(path: PathBuf, on_change: F) -> WatchHandle
where
    F: Fn() + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Cmd>();
    let fs_tx = tx.clone();
    let thread = std::thread::spawn(move || run(path, on_change, fs_tx, rx));
    let thread_id = thread.thread().id();
    WatchHandle {
        cmds: tx,
        thread: Some(thread),
        thread_id,
    }
}

fn run<F>(path: PathBuf, on_change: F, fs_tx: mpsc::Sender<Cmd>, rx: mpsc::Receiver<Cmd>)
where
    F: Fn(),
{
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let filename = path.file_name().map(|f| f.to_os_string());

    let mut watcher =
        match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // Filtered to this one filename: the parent directory can (and in
            // production will) hold other files -- `index.db` lives right next
            // to `config.toml` -- so an unfiltered watch would call `on_change`
            // on every unrelated write in that directory.
            let hit = match &res {
                Ok(ev) => ev
                    .paths
                    .iter()
                    .any(|p| p.file_name() == filename.as_deref()),
                Err(_) => true,
            };
            if hit {
                let _ = fs_tx.send(Cmd::Fs);
            }
        }) {
            Ok(w) => Some(w),
            Err(err) => {
                eprintln!("perch: settings watch: failed to create filesystem watcher: {err}");
                None
            }
        };

    // A fresh install has no app-data directory yet. Fall through to a
    // poll-only loop and retry `watch()` on every tick, exactly as
    // `ui::watcher::try_watch` does for the sessions directory. The
    // directory is never created here.
    let mut watching = try_watch(watcher.as_mut(), &dir, POLL, false);

    let mut last = Instant::now();
    // Set when an event arrives inside the debounce dead window and gets
    // dropped: the next wait ends at the debounce boundary instead of the
    // full poll boundary, so that change is not held back until the next
    // poll.
    let mut pending = false;

    loop {
        let wait = if watching && pending {
            DEBOUNCE.saturating_sub(last.elapsed())
        } else {
            POLL
        };
        match rx.recv_timeout(wait) {
            Ok(Cmd::Stop) => return,
            Ok(Cmd::Fs) => {
                if watching && last.elapsed() < DEBOUNCE {
                    pending = true;
                } else {
                    on_change();
                    last = Instant::now();
                    pending = false;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Mirrors `ui::watcher`'s backstop tick: fire unconditionally,
                // whether this is flushing a debounced change or the plain
                // poll interval elapsing with nothing pending. Unlike that
                // watcher, nothing here can change invisibly -- a settings
                // file only ever changes via a write, which is always a
                // filesystem event -- but `on_change` is cheap and idempotent
                // (parsing one small file), so an occasional redundant call
                // every `POLL` costs nothing, and keeping the identical
                // unconditional-fire shape here is less to get subtly wrong
                // than inventing a "was anything actually pending" branch.
                on_change();
                last = Instant::now();
                pending = false;
                if !watching {
                    watching = try_watch(watcher.as_mut(), &dir, POLL, true);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Never creates the directory: if it is absent, `watch` fails and we poll
/// instead. `is_retry` distinguishes the initial attempt (logs failure once,
/// with the poll interval) from a later poll-tick retry (silent on repeated
/// failure; logs once, on success, the promotion back onto the filesystem
/// watch) -- same split as `ui::watcher::try_watch`.
fn try_watch(
    watcher: Option<&mut notify::RecommendedWatcher>,
    dir: &Path,
    poll: Duration,
    is_retry: bool,
) -> bool {
    let Some(w) = watcher else { return false };
    match w.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => {
            if is_retry {
                eprintln!(
                    "perch: settings watch: now watching {} (was polling)",
                    dir.display()
                );
            }
            true
        }
        Err(err) => {
            if !is_retry {
                eprintln!(
                    "perch: settings watch: failed to watch {}: {err} -- polling every {poll:?} and retrying",
                    dir.display()
                );
            }
            false
        }
    }
}

/// Shared by every `perch-core` test that touches a process-global env var
/// (`PERCH_CONFIG`, `PERCH_DATA_DIR` here; `CLAUDE_CONFIG_DIR`,
/// `XDG_CONFIG_HOME`, `HOME` in `ui::diagnostics`) -- `cargo test` runs a
/// crate's tests in parallel threads within one process, so any two such
/// tests race, genuinely, not just in theory, unless they take this lock
/// first. One lock for the whole crate, not one per module, so a test here
/// and a test in `ui::diagnostics` can't race past each other either.
/// `perch-ffi` guards its own `PERCH_DATA_DIR` tests the same way.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{BurnRate, MenuBarIcon, RowDensity};

    #[test]
    fn a_missing_file_loads_defaults_without_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let l = load(&tmp.path().join("nope.toml"));
        assert_eq!(l.settings.poll_seconds, 5);
        assert!(
            l.error.is_none(),
            "absence is not a failure — it is a first run"
        );
    }

    #[test]
    fn a_malformed_file_yields_defaults_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, "this is not = = toml").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.poll_seconds, 5, "the app still runs");
        let err = l
            .error
            .expect("a hand-edited file that did not parse must say so");
        assert!(
            err.contains("config.toml") || err.contains("parse"),
            "actionable: {err}"
        );
    }

    #[test]
    fn a_file_that_exists_but_cannot_be_read_yields_defaults_and_says_so() {
        // `chmod 000` is not a portable way to trigger a read failure in a
        // test environment (a root-equivalent test runner can read it
        // anyway), so this uses an equally valid trigger the brief allows:
        // a directory sitting where the settings file is expected.
        // `fs::read_to_string` on a directory fails with a kind other than
        // `NotFound` -- exactly the "present but unreadable" case that must
        // not be confused with a first run.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::create_dir(&p).unwrap();
        let l = load(&p);
        assert_eq!(l.settings.poll_seconds, 5, "the app still runs");
        let err = l.error.expect(
            "present but unreadable must not be silent like a missing file — it must say so",
        );
        assert!(
            err.contains("config.toml") || err.contains("read"),
            "actionable: {err}"
        );
    }

    #[test]
    fn out_of_range_values_load_clamped_with_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, "[sessions]\npoll_seconds = 0\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.poll_seconds, 1);
        assert_eq!(l.notes.len(), 1);
    }

    #[test]
    fn saving_preserves_comments_key_order_and_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(
            &p,
            concat!(
                "# my own note about this file\n",
                "version = 1\n",
                "\n",
                "[sessions]\n",
                "# I like a slow poll\n",
                "poll_seconds = 30\n",
                "something_a_newer_perch_added = true\n",
            ),
        )
        .unwrap();

        let mut s = load(&p).settings;
        s.poll_seconds = 12;
        save(&p, &s).unwrap();

        let after = std::fs::read_to_string(&p).unwrap();
        assert!(
            after.contains("# my own note about this file"),
            "comments survive"
        );
        assert!(
            after.contains("# I like a slow poll"),
            "inline comments survive"
        );
        assert!(
            after.contains("something_a_newer_perch_added = true"),
            "a key this binary does not know must not be eaten"
        );
        assert!(after.contains("poll_seconds = 12"), "and the change lands");
    }

    #[test]
    fn a_created_file_documents_itself_and_reloads_identically() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        save(&p, &Settings::default()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.starts_with("#"),
            "the file opens with a header comment"
        );
        assert!(
            text.contains("poll_seconds"),
            "every key is present, not just non-defaults"
        );
        assert_eq!(
            load(&p).settings,
            Settings::default(),
            "round trip is exact"
        );
    }

    #[test]
    fn perch_config_overrides_the_default_path() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let want = tmp.path().join("elsewhere.toml");
        std::env::set_var("PERCH_CONFIG", &want);
        let got = config_path().unwrap();
        std::env::remove_var("PERCH_CONFIG");
        assert_eq!(got, want);
    }

    #[test]
    fn app_data_dir_honours_perch_data_dir() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("PERCH_DATA_DIR", tmp.path());
        let got = app_data_dir().unwrap();
        std::env::remove_var("PERCH_DATA_DIR");
        assert_eq!(got, tmp.path());
    }

    #[test]
    fn an_unrecognized_menu_bar_display_is_reported_not_silently_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, "[menu_bar]\ndisplay = \"blinking\"\n").unwrap();
        let l = load(&p);
        assert_eq!(
            l.settings.menu_bar_display,
            MenuBarDisplay::Count,
            "an unrecognized value still degrades to the default so the app runs"
        );
        assert!(
            l.error.is_none(),
            "an unrecognized enum value is not the same failure as a parse error"
        );
        assert_eq!(
            l.notes.len(),
            1,
            "but unlike a recognized default, it must not be silent -- one note"
        );
        assert!(l.notes[0].contains("menu_bar.display"));
        assert!(l.notes[0].contains("blinking"));
    }

    #[test]
    fn a_recognized_menu_bar_display_earns_no_note() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        std::fs::write(&p, "[menu_bar]\ndisplay = \"icon\"\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.menu_bar_display, MenuBarDisplay::Icon);
        assert!(l.notes.is_empty());
    }

    fn write_config(p: &Path, poll_seconds: u32) {
        std::fs::write(p, format!("[sessions]\npoll_seconds = {poll_seconds}\n")).unwrap();
    }

    #[test]
    fn watch_detects_a_modification() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        write_config(&p, 5);

        let (tx, rx) = mpsc::channel::<()>();
        let h = watch(p.clone(), move || {
            let _ = tx.send(());
        });

        // Exactly when the watch thread's `try_watch` actually attaches is
        // not observable from here, so rather than a single write raced
        // against that, nudge with repeated writes (each a real, distinct
        // change) until one is seen -- deterministic once the watch is
        // live, same technique `ui::watcher`'s own promotion test uses.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut poll_seconds = 6;
        let mut seen = false;
        while Instant::now() < deadline {
            write_config(&p, poll_seconds);
            poll_seconds += 1;
            if rx.recv_timeout(Duration::from_millis(150)).is_ok() {
                seen = true;
                break;
            }
        }
        assert!(seen, "a modification in place must surface");
        h.stop();
    }

    #[test]
    fn watch_survives_the_file_being_replaced_rather_than_modified_in_place() {
        // Mimics an editor's atomic save: write to a sibling file, then
        // rename it over the target. This replaces the target's inode
        // entirely -- an inode-level watch on the file itself would be
        // silently orphaned by the rename and see nothing further, which is
        // exactly why `watch` watches the parent directory instead.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        write_config(&p, 5);

        let (tx, rx) = mpsc::channel::<()>();
        let h = watch(p.clone(), move || {
            let _ = tx.send(());
        });

        // Same nudge-until-seen technique as above, and for the same
        // reason: the write-then-rename below must land after the watch
        // thread has actually attached, which is not observable from here.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut poll_seconds = 7;
        let mut seen = false;
        while Instant::now() < deadline {
            let replacement = tmp.path().join(format!("config.toml.tmp{poll_seconds}"));
            write_config(&replacement, poll_seconds);
            std::fs::rename(&replacement, &p).unwrap();
            poll_seconds += 1;
            if rx.recv_timeout(Duration::from_millis(150)).is_ok() {
                seen = true;
                break;
            }
        }
        assert!(
            seen,
            "a replace-via-rename must surface exactly like an in-place write"
        );
        h.stop();
    }

    #[test]
    fn watch_never_creates_the_directory_it_watches() {
        let tmp = tempfile::tempdir().unwrap();
        let missing_dir = tmp.path().join("not-yet-created");
        let p = missing_dir.join("config.toml");

        let h = watch(p, || {});
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !missing_dir.exists(),
            "watch must never create the directory it watches"
        );
        h.stop();
    }

    /// The compiler already catches an *omitted* field -- both `From` impls
    /// in `settings::mod` build struct literals, so a field with no wire
    /// mapping fails the build. What it cannot catch is a field wired to
    /// the *wrong* key, because so many of these fields share a type. Hence
    /// a distinct value per field: a test that reused one value would pass
    /// with two same-typed fields swapped.
    #[test]
    fn every_field_survives_a_save_and_load_at_a_distinct_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let want = Settings {
            launch_at_login: true,
            claude_config_dir: dir.path().to_string_lossy().into_owned(),
            menu_bar_display: MenuBarDisplay::CountAndWaiting,
            menu_bar_icon: MenuBarIcon::Binoculars,
            dim_when_stale: false,
            stale_after_minutes: 11,
            poll_seconds: 12,
            preferred_terminal: "iTerm2".to_string(),
            resume_bypass_permissions: true,
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

    /// Reads a top-level section's key out of saved text, whichever way the
    /// section is spelled. Structural, rather than a substring match, so this
    /// says what it means: the key is *gone*, not merely re-ordered.
    fn key_of(text: &str, section: &str, key: &str) -> Option<toml_edit::Item> {
        let doc: DocumentMut = text.parse().expect("saved text parses");
        doc.get(section)
            .and_then(Item::as_table_like)
            .and_then(|t| t.get(key))
            .cloned()
    }

    /// `include_background` is the one key this milestone *moves* rather than
    /// adds. Every other new key is absent from a v1 file and picks up its
    /// default; this one already holds a value the user chose, and a section
    /// change that lost it would be silent.
    #[test]
    fn v1_carries_include_background_into_the_notifications_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            concat!(
                "# my own note\n",
                "version = 1\n",
                "\n",
                "[sessions]\n",
                "poll_seconds = 9\n",
                "include_background = true\n",
                "something_a_newer_perch_added = true\n",
            ),
        )
        .unwrap();

        let loaded = load(&path);
        assert!(
            loaded.settings.include_background,
            "the value must survive the move"
        );
        assert_eq!(
            loaded.settings.poll_seconds, 9,
            "its neighbours are untouched"
        );

        save(&path, &loaded.settings).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# my own note"),
            "comments survive a migrating save"
        );
        assert!(
            text.contains("something_a_newer_perch_added = true"),
            "a key this binary does not own survives a migrating save"
        );
        assert!(
            !text.contains("[sessions]\npoll_seconds = 9\ninclude_background"),
            "the key no longer lives under [sessions]"
        );
        assert!(
            key_of(&text, "sessions", "include_background").is_none(),
            "the dead key is removed, so a later hand-edit of it cannot appear to work"
        );
        assert_eq!(
            key_of(&text, "notifications", "include_background")
                .as_ref()
                .and_then(|i| i.as_bool()),
            Some(true),
            "it lands under the section that reads it, at the value the user chose"
        );

        let again = load(&path);
        assert!(
            again.settings.include_background,
            "and it is still there on the next read"
        );
        assert_eq!(again.settings.poll_seconds, 9, "as is everything beside it");
    }

    /// TOML lets a section be spelled inline, and the file's own header
    /// invites hand-editing -- `a_section_written_inline_can_still_be_saved`
    /// already proves `save` copes. The migration reads and writes the same
    /// document, so it has to cope with the same spelling.
    #[test]
    fn a_v1_file_whose_sessions_section_is_inline_still_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "version = 1\nsessions = { poll_seconds = 9, include_background = true }\n",
        )
        .unwrap();

        let loaded = load(&path);
        assert!(
            loaded.error.is_none(),
            "reload reported: {:?}",
            loaded.error
        );
        assert!(
            loaded.settings.include_background,
            "an inline [sessions] holds the value just as a real table does"
        );
        assert_eq!(loaded.settings.poll_seconds, 9);

        save(&path, &loaded.settings).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            key_of(&text, "sessions", "include_background").is_none(),
            "the dead key is removed from an inline section too"
        );
        assert_eq!(
            key_of(&text, "notifications", "include_background")
                .as_ref()
                .and_then(|i| i.as_bool()),
            Some(true),
        );
    }

    /// A file already at version 2 is current: the migration must not run
    /// again. The case that would hurt is a leftover `[sessions]` copy -- a
    /// re-run would move it over the real value and silently undo whatever
    /// the user last set.
    #[test]
    fn a_v2_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            concat!(
                "version = 2\n",
                "\n",
                "[sessions]\n",
                "poll_seconds = 9\n",
                "include_background = true\n",
                "\n",
                "[notifications]\n",
                "include_background = false\n",
            ),
        )
        .unwrap();

        let loaded = load(&path);
        assert!(
            !loaded.settings.include_background,
            "the [notifications] value is the live one; a stale [sessions] copy must not overwrite it"
        );

        save(&path, &loaded.settings).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            key_of(&text, "notifications", "include_background")
                .as_ref()
                .and_then(|i| i.as_bool()),
            Some(false),
            "and saving does not re-migrate it either"
        );
        assert!(
            !load(&path).settings.include_background,
            "still false after a round trip"
        );
    }
}

#[cfg(test)]
mod inline_table_tests {
    use super::{load, save};
    use tempfile::tempdir;

    /// A user is invited by the file's own header to "edit freely", and TOML
    /// lets them write a section as an inline table. `load` accepts that
    /// spelling, so `save` must too -- it is the same document, not a
    /// malformed one.
    #[test]
    fn a_section_written_inline_can_still_be_saved() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "version = 1\ngeneral = { launch_at_login = true }\n").unwrap();

        let loaded = load(&path);
        assert!(
            loaded.error.is_none(),
            "inline table should load: {:?}",
            loaded.error
        );
        assert!(
            loaded.settings.launch_at_login,
            "the inline value should survive load"
        );

        save(&path, &loaded.settings).expect("saving a file with an inline section must not fail");

        let again = load(&path);
        assert!(
            again.settings.launch_at_login,
            "the value must survive the round trip"
        );
    }
}

#[cfg(test)]
mod unreadable_save_tests {
    use super::{load, save};
    use crate::settings::Settings;
    use tempfile::tempdir;

    /// `save` refuses to overwrite a file it cannot parse, precisely so a
    /// user's comments and unknown keys are never thrown away. A file it
    /// cannot *read* deserves the same refusal: falling back to the template
    /// discards exactly the same content, just without the parse error to
    /// explain it.
    #[test]
    fn an_unreadable_file_is_refused_rather_than_replaced() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# a comment worth keeping\nversion = 2\n").unwrap();

        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o222); // write-only
        std::fs::set_permissions(&path, perms).unwrap();

        let err = save(&path, &Settings::default()).unwrap_err();
        assert!(
            err.to_string().contains("could not be read"),
            "the refusal must say why: {err}"
        );

        // Restore read permission and prove nothing was lost.
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o644);
        std::fs::set_permissions(&path, perms).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# a comment worth keeping"));
    }

    /// A file that is simply absent is still the ordinary first-run case and
    /// must still be created from the template.
    #[test]
    fn a_missing_file_is_still_created_from_the_template() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &Settings::default()).expect("first run writes a fresh file");
        assert!(load(&path).error.is_none());
    }
}
