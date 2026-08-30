//! Parsing of individual transcript JSONL lines. Lenient by design.

use crate::model::TurnUsage;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedLine {
    pub kind: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub ts: Option<i64>,
    pub model: Option<String>,
    pub usage: Option<TurnUsage>,
}

fn u64_at(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn string_at(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn parse_ts(v: &Value) -> Option<i64> {
    let raw = v.get("timestamp")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn parse_usage(message: &Value) -> Option<TurnUsage> {
    let usage = message.get("usage")?;
    if !usage.is_object() {
        return None;
    }

    let (w5, w1h) = match usage.get("cache_creation") {
        Some(c) if c.is_object() => (
            u64_at(c, "ephemeral_5m_input_tokens"),
            u64_at(c, "ephemeral_1h_input_tokens"),
        ),
        // No breakdown available: attribute the total to the 5m bucket.
        _ => (u64_at(usage, "cache_creation_input_tokens"), 0),
    };

    let thinking = usage
        .get("output_tokens_details")
        .map(|d| u64_at(d, "thinking_tokens"))
        .unwrap_or(0);

    Some(TurnUsage {
        input: u64_at(usage, "input_tokens"),
        output: u64_at(usage, "output_tokens"),
        cache_read: u64_at(usage, "cache_read_input_tokens"),
        cache_write_5m: w5,
        cache_write_1h: w1h,
        thinking,
    })
}

/// Parse one transcript line. Returns `None` for blank or malformed lines,
/// which callers count and skip rather than treating as fatal.
pub fn parse_line(line: &str) -> Option<ParsedLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    if !v.is_object() {
        return None;
    }

    let message = v.get("message");
    Some(ParsedLine {
        kind: v
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        cwd: string_at(&v, "cwd"),
        git_branch: string_at(&v, "gitBranch"),
        cc_version: string_at(&v, "version"),
        ts: parse_ts(&v),
        model: message.and_then(|m| string_at(m, "model")),
        usage: message.and_then(parse_usage),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSISTANT: &str = r#"{"type":"assistant","sessionId":"s1","cwd":"/Users/a/proj","gitBranch":"main","version":"2.1.234","timestamp":"2026-08-18T12:16:08.619Z","message":{"model":"claude-fable-5","usage":{"input_tokens":2,"cache_creation_input_tokens":23848,"cache_read_input_tokens":24221,"output_tokens":231,"output_tokens_details":{"thinking_tokens":79},"cache_creation":{"ephemeral_1h_input_tokens":23848,"ephemeral_5m_input_tokens":0}}}}"#;

    #[test]
    fn parses_assistant_usage_into_separate_classes() {
        let p = parse_line(ASSISTANT).expect("should parse");
        let u = p.usage.expect("should have usage");
        assert_eq!(u.input, 2);
        assert_eq!(u.output, 231);
        assert_eq!(u.cache_read, 24221);
        assert_eq!(u.cache_write_1h, 23848);
        assert_eq!(u.cache_write_5m, 0);
        assert_eq!(u.thinking, 79);
        assert_eq!(p.model.as_deref(), Some("claude-fable-5"));
    }

    #[test]
    fn extracts_session_metadata() {
        let p = parse_line(ASSISTANT).unwrap();
        assert_eq!(p.cwd.as_deref(), Some("/Users/a/proj"));
        assert_eq!(p.git_branch.as_deref(), Some("main"));
        assert_eq!(p.cc_version.as_deref(), Some("2.1.234"));
        assert!(p.ts.is_some());
    }

    #[test]
    fn parses_timestamp_to_unix_millis() {
        let p = parse_line(ASSISTANT).unwrap();
        // 2026-08-18T12:16:08.619Z
        assert_eq!(p.ts, Some(1787055368619));
    }

    #[test]
    fn falls_back_to_total_cache_creation_when_breakdown_absent() {
        let line = r#"{"type":"assistant","timestamp":"2026-08-18T12:16:08.619Z","message":{"model":"m","usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":40}}}"#;
        let u = parse_line(line).unwrap().usage.unwrap();
        assert_eq!(u.cache_write_5m, 40);
        assert_eq!(u.cache_write_1h, 0);
    }

    #[test]
    fn user_line_has_metadata_but_no_usage() {
        let line = r#"{"type":"user","cwd":"/Users/a/proj","timestamp":"2026-08-18T12:00:00.000Z","message":{"role":"user","content":"hi"}}"#;
        let p = parse_line(line).unwrap();
        assert!(p.usage.is_none());
        assert_eq!(p.kind, "user");
        assert_eq!(p.cwd.as_deref(), Some("/Users/a/proj"));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let line = r#"{"type":"assistant","brandNewField":{"x":1},"timestamp":"2026-08-18T12:16:08.619Z","message":{"model":"m","usage":{"input_tokens":5,"output_tokens":6,"somethingNew":true}}}"#;
        let u = parse_line(line).unwrap().usage.unwrap();
        assert_eq!(u.input, 5);
        assert_eq!(u.output, 6);
    }

    #[test]
    fn returns_none_for_malformed_json() {
        assert!(parse_line("{not json").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
    }

    #[test]
    fn returns_none_for_json_that_is_not_an_object() {
        assert!(parse_line("[1,2,3]").is_none());
        assert!(parse_line("\"a string\"").is_none());
    }

    #[test]
    fn handles_meta_lines_without_type() {
        let line = r#"{"leafUuid":"x","sessionId":"s1"}"#;
        let p = parse_line(line).unwrap();
        assert_eq!(p.kind, "");
        assert!(p.usage.is_none());
    }
}
