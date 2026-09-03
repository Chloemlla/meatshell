use std::io::BufRead;

use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use serde_json::{json, Value};

use super::ActivityLog;

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    LATEST_PROTOCOL_VERSION,
];

pub(crate) fn run_stdio() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create MCP runtime")?;
    let activity = std::sync::Arc::new(ActivityLog::open());
    let stdout = std::sync::Arc::new(std::sync::Mutex::new(std::io::stdout()));
    let stdin = std::io::stdin();
    let mut caller = String::from("unknown");
    let mut in_flight: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    for line in stdin.lock().lines() {
        let line = line.context("read MCP request")?;
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Value>(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = error_response(Value::Null, -32700, &error.to_string());
                let mut out = stdout.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Err(write_error) = write_response(&mut *out, &response) {
                    tracing::warn!("mcp: write parse-error response: {write_error}");
                }
                continue;
            }
        };

        // Update the caller identity synchronously (so `initialize` is reflected
        // in the very next audit record), then snapshot it for the worker task.
        remember_caller(&request, &mut caller);
        let caller_snapshot = caller.clone();
        record_usage_start(&activity, &request, &caller_snapshot);

        // Handle each request on the shared multi-thread runtime so one slow
        // SFTP/command call no longer blocks every subsequent request (#58).
        // Responses may complete out of order — JSON-RPC matches them by id —
        // and each is written as a single newline-terminated line under the
        // stdout mutex, so lines are never interleaved. Concurrency is bounded
        // by the runtime's own worker pool.
        let activity = std::sync::Arc::clone(&activity);
        let stdout = std::sync::Arc::clone(&stdout);
        let started = std::time::Instant::now();
        in_flight.push(runtime.spawn(async move {
            let response = handle(request.clone()).await;
            record_usage_end(&activity, &request, &caller_snapshot, started, &response);
            if let Some(response) = response {
                let mut out = stdout.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Err(error) = write_response(&mut *out, &response) {
                    tracing::warn!("mcp: write response: {error}");
                }
            }
        }));
    }

    // stdin reached EOF: wait briefly for in-flight requests so the audit log
    // receives their completion records before the process exits. Bounded so a
    // stuck request cannot delay shutdown indefinitely.
    if !in_flight.is_empty() {
        let drain = async move {
            for handle in in_flight {
                let _ = handle.await;
            }
        };
        let _ = runtime.block_on(tokio::time::timeout(
            std::time::Duration::from_secs(5),
            drain,
        ));
    }
    Ok(())
}

fn write_response(stdout: &mut impl std::io::Write, response: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, response).context("write MCP response")?;
    stdout.write_all(b"\n").context("finish MCP response")?;
    stdout.flush().context("flush MCP response")?;
    Ok(())
}

/// Remember the MCP client's identity from the `initialize` handshake so it can
/// be attached to every audit record.
fn remember_caller(request: &Value, caller: &mut String) {
    if request.get("method").and_then(Value::as_str) != Some("initialize") {
        return;
    }
    let Some(client_info) = request.pointer("/params/clientInfo") else {
        return;
    };
    let name = client_info.get("name").and_then(Value::as_str).unwrap_or("");
    let version = client_info
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("");
    *caller = if name.is_empty() {
        "unknown".to_string()
    } else if version.is_empty() {
        name.to_string()
    } else {
        format!("{name}/{version}")
    };
}

/// Record the start of a tracked request (connect, tool call, tools/list).
fn record_usage_start(activity: &ActivityLog, request: &Value, caller: &str) {
    if request.get("id").is_none() {
        return;
    }
    let method = request.get("method").and_then(Value::as_str);
    let event = match method {
        Some("tools/call") => "tool_start",
        Some("initialize") => "connect",
        Some("tools/list") => "method",
        _ => return,
    };
    activity.record(&base_record(request, caller, event, Utc::now()));
}

/// Record the completion of a `tools/call` with its duration (and error, if any).
fn record_usage_end(
    activity: &ActivityLog,
    request: &Value,
    caller: &str,
    started: std::time::Instant,
    response: &Option<Value>,
) {
    if request.get("method").and_then(Value::as_str) != Some("tools/call")
        || request.get("id").is_none()
    {
        return;
    }
    let is_error = response_is_error(response);
    let event = if is_error { "tool_error" } else { "tool_done" };
    let mut record = base_record(request, caller, event, Utc::now());
    record["duration_ms"] = json!(started.elapsed().as_millis() as u64);
    if is_error {
        record["error"] = json!(truncate(&response_error_text(response), 500));
    }
    activity.record(&record);
}

/// Build the shared audit record shape. For `tools/call`, copies the tool name
/// and only whitelisted string arguments — never the whole arguments object,
/// which could carry secrets.
fn base_record(
    request: &Value,
    caller: &str,
    event: &str,
    ts: chrono::DateTime<chrono::Utc>,
) -> Value {
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let mut record = json!({
        "ts": ts.to_rfc3339(),
        "caller": caller,
        "event": event,
        "method": method,
    });
    if let Some(id) = request.get("id") {
        record["id"] = id.clone();
    }
    if method == "tools/call" {
        if let Some(tool) = request.pointer("/params/name").and_then(Value::as_str) {
            record["tool"] = json!(tool);
        }
        const WHITELISTED_KEYS: &[&str] = &[
            "session_id",
            "group",
            "command",
            "path",
            "local_path",
            "remote_path",
            "remote_directory",
            "local_directory",
        ];
        if let Some(arguments) = request
            .pointer("/params/arguments")
            .and_then(Value::as_object)
        {
            for key in WHITELISTED_KEYS {
                if let Some(Value::String(value)) = arguments.get(*key) {
                    let text = if *key == "command" {
                        // Commands can carry inline secrets (tokens, passwords);
                        // redact them before they reach the shared audit file (#56).
                        redact_command(value)
                    } else {
                        value.clone()
                    };
                    record[*key] = json!(text);
                }
            }
        }
    }
    record
}

/// A `tools/call` response counts as an error when the response object carries
/// an `"error"` member or `/result/isError == true`. A `None` response (a
/// notification) is not an error.
fn response_is_error(response: &Option<Value>) -> bool {
    let Some(response) = response else {
        return false;
    };
    if response.get("error").is_some() {
        return true;
    }
    matches!(
        response.pointer("/result/isError"),
        Some(Value::Bool(true))
    )
}

/// Best-effort human-readable error text from a tool response.
fn response_error_text(response: &Option<Value>) -> String {
    let Some(response) = response else {
        return "unknown error".to_string();
    };
    if let Some(text) = response
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
    {
        return text.to_string();
    }
    if let Some(message) = response
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return message.to_string();
    }
    if let Some(error) = response.get("error") {
        return error.to_string();
    }
    "unknown error".to_string()
}

/// Truncate `s` to at most `max` chars.
fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Redact common inline-secret patterns from a command before it is written to
/// the shared audit file (#56). The audit log is world-readable on disk, so a
/// command like `curl -H "Authorization: Bearer ..."` or `echo pw | sudo -S`
/// must not leave the secret in plaintext. Best-effort: an unrecognized secret
/// pattern would still appear, which keeps this a redaction layer rather than a
/// guarantee.
fn redact_command(command: &str) -> String {
    let mut redacted = command.to_string();
    // Authorization: Bearer/Basic <token>
    redacted = redact(
        &redacted,
        r"(?i)(authorization\s*:\s*(?:bearer|basic)\s+)\S+",
        "$1***",
    );
    // token=/key=/secret=/password= assignments and query args.
    redacted = redact(
        &redacted,
        r#"(?i)(\b(?:token|access_token|api[_-]?key|secret|password|passwd|pwd)\b\s*[=:]\s*['"]?)[^\s'"&]+"#,
        "$1***",
    );
    // echo/printf '...' | sudo -S  (password piped into sudo/su).
    redacted = redact(
        &redacted,
        r#"(?i)((?:echo|printf)\s+['"]?)[^\s'"]+(['"]?\s*\|\s*(?:sudo|su)\s+-[sS]\b)"#,
        "$1***$2",
    );
    redacted
}

/// Apply `re` to `input`; on a pattern compile error, return `input` unchanged.
fn redact(input: &str, pattern: &str, replacement: &str) -> String {
    match Regex::new(pattern) {
        Ok(re) => re.replace_all(input, replacement).into_owned(),
        Err(_) => input.to_string(),
    }
}

async fn handle(request: Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    if id.is_none() {
        return None;
    }
    let id = id.unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        Some("initialize") => Some(success_response(id, initialize(&params))),
        Some("ping") => Some(success_response(id, json!({}))),
        Some("tools/list") => Some(success_response(
            id,
            json!({ "tools": super::tools::definitions() }),
        )),
        Some("tools/call") => Some(call_tool(id, &params).await),
        Some(_) => Some(error_response(id, -32601, "method not found")),
        None => Some(error_response(id, -32600, "invalid request")),
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    let protocol_version = SUPPORTED_PROTOCOL_VERSIONS
        .contains(&requested)
        .then_some(requested)
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    json!({
        "protocolVersion": protocol_version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "meatshell",
            "title": "MeatShell MCP",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "MeatShell exposes the SSH and SFTP sessions saved in its GUI so you can inspect and operate those hosts without ever seeing the stored credentials.\n\n\
Call list_sessions first: every other tool takes either a session's id or its display name, so the name the user says is usually enough. A session must have been connected once in the GUI, because an unknown or changed host key fails closed here with no way to accept it.\n\n\
Three switches in Settings > Interface > MCP gate the tools: saved credentials (everything that connects), arbitrary commands (run_command), and file transfers (upload_file, download_file). A disabled switch comes back as an explanatory error, so ask the user to enable it instead of retrying.\n\n\
Every call is self-contained: it connects, does one thing, and disconnects. Nothing persists between calls, not the working directory, not shell state, not background jobs, so put a whole job into a single command and detach anything that has to outlive the call.\n\n\
Never put a password, token, or private key into a command or a path: every call is appended to MeatShell's MCP activity log, which the user can read in the GUI. Move a secret with upload_file instead, then tighten its mode with run_command."
    })
}

async fn call_tool(id: Value, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, -32602, "missing tool name");
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match super::tools::call_mcp(name, &arguments).await {
        Ok(value) => success_response(
            id,
            json!({
                "content": [{ "type": "text", "text": pretty_json(&value) }],
                "structuredContent": value,
                "isError": false
            }),
        ),
        Err(error) => success_response(
            id,
            json!({
                "content": [{ "type": "text", "text": error.to_string() }],
                "isError": true
            }),
        ),
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialize_negotiates_a_supported_version() {
        let response = handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" }
        }))
        .await
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(response["result"]["serverInfo"]["name"], "meatshell");
    }

    #[tokio::test]
    async fn notifications_do_not_receive_responses() {
        assert!(handle(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await
        .is_none());
    }

    #[tokio::test]
    async fn lists_tools() {
        let response = handle(json!({
            "jsonrpc": "2.0",
            "id": "tools",
            "method": "tools/list"
        }))
        .await
        .unwrap();
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 7);
    }
}
