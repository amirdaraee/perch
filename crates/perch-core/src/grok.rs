//! xAI's Grok Build, as a [`Provider`].
//!
//! Grok keeps each session in `~/.grok/sessions/<folder>/<uuid>/`, where
//! `<folder>` is the session's working folder, URL-encoded. Perch reads one
//! file there, `usage.json`, which records every turn's usage split by model.
//! Nothing else in the session folder is opened: the others hold the
//! conversation, and `~/.grok/auth.json` holds the login.
//!
//! Grok counts cached and cache-written tokens inside `inputTokens`, and
//! reasoning inside `outputTokens`. Perch keeps them apart, as Claude Code
//! reports them, so nothing is counted twice.
//!
//! `active_sessions.json` may one day say what's running, but its shape while
//! a session runs isn't known yet, so Grok declares no live status.

use crate::actions::TerminalCommand;
use crate::db::Db;
use crate::index::IndexStats;
use crate::live::LiveSession;
use crate::model::{SessionRecord, Turn, TurnUsage};
use crate::platform::ProcessProbe;
use crate::provider::{project_for, session_key, Capabilities, Provider};
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const GROK: &str = "grok";

/// Grok's home: `~/.grok`.
pub fn grok_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GROK_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".grok"))
}

pub struct Grok {
    home: PathBuf,
}

impl Grok {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    /// `(working folder, session id, usage.json)` for every session.
    fn sessions(&self) -> Vec<(String, String, PathBuf)> {
        let mut out = Vec::new();
        for folder in subdirs(&self.home.join("sessions")) {
            let Some(cwd) = folder
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(percent_decode)
                .filter(|c| c.starts_with('/'))
            else {
                continue;
            };
            for session in subdirs(&folder) {
                let Some(id) = session.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let usage = session.join("usage.json");
                if usage.is_file() {
                    out.push((cwd.clone(), id.to_string(), usage));
                }
            }
        }
        out.sort();
        out
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// `%2FUsers%2Fa` to `/Users/a`. `None` for anything malformed.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn count(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn usage_of(u: &Value) -> TurnUsage {
    let input = count(u, "inputTokens");
    let cached = count(u, "cachedReadTokens");
    let written = count(u, "cacheCreationTokens");
    TurnUsage {
        input: input.saturating_sub(cached).saturating_sub(written),
        output: count(u, "outputTokens"),
        cache_read: cached,
        cache_write_5m: written,
        cache_write_1h: 0,
        thinking: count(u, "reasoningTokens"),
    }
}

fn time_ms(v: &Value, key: &str) -> Option<i64> {
    let s = v.get(key)?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// One turn per model per Grok turn, so each model's tokens get its own price.
pub(crate) fn parse_usage(json: &Value, key: &str) -> Vec<Turn> {
    let mut out = Vec::new();
    for t in json
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(ts) = time_ms(t, "endedAt") else {
            continue;
        };
        let number = count(t, "turnNumber");
        match t.get("modelUsage").and_then(Value::as_object) {
            Some(models) if !models.is_empty() => {
                for (model, u) in models {
                    out.push(Turn {
                        message_id: Some(format!("{key}:{number}:{model}")),
                        ts,
                        model: model.clone(),
                        usage: usage_of(u),
                    });
                }
            }
            _ => out.push(Turn {
                message_id: Some(format!("{key}:{number}")),
                ts,
                model: t
                    .get("primaryModelId")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                usage: usage_of(t),
            }),
        }
    }
    out
}

impl Provider for Grok {
    fn id(&self) -> &'static str {
        GROK
    }

    fn name(&self) -> &'static str {
        "Grok Build"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            live_sessions: false,
            waiting: false,
            tokens: true,
            resume: false,
        }
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.home.join("sessions")]
    }

    fn index(&self, db: &Db) -> Result<IndexStats> {
        let mut stats = IndexStats::default();
        let mut projects = std::collections::HashSet::new();
        for (cwd, id, path) in self.sessions() {
            let key = session_key(GROK, &id);
            let Ok(len) = std::fs::metadata(&path).map(|m| m.len()) else {
                stats.sessions_skipped += 1;
                continue;
            };
            stats.sessions += 1;
            if db.session_offset(&key)? == len {
                continue;
            }
            let Some(json) = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            else {
                stats.sessions_skipped += 1;
                continue;
            };
            let turns = parse_usage(&json, &key);
            let project = project_for(db, &cwd)?;
            projects.insert(project);
            stats.bytes_read += len;
            stats.new_turns += turns.len();
            let record = SessionRecord {
                provider: GROK.into(),
                id: key,
                project_id: project,
                file_path: path.to_string_lossy().into_owned(),
                file_size: len,
                indexed_offset: len,
                started_at: turns.iter().map(|t| t.ts).min(),
                last_activity_at: time_ms(&json, "updatedAt")
                    .or_else(|| turns.iter().map(|t| t.ts).max()),
                cwd: Some(cwd),
                git_branch: None,
                cc_version: None,
                title: None,
                message_count: json
                    .get("turns")
                    .and_then(Value::as_array)
                    .map_or(0, |t| t.len()) as u64,
            };
            db.apply_scan(&record, &turns, true)?;
        }
        stats.projects = projects.len();
        Ok(stats)
    }

    fn live_sessions(&self, _probe: &dyn ProcessProbe) -> Vec<LiveSession> {
        Vec::new()
    }

    fn resume_command(&self, _: &str, _: &str, _: bool) -> Option<TerminalCommand> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn turn(n: u64, at: &str, models: &str) -> String {
        format!(
            r#"{{"turnNumber":{n},"endedAt":"{at}","inputTokens":0,"outputTokens":0,"totalTokens":0,"primaryModelId":"grok-4.7","modelUsage":{{{models}}}}}"#
        )
    }

    fn model(
        name: &str,
        input: u64,
        cached: u64,
        written: u64,
        output: u64,
        reasoning: u64,
    ) -> String {
        format!(
            r#""{name}":{{"inputTokens":{input},"outputTokens":{output},"cachedReadTokens":{cached},"cacheCreationTokens":{written},"reasoningTokens":{reasoning},"totalTokens":{},"costUsdTicks":1}}"#,
            input + output
        )
    }

    fn usage_json(turns: &[String]) -> String {
        format!(
            r#"{{"sessionId":"s","updatedAt":"2026-11-05T10:05:00Z","session":{{}},"turns":[{}]}}"#,
            turns.join(",")
        )
    }

    fn home_with(usage: &str) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let session = home
            .path()
            .join("sessions/%2FUsers%2Fa%2Fmy%20app/019f-uuid");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("usage.json"), usage).unwrap();
        std::fs::write(session.join("chat_history.jsonl"), "secret").unwrap();
        std::fs::write(home.path().join("auth.json"), "{}").unwrap();
        home
    }

    #[test]
    fn each_model_in_each_turn_is_its_own_response_and_cache_comes_out_of_input() {
        let json: Value = serde_json::from_str(&usage_json(&[
            turn(
                1,
                "2026-11-05T10:00:00Z",
                &model("grok-4.7-build", 1000, 600, 100, 50, 20),
            ),
            turn(
                2,
                "2026-11-05T10:01:00Z",
                &[
                    model("grok-4.7-build", 10, 0, 0, 5, 0),
                    model("grok-4.7", 20, 0, 0, 5, 0),
                ]
                .join(","),
            ),
        ]))
        .unwrap();
        let turns = parse_usage(&json, "grok:s");
        assert_eq!(turns.len(), 3);
        assert_eq!(
            turns[0].usage,
            TurnUsage {
                input: 300,
                output: 50,
                cache_read: 600,
                cache_write_5m: 100,
                cache_write_1h: 0,
                thinking: 20,
            }
        );
        assert_eq!(
            turns[0].usage.total_tokens(),
            1050,
            "Grok's own total, input plus output"
        );
        let ids: std::collections::HashSet<_> =
            turns.iter().map(|t| t.message_id.clone()).collect();
        assert_eq!(ids.len(), 3, "every response has its own id");
    }

    #[test]
    fn a_session_is_indexed_from_usage_json_alone_into_its_folders_project() {
        let home = home_with(&usage_json(&[turn(
            1,
            "2026-11-05T10:00:00Z",
            &model("grok-4.7", 100, 0, 0, 10, 0),
        )]));
        let db = open_in_memory().unwrap();
        crate::pricing::seed_default_prices(&db).unwrap();
        let grok = Grok::new(home.path());
        let stats = grok.index(&db).unwrap();
        assert_eq!((stats.sessions, stats.new_turns), (1, 1));
        let path: String = db
            .conn()
            .query_row("SELECT real_path FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "/Users/a/my app");
        assert!(crate::query::model_has_price(&db, "grok-4.7").unwrap());
        assert!(!crate::query::model_has_price(&db, "grok-4.7-build").unwrap());

        let again = grok.index(&db).unwrap();
        assert_eq!(
            again.new_turns, 0,
            "an unchanged usage.json isn't read again"
        );
        assert_eq!(db.turn_count().unwrap(), 1);
    }

    #[test]
    fn a_session_that_grew_is_replaced_not_added_to() {
        let home = home_with(&usage_json(&[turn(
            1,
            "2026-11-05T10:00:00Z",
            &model("grok-4.7", 100, 0, 0, 10, 0),
        )]));
        let db = open_in_memory().unwrap();
        let grok = Grok::new(home.path());
        grok.index(&db).unwrap();
        let usage = home
            .path()
            .join("sessions/%2FUsers%2Fa%2Fmy%20app/019f-uuid/usage.json");
        std::fs::write(
            &usage,
            usage_json(&[
                turn(
                    1,
                    "2026-11-05T10:00:00Z",
                    &model("grok-4.7", 100, 0, 0, 10, 0),
                ),
                turn(
                    2,
                    "2026-11-05T10:02:00Z",
                    &model("grok-4.7", 200, 0, 0, 20, 0),
                ),
            ]),
        )
        .unwrap();
        grok.index(&db).unwrap();
        assert_eq!(db.turn_count().unwrap(), 2);
    }

    #[test]
    fn folders_that_are_not_encoded_paths_are_ignored() {
        assert_eq!(percent_decode("%2FUsers%2Fa").as_deref(), Some("/Users/a"));
        assert_eq!(percent_decode("%2"), None);
        let home = home_with("{}");
        std::fs::create_dir_all(home.path().join("sessions/not-a-path/x")).unwrap();
        std::fs::write(home.path().join("sessions/not-a-path/x/usage.json"), "{}").unwrap();
        assert_eq!(Grok::new(home.path()).sessions().len(), 1);
    }
}
