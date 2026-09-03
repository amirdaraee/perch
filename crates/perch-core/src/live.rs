//! Live Claude Code session records. Status is read, never inferred.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionStatus {
    Working,
    Idle,
    Waiting {
        reason: Option<String>,
        since_ms: i64,
    },
    Background,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSession {
    pub pid: i32,
    pub session_id: String,
    pub cwd: String,
    pub name: String,
    /// "interactive" or "bg". Independent of `status`.
    pub kind: String,
    pub status: SessionStatus,
    pub started_at: i64,
    pub status_updated_at: i64,
    pub cc_version: Option<String>,
    pub socket_path: Option<String>,
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn i64_at(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// Parse one `<pid>.json` session record. Lenient: unknown fields are ignored and a
/// malformed record yields `None` rather than an error, because Claude Code owns this
/// format and may extend it at any time.
pub fn parse_session_record(json: &str) -> Option<LiveSession> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    if !v.is_object() {
        return None;
    }

    let pid = v.get("pid").and_then(Value::as_i64)? as i32;
    let session_id = str_at(&v, "sessionId")?;
    let status_updated_at = i64_at(&v, "statusUpdatedAt");

    let status = match v.get("status").and_then(Value::as_str) {
        Some("waiting") => SessionStatus::Waiting {
            reason: str_at(&v, "waitingFor"),
            since_ms: status_updated_at,
        },
        Some("idle") => SessionStatus::Idle,
        // "busy" and anything unrecognised: treat as working rather than
        // inventing a state. A future status string must not break the UI.
        _ => SessionStatus::Working,
    };

    Some(LiveSession {
        pid,
        session_id,
        cwd: str_at(&v, "cwd").unwrap_or_default(),
        name: str_at(&v, "name").unwrap_or_default(),
        kind: str_at(&v, "kind").unwrap_or_else(|| "interactive".to_string()),
        status,
        started_at: i64_at(&v, "startedAt"),
        status_updated_at,
        cc_version: str_at(&v, "version"),
        socket_path: str_at(&v, "messagingSocketPath"),
    })
}

/// List `<pid>.json` files in the sessions directory. A missing directory is normal.
pub fn record_files(sessions_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
        .collect()
}

use crate::platform::ProcessProbe;

/// Read every session record and keep only those passing all three confirmations
/// from spec §7. Blocked sessions sort first — they are the ones needing you.
pub fn live_sessions(sessions_dir: &Path, probe: &dyn ProcessProbe) -> Vec<LiveSession> {
    let mut out: Vec<LiveSession> = record_files(sessions_dir)
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(&p).ok())
        .filter_map(|s| parse_session_record(&s))
        .filter(|s| probe.is_alive(s.pid) && probe.process_name(s.pid).as_deref() == Some("claude"))
        .collect();

    out.sort_by(|a, b| {
        let rank = |s: &LiveSession| match s.status {
            SessionStatus::Waiting { .. } => 0,
            _ => 1,
        };
        // pid breaks the tie: a just-started session can have no name at all,
        // and two nameless records would otherwise fall back to `read_dir`
        // order, letting rows swap places on every poll.
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.pid.cmp(&b.pid))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: &str = r#"{"pid":11107,"sessionId":"s-busy","cwd":"/Users/a/proj","startedAt":1788097229808,"version":"2.1.251","kind":"interactive","messagingSocketPath":"/tmp/cc-socks/11107.sock","name":"first-week-university-b6","status":"busy","statusUpdatedAt":1788099012751}"#;

    const WAITING: &str = r#"{"pid":30778,"sessionId":"s-wait","cwd":"/Users/a/dash","startedAt":1787988672885,"version":"2.1.250","kind":"bg","name":"Personal dashboard finance section","status":"waiting","waitingFor":"dialog open","statusUpdatedAt":1787988673228}"#;

    #[test]
    fn parses_a_busy_interactive_session() {
        let s = parse_session_record(BUSY).expect("should parse");
        assert_eq!(s.pid, 11107);
        assert_eq!(s.session_id, "s-busy");
        assert_eq!(s.name, "first-week-university-b6");
        assert_eq!(s.kind, "interactive");
        assert_eq!(s.cwd, "/Users/a/proj");
        assert_eq!(s.status, SessionStatus::Working);
        assert_eq!(s.socket_path.as_deref(), Some("/tmp/cc-socks/11107.sock"));
    }

    #[test]
    fn waiting_carries_its_reason_and_timestamp() {
        let s = parse_session_record(WAITING).unwrap();
        match &s.status {
            SessionStatus::Waiting { reason, since_ms } => {
                assert_eq!(reason.as_deref(), Some("dialog open"));
                assert_eq!(*since_ms, 1787988673228);
            }
            other => panic!("expected Waiting, got {other:?}"),
        }
    }

    #[test]
    fn background_kind_is_preserved_alongside_status() {
        // `kind` and `status` are independent: a bg session can be busy or waiting.
        let s = parse_session_record(WAITING).unwrap();
        assert_eq!(s.kind, "bg");
        assert!(matches!(s.status, SessionStatus::Waiting { .. }));
    }

    #[test]
    fn unknown_status_string_is_not_fatal() {
        let line = r#"{"pid":1,"sessionId":"x","cwd":"/tmp","name":"n","kind":"interactive","status":"some-future-state","startedAt":1,"statusUpdatedAt":2}"#;
        let s = parse_session_record(line).unwrap();
        assert_eq!(
            s.status,
            SessionStatus::Working,
            "unknown status falls back to Working"
        );
    }

    #[test]
    fn idle_status_parses_to_the_idle_variant() {
        let line = r#"{"pid":1,"sessionId":"x","cwd":"/tmp","name":"n","kind":"interactive","status":"idle","startedAt":1,"statusUpdatedAt":2}"#;
        let s = parse_session_record(line).unwrap();
        assert_eq!(s.status, SessionStatus::Idle);
    }

    #[test]
    fn missing_optional_fields_are_tolerated() {
        let line = r#"{"pid":2,"sessionId":"y","cwd":"/tmp","name":"n","kind":"interactive","status":"busy","startedAt":1,"statusUpdatedAt":2}"#;
        let s = parse_session_record(line).unwrap();
        assert!(s.socket_path.is_none());
        assert!(s.cc_version.is_none());
    }

    #[test]
    fn a_record_without_a_pid_or_session_id_is_rejected() {
        assert!(parse_session_record(r#"{"sessionId":"z","cwd":"/tmp"}"#).is_none());
        assert!(parse_session_record(r#"{"pid":3,"cwd":"/tmp"}"#).is_none());
    }

    #[test]
    fn malformed_json_is_rejected_not_panicked() {
        assert!(parse_session_record("{nope").is_none());
        assert!(parse_session_record("").is_none());
        assert!(parse_session_record("[1,2]").is_none());
    }

    #[test]
    fn record_files_lists_only_json_and_tolerates_a_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("111.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("222.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("333.key"), "x").unwrap();
        let mut found = record_files(tmp.path());
        found.sort();
        assert_eq!(found.len(), 2);

        assert!(record_files(&tmp.path().join("nope")).is_empty());
    }

    #[test]
    fn status_serializes_with_the_field_names_the_frontend_expects() {
        let waiting = SessionStatus::Waiting {
            reason: Some("dialog open".into()),
            since_ms: 5,
        };
        let v = serde_json::to_value(&waiting).unwrap();
        assert_eq!(
            v["kind"], "waiting",
            "variant must be camelCase under the `kind` tag"
        );
        assert_eq!(
            v["sinceMs"], 5,
            "rename_all_fields must camelCase struct-variant fields"
        );
        assert_eq!(v["reason"], "dialog open");

        let working = serde_json::to_value(SessionStatus::Working).unwrap();
        assert_eq!(working["kind"], "working");

        let idle = serde_json::to_value(SessionStatus::Idle).unwrap();
        assert_eq!(idle["kind"], "idle");
    }

    #[test]
    fn live_session_serializes_with_camel_case_keys() {
        let s = parse_session_record(WAITING).unwrap();
        let v = serde_json::to_value(&s).unwrap();
        for key in [
            "sessionId",
            "statusUpdatedAt",
            "startedAt",
            "ccVersion",
            "socketPath",
        ] {
            assert!(
                v.get(key).is_some(),
                "missing camelCase key `{key}` in serialized LiveSession"
            );
        }
        assert!(
            v.get("session_id").is_none(),
            "snake_case keys must not leak to the frontend"
        );
    }

    #[test]
    fn kind_and_status_are_independent_across_all_four_combinations() {
        let cases = [
            ("interactive", "busy", false),
            ("interactive", "waiting", true),
            ("bg", "busy", false),
            ("bg", "waiting", true),
        ];
        for (kind, status, expect_waiting) in cases {
            let line = format!(
                r#"{{"pid":9,"sessionId":"s","cwd":"/tmp","name":"n","kind":"{kind}","status":"{status}","startedAt":1,"statusUpdatedAt":2}}"#
            );
            let s = parse_session_record(&line).expect("should parse");
            assert_eq!(s.kind, kind, "kind must be preserved verbatim");
            assert_eq!(
                matches!(s.status, SessionStatus::Waiting { .. }),
                expect_waiting,
                "status must be derived from `status` alone, never from `kind` ({kind}/{status})"
            );
        }
    }

    use crate::platform::ProcessProbe;
    use std::collections::HashSet;

    /// Fake probe so liveness rules are unit-testable without real processes.
    struct FakeProbe {
        alive: HashSet<i32>,
        names: std::collections::HashMap<i32, String>,
    }

    impl FakeProbe {
        fn new(alive: &[i32]) -> Self {
            Self {
                alive: alive.iter().copied().collect(),
                names: Default::default(),
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

    fn write_record(dir: &Path, pid: i32, session_id: &str, status: &str) {
        write_record_named(dir, pid, session_id, status, "n");
    }

    fn write_record_named(dir: &Path, pid: i32, session_id: &str, status: &str, name: &str) {
        let json = format!(
            r#"{{"pid":{pid},"sessionId":"{session_id}","cwd":"/tmp/p","name":"{name}","kind":"interactive","status":"{status}","startedAt":1,"statusUpdatedAt":2}}"#
        );
        std::fs::write(dir.join(format!("{pid}.json")), json).unwrap();
    }

    #[test]
    fn a_session_needs_all_three_confirmations() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 100, "alive-and-matching", "busy");

        let probe = FakeProbe::new(&[100]).with_name(100, "claude");
        assert_eq!(live_sessions(tmp.path(), &probe).len(), 1);
    }

    #[test]
    fn a_dead_pid_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 101, "dead", "busy");
        let probe = FakeProbe::new(&[]).with_name(101, "claude");
        assert!(
            live_sessions(tmp.path(), &probe).is_empty(),
            "stale record must not resurrect"
        );
    }

    #[test]
    fn a_recycled_pid_running_something_else_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 102, "ours", "busy");
        // pid is alive, but it is now some unrelated process
        let probe = FakeProbe::new(&[102]).with_name(102, "python3");
        assert!(
            live_sessions(tmp.path(), &probe).is_empty(),
            "pid reuse must not be reported as a live session"
        );
    }

    #[test]
    fn a_process_whose_path_merely_contains_claude_is_not_live() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 103, "not-claude", "busy");
        // pid is alive, but comm= reports it is actually vim, not claude
        let probe = FakeProbe::new(&[103]).with_name(103, "vim");
        assert!(
            live_sessions(tmp.path(), &probe).is_empty(),
            "a process name that is merely similar to claude must not match"
        );
    }

    #[test]
    fn live_sessions_are_sorted_waiting_first_then_by_name_then_by_pid() {
        let tmp = tempfile::tempdir().unwrap();
        write_record_named(tmp.path(), 200, "s-working", "busy", "b-working");
        write_record_named(tmp.path(), 201, "s-waiting", "waiting", "a-waiting");
        // Two records sharing a name — e.g. two just-started sessions that have
        // no name yet — must not fall back to read_dir order, which can swap
        // them between polls. Written high-pid-first so a stable-but-unsorted
        // implementation would be caught.
        write_record_named(tmp.path(), 203, "s-nameless-hi", "busy", "");
        write_record_named(tmp.path(), 202, "s-nameless-lo", "busy", "");

        let probe = FakeProbe::new(&[200, 201, 202, 203])
            .with_name(200, "claude")
            .with_name(201, "claude")
            .with_name(202, "claude")
            .with_name(203, "claude");

        let out = live_sessions(tmp.path(), &probe);
        assert_eq!(out.len(), 4);
        assert_eq!(
            out[0].session_id, "s-waiting",
            "blocked sessions surface first"
        );
        let pids: Vec<i32> = out.iter().map(|s| s.pid).collect();
        assert_eq!(
            pids,
            vec![201, 202, 203, 200],
            "waiting first, then by name, then by pid for identical names"
        );
    }

    #[test]
    fn an_unreadable_record_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bad.json"), "{ not json").unwrap();
        write_record(tmp.path(), 300, "good", "busy");
        let probe = FakeProbe::new(&[300]).with_name(300, "claude");
        assert_eq!(live_sessions(tmp.path(), &probe).len(), 1);
    }
}
