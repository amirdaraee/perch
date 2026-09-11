//! The advanced pane's view-model: where Perch keeps its own two files, what
//! is in the index, and what the app itself is.
//!
//! Every value is a finished string, including the sizes and the ages — the
//! pane draws what it is given and formats nothing. A fact Perch cannot
//! establish (an index that will not open, a file whose size the filesystem
//! will not report) is a dash rather than a zero, the same rule the
//! diagnostics pane follows: a fabricated zero here reads as "your index is
//! empty", which is a different and much more alarming claim than "Perch
//! could not look".

use crate::db::Db;
use crate::ui::format::{human_bytes, human_elapsed};
use std::path::Path;

/// The one placeholder this module shows, and only in place of something
/// Perch genuinely does not know.
const DASH: &str = "—";

/// One read-only fact. `reveal_path` is set only where the value names a
/// real file or folder on this machine, so a shell offers Reveal exactly
/// where Reveal would land somewhere.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FactRow {
    pub label: String,
    pub value: String,
    pub reveal_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FactGroup {
    pub heading: String,
    pub rows: Vec<FactRow>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LinkRow {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AdvancedModel {
    pub groups: Vec<FactGroup>,
    pub links: Vec<LinkRow>,
    /// What "Reindex now" actually does, said before it is pressed.
    pub reindex_help: String,
    /// Shown in the confirmation before every setting goes back to its
    /// factory value. Names the file it will rewrite, because that file is
    /// one the user may have hand-edited.
    pub reset_warning: String,
}

/// The whole pane. `db` is `None` when the index will not open — every
/// index-derived row is then a dash and the rest of the pane still renders,
/// because someone whose index is broken is exactly who needs the paths and
/// the reindex button in front of them.
pub fn build_advanced(
    db: Option<&Db>,
    config_path: &Path,
    db_path: &Path,
    claude_dir: &Path,
    now_ms: i64,
) -> AdvancedModel {
    let files = FactGroup {
        heading: "Files".to_string(),
        rows: vec![
            path_row("Settings file", config_path),
            path_row("Claude Code folder", claude_dir),
        ],
    };

    let index = FactGroup {
        heading: "Index".to_string(),
        rows: vec![
            path_row("Index file", db_path),
            FactRow {
                label: "Index size".to_string(),
                value: file_size(db_path),
                reveal_path: None,
            },
            FactRow {
                label: "Sessions indexed".to_string(),
                value: count_or_dash(db.map(|d| d.session_count())),
                reveal_path: None,
            },
            FactRow {
                label: "Turns indexed".to_string(),
                value: count_or_dash(db.map(|d| d.turn_count())),
                reveal_path: None,
            },
            FactRow {
                label: "Index last written".to_string(),
                value: modified_ago(db_path, now_ms),
                reveal_path: None,
            },
            FactRow {
                label: "Newest session activity".to_string(),
                value: db
                    .and_then(|d| d.last_turn_ts().ok())
                    .flatten()
                    .map_or_else(|| DASH.to_string(), |ts| ago(now_ms, ts)),
                reveal_path: None,
            },
        ],
    };

    let about = FactGroup {
        heading: "About".to_string(),
        rows: vec![
            FactRow {
                label: "Version".to_string(),
                value: env!("CARGO_PKG_VERSION").to_string(),
                reveal_path: None,
            },
            FactRow {
                label: "Build".to_string(),
                value: build_label(),
                reveal_path: None,
            },
            FactRow {
                label: "Licence".to_string(),
                value: env!("CARGO_PKG_LICENSE").to_string(),
                reveal_path: None,
            },
        ],
    };

    AdvancedModel {
        groups: vec![files, index, about],
        links: links(),
        reindex_help: "Re-reads every session file Claude Code has written and rebuilds the \
                       index from scratch. Perch only ever reads your Claude Code folder; \
                       nothing in it is changed."
            .to_string(),
        reset_warning: format!(
            "This puts all twenty-six settings back to their factory values by rewriting {}, \
             including anything you edited there by hand. Your index, your project notes and \
             your model prices are left alone.",
            config_path.display()
        ),
    }
}

/// Where the source lives. Composed here rather than in a shell because the
/// shell may not carry an absolute URL at all — CI forbids one in the app's
/// Swift, precisely so links live on this side.
fn links() -> Vec<LinkRow> {
    let repo = env!("CARGO_PKG_REPOSITORY");
    if repo.is_empty() {
        return Vec::new();
    }
    vec![
        LinkRow {
            label: "Source code".to_string(),
            url: repo.to_string(),
        },
        LinkRow {
            label: "Report an issue".to_string(),
            url: format!("{}/issues", repo.trim_end_matches('/')),
        },
    ]
}

/// Which build this is, in the only terms the binary can honestly establish
/// on its own. `PERCH_BUILD` is read if whoever compiled it set one (a tag,
/// a commit) — never invented here, because a build identifier Perch made up
/// is worse than none in the one place a person goes to report a bug.
fn build_label() -> String {
    match option_env!("PERCH_BUILD") {
        Some(b) if !b.trim().is_empty() => b.trim().to_string(),
        _ if cfg!(debug_assertions) => "debug".to_string(),
        _ => "release".to_string(),
    }
}

/// A path row: the path as its own value, and Reveal offered only when
/// something is actually there to reveal.
fn path_row(label: &str, path: &Path) -> FactRow {
    FactRow {
        label: label.to_string(),
        value: path.display().to_string(),
        reveal_path: path.exists().then(|| path.display().to_string()),
    }
}

fn file_size(path: &Path) -> String {
    std::fs::metadata(path)
        .ok()
        .map_or_else(|| DASH.to_string(), |m| human_bytes(m.len()))
}

/// When the index file was last written — which is what "last successful
/// index" means on disk, and is established rather than recorded, because
/// Perch keeps no index-run log to read one out of.
fn modified_ago(path: &Path, now_ms: i64) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return DASH.to_string();
    };
    let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return DASH.to_string();
    };
    match i64::try_from(since_epoch.as_millis()) {
        Ok(ms) => ago(now_ms, ms),
        Err(_) => DASH.to_string(),
    }
}

fn ago(now_ms: i64, then_ms: i64) -> String {
    if then_ms <= 0 {
        return DASH.to_string();
    }
    format!("{} ago", human_elapsed(now_ms - then_ms))
}

fn count_or_dash(count: Option<anyhow::Result<i64>>) -> String {
    match count {
        Some(Ok(n)) => n.to_string(),
        _ => DASH.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn rows<'a>(m: &'a AdvancedModel, heading: &str) -> &'a [FactRow] {
        &m.groups
            .iter()
            .find(|g| g.heading == heading)
            .expect("a group by that name")
            .rows
    }

    fn value(m: &AdvancedModel, heading: &str, label: &str) -> String {
        rows(m, heading)
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no {label} row"))
            .value
            .clone()
    }

    #[test]
    fn a_real_index_reports_its_own_counts_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("perch-index.db");
        let database = db::open(&db_path).unwrap();
        database
            .conn()
            .execute_batch(
                "INSERT INTO projects (id, slug, real_path) VALUES (1, 'p', '/p');
                 INSERT INTO sessions (id, project_id, file_path) VALUES ('s1', 1, '/p/s1.jsonl');
                 INSERT INTO turns (session_id, ts, model, input, output)
                     VALUES ('s1', 1000, 'claude-opus-5', 10, 20);",
            )
            .unwrap();

        let config = dir.path().join("config.toml");
        std::fs::write(&config, "").unwrap();
        let m = build_advanced(Some(&database), &config, &db_path, dir.path(), 2_000);

        assert_eq!(value(&m, "Index", "Sessions indexed"), "1");
        assert_eq!(value(&m, "Index", "Turns indexed"), "1");
        assert_ne!(value(&m, "Index", "Index size"), DASH);
        assert_eq!(
            value(&m, "Index", "Newest session activity"),
            "1s ago",
            "an age is finished here, never assembled in a shell"
        );
    }

    #[test]
    fn an_index_that_will_not_open_still_yields_a_pane_of_dashes_not_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let db_path = dir.path().join("missing.db");

        let m = build_advanced(None, &config, &db_path, dir.path(), 2_000);

        assert_eq!(value(&m, "Index", "Sessions indexed"), DASH);
        assert_eq!(value(&m, "Index", "Turns indexed"), DASH);
        assert_eq!(value(&m, "Index", "Index size"), DASH);
        assert_eq!(value(&m, "Index", "Index last written"), DASH);
        assert_eq!(
            value(&m, "Files", "Settings file"),
            config.display().to_string(),
            "the paths must survive a broken index — that is who needs them"
        );
    }

    #[test]
    fn reveal_is_offered_only_where_something_is_there_to_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let m = build_advanced(None, &config, &dir.path().join("x.db"), dir.path(), 0);

        let settings_row = rows(&m, "Files")
            .iter()
            .find(|r| r.label == "Settings file")
            .unwrap();
        assert_eq!(
            settings_row.reveal_path, None,
            "the file does not exist yet"
        );

        std::fs::write(&config, "").unwrap();
        let m = build_advanced(None, &config, &dir.path().join("x.db"), dir.path(), 0);
        let settings_row = rows(&m, "Files")
            .iter()
            .find(|r| r.label == "Settings file")
            .unwrap();
        assert!(settings_row.reveal_path.is_some());
    }

    #[test]
    fn about_names_the_version_and_the_licence_the_crate_declares() {
        let dir = tempfile::tempdir().unwrap();
        let m = build_advanced(
            None,
            &dir.path().join("c.toml"),
            &dir.path().join("x.db"),
            dir.path(),
            0,
        );
        assert_eq!(value(&m, "About", "Version"), env!("CARGO_PKG_VERSION"));
        assert_eq!(value(&m, "About", "Licence"), "MIT");
        assert!(!value(&m, "About", "Build").is_empty());
    }

    #[test]
    fn the_reset_warning_names_the_file_it_will_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let m = build_advanced(None, &config, &dir.path().join("x.db"), dir.path(), 0);
        assert!(m.reset_warning.contains(&config.display().to_string()));
    }
}
