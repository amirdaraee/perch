//! MCP over stdio: newline-delimited JSON-RPC 2.0. Only the slice of the
//! protocol a tools-only server needs — `initialize`, `ping`, `tools/list`,
//! `tools/call` — written directly against `serde_json` rather than pulling in
//! an SDK.

use crate::tools::{self, Host};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Newest first. A client asking for one of these gets it echoed back; anything
/// else is offered the newest, which the client may then refuse.
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Serve until the client closes stdin. Each request line gets exactly one
/// response line; notifications get none.
pub fn serve(input: impl BufRead, mut output: impl Write, host: &dyn Host) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&line, host) {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// One incoming line to at most one outgoing message.
pub fn handle_line(line: &str, host: &dyn Host) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(error(
                Value::Null,
                PARSE_ERROR,
                &format!("Parse error: {e}"),
            ))
        }
    };
    let Some(object) = message.as_object() else {
        return Some(error(Value::Null, INVALID_REQUEST, "Invalid Request"));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        // A response from the client (we never send requests) or garbage.
        // Neither is answered unless it carries an id to answer to.
        return id.map(|id| error(id, INVALID_REQUEST, "Invalid Request"));
    };
    // No id: a notification. `notifications/initialized` and
    // `notifications/cancelled` need nothing from a server with no long work.
    let id = id?;
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));

    Some(match method {
        "initialize" => result(id, initialize(&params)),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, INVALID_PARAMS, "tools/call needs a tool name"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match tools::call(host, name, &arguments) {
                None => error(id, INVALID_PARAMS, &format!("Unknown tool: {name}")),
                Some(Ok(value)) => result(id, tool_result(value, false)),
                Some(Err(message)) => result(id, tool_result(json!({ "error": message }), true)),
            }
        }
        _ => error(id, METHOD_NOT_FOUND, &format!("Method not found: {method}")),
    })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|v| SUPPORTED_VERSIONS.contains(v))
        .unwrap_or(SUPPORTED_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "perch", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Read-only access to the Claude Code projects, live sessions and token usage Perch has indexed on this Mac."
    })
}

/// Tool output goes out twice: as text for clients that only read `content`,
/// and as `structuredContent` for those that read data.
fn tool_result(value: Value, is_error: bool) -> Value {
    let text = if is_error {
        value["error"].as_str().unwrap_or_default().to_string()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_default()
    };
    let mut out = json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    });
    if !is_error {
        out["structuredContent"] = value;
    }
    out
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tests::FixtureHost;

    fn ask(line: &str) -> Value {
        handle_line(line, &FixtureHost::with_one_project()).expect("a request gets a response")
    }

    #[test]
    fn initialize_echoes_a_supported_version_and_offers_the_newest_otherwise() {
        let r = ask(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        );
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r["result"]["serverInfo"]["name"], "perch");

        let r = ask(
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );
        assert_eq!(r["result"]["protocolVersion"], SUPPORTED_VERSIONS[0]);
    }

    #[test]
    fn notifications_get_no_response() {
        let host = FixtureHost::with_one_project();
        assert!(handle_line(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &host
        )
        .is_none());
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"tools/list"}"#, &host).is_none());
    }

    #[test]
    fn errors_use_the_json_rpc_codes() {
        let host = FixtureHost::with_one_project();
        assert_eq!(
            handle_line("{not json", &host).unwrap()["error"]["code"],
            PARSE_ERROR
        );
        assert_eq!(
            handle_line("[1,2]", &host).unwrap()["error"]["code"],
            INVALID_REQUEST
        );
        assert_eq!(
            ask(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#)["error"]["code"],
            METHOD_NOT_FOUND
        );
        assert_eq!(
            ask(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}"#)
                ["error"]["code"],
            INVALID_PARAMS
        );
    }

    #[test]
    fn tools_list_names_all_four_tools_with_object_schemas() {
        let r = ask(r#"{"jsonrpc":"2.0","id":5,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "list_projects",
                "get_project",
                "live_sessions",
                "usage_summary"
            ]
        );
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
            assert!(!t["description"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn a_tool_call_returns_text_and_structured_content_that_agree() {
        let r = ask(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_projects","arguments":{}}}"#,
        );
        let result = &r["result"];
        assert_eq!(result["isError"], false);
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"]);
        assert_eq!(
            result["structuredContent"]["projects"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_failing_tool_is_an_error_result_not_a_protocol_error() {
        let host = FixtureHost::without_index();
        let r = handle_line(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_projects"}}"#,
            &host,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["error"].is_null());
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("open Perch once"));
    }

    #[test]
    fn serve_answers_each_request_line_once_in_order() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            "\n",
        );
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &FixtureHost::with_one_project()).unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["id"], 2);
        assert_eq!(lines[1]["result"], json!({}));
    }
}
