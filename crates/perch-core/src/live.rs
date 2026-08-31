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
}
