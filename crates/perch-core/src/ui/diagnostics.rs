//! The diagnostics pane's view-model. It answers exactly one question — "why
//! isn't my session showing up?" — by spelling out, per session record, the
//! spec §7 verdict `live::live_sessions` reached for it: not just a boolean,
//! but the concrete detail (a pid, an actual executable name) that makes the
//! verdict actionable. It shares `live::decide_record` with the session list
//! itself rather than re-deciding liveness a second way, so the two can
//! never quietly disagree.

use crate::db::Db;
use crate::live::{self, RecordDecision};
use crate::platform::ProcessProbe;
use crate::settings::store::load as load_settings;
use crate::ui::format::elapsed_or_dash;
use std::path::Path;

/// A missing value that would otherwise have to be fabricated (a zero count,
/// an invented timestamp) — the one placeholder this module ever shows, and
/// only ever in place of something Perch genuinely does not know.
const DASH: &str = "—";

#[derive(Debug, Clone, PartialEq)]
pub enum RecordVerdict {
    Accepted,
    NoSuchProcess,
    NotClaude { actual: String },
    Unparsable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordRow {
    pub file: String,
    pub pid: i32,
    pub session_id: String,
    pub verdict: RecordVerdict,
    pub verdict_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticsModel {
    pub config_dir: String,
    /// "CLAUDE_CONFIG_DIR" | "XDG" | "default" | "setting"
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

/// The finished sentence a confused person reads next to one session
/// record — not a debug label. Every branch names the pid; `NotClaude` also
/// names what the process actually is, because "not claude" alone leaves the
/// reader no wiser about what they're looking at.
///
/// `unreadable` distinguishes the two ways a record can end up `Unparsable`:
/// the file itself could not be opened (permissions, or it vanished between
/// listing and reading), versus a file that opened fine but whose content
/// is not JSON `parse_session_record` understands. These point a confused
/// user at two different problems — a filesystem issue versus a corrupt or
/// unexpected record — so collapsing them into one message would send them
/// to debug the wrong one.
fn verdict_label(pid: i32, verdict: &RecordVerdict, unreadable: bool) -> String {
    match verdict {
        RecordVerdict::Accepted => format!("showing: pid {pid} is running claude"),
        RecordVerdict::NoSuchProcess => format!("ignored: pid {pid} is not running"),
        RecordVerdict::NotClaude { actual } => {
            format!("ignored: pid {pid} is `{actual}`, not `claude`")
        }
        RecordVerdict::Unparsable if unreadable => {
            "ignored: this record could not be read".to_string()
        }
        RecordVerdict::Unparsable => "ignored: this record's JSON could not be parsed".to_string(),
    }
}

/// Which branch of `config::resolve_config_dir`'s precedence actually
/// produced a directory, plus the settings override that beats all of them.
/// Pure and independent of the real process environment so every branch is
/// testable directly, exactly like `config::resolve_config_dir` itself —
/// which this mirrors so the two can never silently name a different
/// winner than the one that actually resolved.
fn config_dir_source(
    setting_override: &str,
    claude_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> &'static str {
    fn non_empty(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    if non_empty(Some(setting_override)).is_some() {
        return "setting";
    }
    if non_empty(claude_config_dir).is_some() {
        return "CLAUDE_CONFIG_DIR";
    }
    if non_empty(xdg_config_home).is_some() {
        return "XDG";
    }
    // Nothing overrides it: `config::resolve_config_dir` falls through to
    // `$HOME/.claude` when `home` is set, and has nothing left to resolve to
    // at all when it isn't — either way this is the fallback branch, not one
    // of the three explicit sources above.
    let _ = home;
    "default"
}

fn count_or_dash(count: anyhow::Result<i64>) -> String {
    match count {
        Ok(n) => n.to_string(),
        Err(_) => DASH.to_string(),
    }
}

/// Build one row per session record file, explaining `live::decide_record`'s
/// verdict for it. A record that fails to even be read from disk gets its
/// own `unreadable` label rather than being folded into "could not be
/// parsed" — see [`verdict_label`] for why that distinction matters.
fn record_row(path: &std::path::Path, probe: &dyn ProcessProbe) -> RecordRow {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            let verdict = RecordVerdict::Unparsable;
            let verdict_label = verdict_label(0, &verdict, true);
            return RecordRow {
                file,
                pid: 0,
                session_id: String::new(),
                verdict,
                verdict_label,
            };
        }
    };

    let (pid, session_id, verdict) = match live::decide_record(&text, probe) {
        RecordDecision::Accepted(session) => {
            (session.pid, session.session_id, RecordVerdict::Accepted)
        }
        RecordDecision::NoSuchProcess { pid, session_id } => {
            (pid, session_id, RecordVerdict::NoSuchProcess)
        }
        RecordDecision::NotClaude {
            pid,
            session_id,
            actual,
        } => (pid, session_id, RecordVerdict::NotClaude { actual }),
        // The file read fine but its JSON didn't parse, so there is no pid
        // or session id to report — 0 / "" here mirror the same
        // absent-numeric/absent-string sentinels `parse_session_record`'s
        // own helpers already use.
        RecordDecision::Unparsable => (0, String::new(), RecordVerdict::Unparsable),
    };

    let verdict_label = verdict_label(pid, &verdict, false);
    RecordRow {
        file,
        pid,
        session_id,
        verdict,
        verdict_label,
    }
}

/// Everything the diagnostics pane needs: where Perch thinks the Claude Code
/// directory is and why, every session record it found there with a
/// spelled-out verdict, and the state of Perch's own settings file and index.
/// Read-only throughout — this never writes anything, including to the
/// Claude Code directory it inspects.
pub fn build_diagnostics(
    db: Option<&Db>,
    config_dir: &Path,
    settings_path: &Path,
    probe: &dyn ProcessProbe,
    now_ms: i64,
) -> DiagnosticsModel {
    let loaded = load_settings(settings_path);

    let claude_env = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let xdg_env = std::env::var("XDG_CONFIG_HOME").ok();
    let home_env = std::env::var("HOME").ok();
    let source = config_dir_source(
        &loaded.settings.claude_config_dir,
        claude_env.as_deref(),
        xdg_env.as_deref(),
        home_env.as_deref(),
    );

    let sessions_dir = crate::config::sessions_dir(config_dir);
    let records = live::record_files(&sessions_dir)
        .into_iter()
        .map(|p| record_row(&p, probe))
        .collect();

    // `Connection::path` returns `Some("")` for an in-memory or temporary
    // database rather than `None` — filter that out so it reports the same
    // absence as no database at all.
    let index_path = db
        .and_then(|d| d.conn().path())
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| DASH.to_string());
    let index_sessions = db.map_or_else(|| DASH.to_string(), |d| count_or_dash(d.session_count()));
    let index_turns = db.map_or_else(|| DASH.to_string(), |d| count_or_dash(d.turn_count()));
    let last_indexed = db
        .and_then(|d| d.last_turn_ts().ok())
        .flatten()
        .map_or_else(|| DASH.to_string(), |ts| elapsed_or_dash(now_ms, ts));

    DiagnosticsModel {
        config_dir: config_dir.display().to_string(),
        config_dir_source: source.to_string(),
        sessions_dir: sessions_dir.display().to_string(),
        records,
        settings_path: settings_path.display().to_string(),
        settings_loaded: loaded.error.is_none(),
        index_path,
        index_sessions,
        index_turns,
        last_indexed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeProbe {
        alive: std::collections::HashSet<i32>,
        names: HashMap<i32, String>,
    }

    impl FakeProbe {
        fn new(alive: &[i32]) -> Self {
            Self {
                alive: alive.iter().copied().collect(),
                names: HashMap::new(),
            }
        }
        fn with_name(mut self, pid: i32, name: &str) -> Self {
            self.names.insert(pid, name.to_string());
            self
        }
    }

    impl ProcessProbe for FakeProbe {
        fn is_alive(&self, pid: i32) -> bool {
            self.alive.contains(&pid)
        }
        fn process_name(&self, pid: i32) -> Option<String> {
            self.names.get(&pid).cloned()
        }
    }

    fn write_record(dir: &Path, pid: i32, session_id: &str) {
        let json = format!(
            r#"{{"pid":{pid},"sessionId":"{session_id}","cwd":"/tmp/p","name":"n","kind":"interactive","status":"busy","startedAt":1,"statusUpdatedAt":2}}"#
        );
        std::fs::write(dir.join(format!("{pid}.json")), json).unwrap();
    }

    fn diagnostics_for(
        sessions_dir: &Path,
        probe: &dyn ProcessProbe,
        settings_path: &Path,
    ) -> DiagnosticsModel {
        // `config_dir` need only be a directory whose `sessions` subpath is
        // `sessions_dir` — `crate::config::sessions_dir` joins "sessions" on
        // to whatever it's given, so pass the parent back out here.
        build_diagnostics(
            None,
            sessions_dir.parent().unwrap(),
            settings_path,
            probe,
            0,
        )
    }

    fn sessions_dir_under(tmp: &tempfile::TempDir) -> std::path::PathBuf {
        let dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_accepted_record_is_shown_not_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        write_record(&sessions_dir, 4821, "s1");
        let probe = FakeProbe::new(&[4821]).with_name(4821, "claude");
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.records.len(), 1);
        let row = &model.records[0];
        assert_eq!(row.pid, 4821);
        assert_eq!(row.session_id, "s1");
        assert_eq!(row.verdict, RecordVerdict::Accepted);
        assert!(row.verdict_label.contains("4821"));
        assert!(row.verdict_label.contains("claude"));
        assert!(
            !row.verdict_label.contains("ignored"),
            "an accepted record must not read like a rejection: {}",
            row.verdict_label
        );
    }

    #[test]
    fn a_dead_pid_is_reported_as_no_such_process() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        write_record(&sessions_dir, 4822, "s2");
        let probe = FakeProbe::new(&[]); // nothing alive
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        let row = &model.records[0];
        assert_eq!(row.verdict, RecordVerdict::NoSuchProcess);
        assert!(row.verdict_label.contains("4822"));
        assert!(row.verdict_label.contains("ignored"));
    }

    #[test]
    fn a_recycled_pid_running_something_else_names_it() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        write_record(&sessions_dir, 4823, "s3");
        let probe = FakeProbe::new(&[4823]).with_name(4823, "zsh");
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        let row = &model.records[0];
        assert_eq!(
            row.verdict,
            RecordVerdict::NotClaude {
                actual: "zsh".to_string()
            }
        );
        assert_eq!(
            row.verdict_label, "ignored: pid 4823 is `zsh`, not `claude`",
            "must name the pid and what it actually is, not just that it isn't claude"
        );
    }

    #[test]
    fn an_unparsable_record_is_ignored_without_a_fabricated_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        std::fs::write(sessions_dir.join("9999.json"), "{not json").unwrap();
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        let row = &model.records[0];
        assert_eq!(row.verdict, RecordVerdict::Unparsable);
        assert_eq!(row.pid, 0);
        assert_eq!(row.session_id, "");
        assert!(row.verdict_label.contains("ignored"));
        assert!(
            row.verdict_label.contains("parsed"),
            "a malformed record must say *parsed*, not *read*: {}",
            row.verdict_label
        );
        assert!(!row.verdict_label.contains("could not be read"));
        assert_eq!(row.file, "9999.json");
    }

    #[test]
    fn a_record_that_cannot_be_read_is_distinguished_from_one_that_wont_parse() {
        // `record_files` only ever returns paths that were a regular file at
        // listing time, so the only realistic way `record_row`'s own read
        // fails is a race (the file vanishes or a permission changes between
        // listing and reading) -- not reproducible deterministically without
        // `chmod`, which a root-equivalent test runner can bypass anyway (see
        // `settings::store`'s own note on this). A directory standing in for
        // the file's path makes `read_to_string` fail the exact same way
        // (an I/O error, not a parse error) without either problem, so this
        // calls `record_row` directly rather than going through
        // `build_diagnostics` and `record_files`.
        let tmp = tempfile::tempdir().unwrap();
        let unreadable = tmp.path().join("4824.json");
        std::fs::create_dir(&unreadable).unwrap();
        let probe = FakeProbe::new(&[]);

        let row = record_row(&unreadable, &probe);

        assert_eq!(row.verdict, RecordVerdict::Unparsable);
        assert_eq!(row.pid, 0);
        assert_eq!(row.session_id, "");
        assert!(
            row.verdict_label.contains("could not be read"),
            "an I/O failure must say *read*, not *parsed*: {}",
            row.verdict_label
        );
        assert!(!row.verdict_label.contains("parsed"));
    }

    #[test]
    fn an_empty_sessions_directory_yields_no_records_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert!(model.records.is_empty());
    }

    #[test]
    fn a_missing_sessions_directory_also_yields_no_records() {
        let tmp = tempfile::tempdir().unwrap();
        // Note: not created — `config::sessions_dir`'s target need not exist.
        let sessions_dir = tmp.path().join("sessions");
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert!(model.records.is_empty());
    }

    #[test]
    fn reports_the_sessions_and_config_dir_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.config_dir, tmp.path().display().to_string());
        assert_eq!(model.sessions_dir, sessions_dir.display().to_string());
        assert_eq!(model.settings_path, settings_path.display().to_string());
    }

    #[test]
    fn no_db_yields_dashes_not_fabricated_zeros() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.index_path, DASH);
        assert_eq!(model.index_sessions, DASH);
        assert_eq!(model.index_turns, DASH);
        assert_eq!(model.last_indexed, DASH);
    }

    #[test]
    fn an_in_memory_db_with_nothing_indexed_still_shows_real_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");
        let db = crate::db::open_in_memory().unwrap();

        let model = build_diagnostics(
            Some(&db),
            sessions_dir.parent().unwrap(),
            &settings_path,
            &probe,
            0,
        );

        assert_eq!(model.index_sessions, "0", "a real zero, honestly queried");
        assert_eq!(model.index_turns, "0");
        assert_eq!(
            model.last_indexed, DASH,
            "nothing indexed means no timestamp to show, not an invented one"
        );
        assert_eq!(
            model.index_path, DASH,
            "an in-memory db has no path to report"
        );
    }

    #[test]
    fn settings_loaded_is_false_only_when_the_file_fails_to_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);

        let missing = tmp.path().join("config.toml");
        let model = diagnostics_for(&sessions_dir, &probe, &missing);
        assert!(model.settings_loaded, "absence is a first run, not a fault");

        let broken = tmp.path().join("broken.toml");
        std::fs::write(&broken, "not = = toml").unwrap();
        let model = diagnostics_for(&sessions_dir, &probe, &broken);
        assert!(!model.settings_loaded);
    }

    // --- config_dir_source precedence -----------------------------------

    #[test]
    fn source_prefers_the_settings_override_over_everything() {
        assert_eq!(
            config_dir_source("/custom", Some("/env"), Some("/xdg"), Some("/home")),
            "setting"
        );
    }

    #[test]
    fn source_falls_back_to_claude_config_dir_env() {
        assert_eq!(
            config_dir_source("", Some("/env"), Some("/xdg"), Some("/home")),
            "CLAUDE_CONFIG_DIR"
        );
    }

    #[test]
    fn source_falls_back_to_xdg_when_no_override_or_env() {
        assert_eq!(
            config_dir_source("", None, Some("/xdg"), Some("/home")),
            "XDG"
        );
    }

    #[test]
    fn source_is_default_when_nothing_is_set() {
        assert_eq!(config_dir_source("", None, None, Some("/home")), "default");
        assert_eq!(config_dir_source("", None, None, None), "default");
    }

    #[test]
    fn source_ignores_blank_not_just_absent_values() {
        assert_eq!(
            config_dir_source("   ", Some(""), Some(""), Some("/home")),
            "default"
        );
    }

    #[test]
    fn build_diagnostics_reports_the_setting_source_regardless_of_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml");
        std::fs::write(
            &settings_path,
            "[general]\nclaude_config_dir = \"/wherever\"\n",
        )
        .unwrap();

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.config_dir_source, "setting");
    }

    // The three tests below are the only ones in this module that touch the
    // real process environment, so — like `settings::store`'s own
    // `PERCH_CONFIG`/`PERCH_DATA_DIR` tests — they take `ENV_LOCK` first:
    // `cargo test` runs a crate's tests in parallel threads within one
    // process, and any two tests mutating `CLAUDE_CONFIG_DIR` or
    // `XDG_CONFIG_HOME` concurrently would race for real, not just in
    // theory. Reusing the crate-wide lock (rather than a second one here)
    // means a test in `settings::store` and a test here can't race past
    // each other either. Each restores whatever was there before it, so a
    // developer's own shell environment isn't left altered.
    use crate::settings::store::ENV_LOCK;

    struct EnvVarGuard {
        key: &'static str,
        prior: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prior }
        }

        fn unset(key: &'static str) -> Self {
            let prior = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prior }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn build_diagnostics_reports_claude_config_dir_when_the_env_var_is_set() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _claude = EnvVarGuard::set("CLAUDE_CONFIG_DIR", "/env/claude");
        let _xdg = EnvVarGuard::unset("XDG_CONFIG_HOME");

        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml"); // no override

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.config_dir_source, "CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn build_diagnostics_reports_xdg_when_only_it_is_set() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _claude = EnvVarGuard::unset("CLAUDE_CONFIG_DIR");
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", "/env/xdg");

        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml"); // no override

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.config_dir_source, "XDG");
    }

    #[test]
    fn build_diagnostics_reports_default_when_neither_env_var_is_set() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _claude = EnvVarGuard::unset("CLAUDE_CONFIG_DIR");
        let _xdg = EnvVarGuard::unset("XDG_CONFIG_HOME");

        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = sessions_dir_under(&tmp);
        let probe = FakeProbe::new(&[]);
        let settings_path = tmp.path().join("config.toml"); // no override

        let model = diagnostics_for(&sessions_dir, &probe, &settings_path);

        assert_eq!(model.config_dir_source, "default");
    }
}
