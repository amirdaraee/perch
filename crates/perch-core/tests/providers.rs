//! A second tool, faked, goes through the same path a real one will: indexed
//! beside Claude Code, listed as running, and shown in the main window and
//! the usage view, without disturbing Claude Code's own records.

use perch_core::actions::TerminalCommand;
use perch_core::db::{open_in_memory, Db};
use perch_core::index::IndexStats;
use perch_core::live::{LiveSession, SessionStatus};
use perch_core::model::{SessionRecord, Turn, TurnUsage};
use perch_core::platform::ProcessProbe;
use perch_core::provider::{self, session_key, Capabilities, ClaudeCode, Provider, CLAUDE_CODE};
use perch_core::settings::Settings;
use perch_core::ui::{main_window, model::PopoverModel, usage};
use std::fs;
use std::path::PathBuf;

const NOW: i64 = 1_790_000_000_000;

/// Writes its sessions straight into the index, the way a real provider would
/// after parsing its tool's files.
struct Fake;

impl Provider for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn name(&self) -> &'static str {
        "Fake Tool"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            live_sessions: true,
            waiting: false,
            tokens: true,
            resume: false,
        }
    }
    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn index(&self, db: &Db) -> anyhow::Result<IndexStats> {
        let project = db.upsert_project("fake-proj", "/work/fake-proj", false)?;
        let session = SessionRecord {
            provider: "fake".into(),
            id: session_key("fake", "s1"),
            project_id: project,
            file_path: "/fake/s1".into(),
            file_size: 1,
            indexed_offset: 1,
            started_at: Some(NOW - 60_000),
            last_activity_at: Some(NOW - 1_000),
            cwd: Some("/work/fake-proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        };
        // The same response id Claude Code's session uses: per-provider
        // de-duplication must keep both.
        let turn = Turn {
            message_id: Some("msg_1".into()),
            ts: NOW - 1_000,
            model: "fake-model-1".into(),
            usage: TurnUsage {
                input: 100,
                output: 50,
                ..TurnUsage::default()
            },
        };
        db.apply_scan(&session, &[turn], false)?;
        Ok(IndexStats {
            projects: 1,
            sessions: 1,
            new_turns: 1,
            ..IndexStats::default()
        })
    }
    fn live_sessions(&self, _probe: &dyn ProcessProbe) -> Vec<LiveSession> {
        vec![LiveSession {
            pid: 1,
            session_id: session_key("fake", "s1"),
            cwd: "/work/fake-proj".into(),
            name: String::new(),
            kind: "interactive".into(),
            status: SessionStatus::Working,
            started_at: NOW - 60_000,
            status_updated_at: NOW - 1_000,
            cc_version: None,
            socket_path: None,
        }]
    }
    fn resume_command(&self, _: &str, _: &str, _: bool) -> Option<TerminalCommand> {
        None
    }
}

struct NoProcesses;
impl ProcessProbe for NoProcesses {
    fn is_alive(&self, _: i32) -> bool {
        false
    }
    fn process_name(&self, _: i32) -> Option<String> {
        None
    }
}

fn claude_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("projects").join("-work-claude-proj");
    fs::create_dir_all(&project).unwrap();
    let ts = chrono::DateTime::from_timestamp_millis(NOW - 2_000)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    fs::write(
        project.join("aaaa.jsonl"),
        format!(
            r#"{{"type":"assistant","cwd":"/work/claude-proj","gitBranch":"main","version":"2.1.1","timestamp":"{ts}","message":{{"id":"msg_1","model":"claude-sonnet-5","usage":{{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        ) + "\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_second_tool_shows_up_end_to_end_beside_claude_code() {
    let claude_home = claude_dir();
    let claude = ClaudeCode::new(claude_home.path());
    let db = open_in_memory().unwrap();
    perch_core::pricing::seed_default_prices(&db).unwrap();

    let stats = provider::index_all(&db, &[&claude, &Fake]).unwrap();
    assert_eq!(stats.sessions, 2);
    assert_eq!(
        db.turn_count().unwrap(),
        2,
        "a shared response id is one response per tool, not one overall"
    );

    let provider_of = |id: &str| -> String {
        db.conn()
            .query_row("SELECT provider FROM sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(
        provider_of("aaaa"),
        CLAUDE_CODE,
        "Claude Code's ids stay bare"
    );
    assert_eq!(provider_of("fake:s1"), "fake");

    let live = provider::live_sessions(&[&claude, &Fake], &NoProcesses);
    assert_eq!(live.len(), 1, "only the fake tool has a running session");

    let settings = Settings::default();
    let window =
        main_window::build_main_window(Some(&db), PopoverModel::empty(), &live, NOW, &settings);
    let names: Vec<&str> = window.projects.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"fake-proj"), "{names:?}");
    assert!(names.contains(&"claude-proj"), "{names:?}");
    let fake = window
        .projects
        .iter()
        .find(|p| p.name == "fake-proj")
        .unwrap();
    assert_eq!(fake.live_label.as_deref(), Some("1 running"));

    let usage = usage::build_usage(&db, NOW, &settings).unwrap();
    let models: Vec<&str> = usage.by_model.iter().map(|m| m.model.as_str()).collect();
    assert!(models.contains(&"fake-model-1"), "{models:?}");
    assert!(models.contains(&"claude-sonnet-5"), "{models:?}");

    // A second pass changes nothing for either tool.
    provider::index_all(&db, &[&claude, &Fake]).unwrap();
    assert_eq!(db.turn_count().unwrap(), 2);
}

#[test]
fn one_tool_failing_does_not_stop_the_others() {
    struct Broken;
    impl Provider for Broken {
        fn id(&self) -> &'static str {
            "broken"
        }
        fn name(&self) -> &'static str {
            "Broken Tool"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                live_sessions: false,
                waiting: false,
                tokens: false,
                resume: false,
            }
        }
        fn watch_paths(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn index(&self, _: &Db) -> anyhow::Result<IndexStats> {
            anyhow::bail!("unreadable")
        }
        fn live_sessions(&self, _: &dyn ProcessProbe) -> Vec<LiveSession> {
            panic!("a tool that can't list sessions is never asked to")
        }
        fn resume_command(&self, _: &str, _: &str, _: bool) -> Option<TerminalCommand> {
            None
        }
    }

    let db = open_in_memory().unwrap();
    let err = provider::index_all(&db, &[&Broken, &Fake]).unwrap_err();
    assert!(format!("{err:#}").contains("Broken Tool"), "{err:#}");
    assert_eq!(
        db.turn_count().unwrap(),
        1,
        "the fake tool was still indexed"
    );
    assert!(provider::live_sessions(&[&Broken], &NoProcesses).is_empty());
}
