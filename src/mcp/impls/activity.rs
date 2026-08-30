//! Append-only JSONL audit log of MCP activity.
//!
//! The MCP server runs as a separate stdio process from the GUI, so the only
//! way for the GUI to observe what MCP was used for is through a shared file:
//! `<config_dir>/mcp_activity.jsonl`. This module appends one structured JSON
//! record per line and lets the GUI tail back the newest records.

use std::io::Write as _;
use std::path::PathBuf;

use serde_json::Value;

/// Cap the audit file at ~2 MiB before trimming it back down.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// When the cap is hit, keep only the newest 1000 records (plus the new one).
const RETAIN_LINES: usize = 1000;

/// A best-effort, append-only JSONL log. `lock` serializes concurrent appends
/// and rewrites within this process (the file is shared with the GUI, which
/// only ever reads it).
pub(crate) struct ActivityLog {
    path: PathBuf,
    lock: std::sync::Mutex<()>,
}

impl ActivityLog {
    /// The shared audit file in the app's data directory.
    pub(crate) fn open() -> Self {
        Self {
            path: crate::config::data_dir().join("mcp_activity.jsonl"),
            lock: std::sync::Mutex::new(()),
        }
    }

    /// Test-only constructor pointing at an arbitrary file.
    #[cfg(test)]
    pub(crate) fn with_path(path: PathBuf) -> Self {
        Self {
            path,
            lock: std::sync::Mutex::new(()),
        }
    }

    /// Append one record as a JSONL line, trimming the file back when it would
    /// exceed [`MAX_BYTES`]. Best-effort: failures are logged, never panicked.
    pub(crate) fn record(&self, event: &Value) {
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let line = match serde_json::to_string(event) {
            Ok(line) => line,
            Err(error) => {
                tracing::warn!("mcp activity: serialize record: {error}");
                return;
            }
        };
        let record = format!("{line}\n");
        if let Err(error) = self.append_capped(&record, MAX_BYTES, RETAIN_LINES) {
            tracing::warn!("mcp activity: append record: {error}");
        }
    }

    /// Append `record`, or — when the file would grow past `max_bytes` — rewrite
    /// it keeping only the newest `retain_lines` records plus this one.
    fn append_capped(
        &self,
        record: &str,
        max_bytes: u64,
        retain_lines: usize,
    ) -> std::io::Result<()> {
        let existing = std::fs::metadata(&self.path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        if existing + record.len() as u64 <= max_bytes {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            file.write_all(record.as_bytes())?;
            return Ok(());
        }

        // Over the cap: keep the newest `retain_lines` records, then this one.
        let new_line = record.trim_end_matches('\n');
        let mut body: Vec<String> = self
            .read_records(retain_lines)
            .into_iter()
            .filter_map(|value| serde_json::to_string(&value).ok())
            .collect();
        body.push(new_line.to_string());
        let mut body = body.join("\n");
        body.push('\n');
        std::fs::write(&self.path, body)
    }

    /// Read the file and return the newest up to `limit` records in
    /// oldest→newest order. Unparseable lines are skipped.
    pub(crate) fn tail(&self, limit: usize) -> Vec<Value> {
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.read_records(limit)
    }

    /// Assumes the caller already holds `lock`. Reads and parses the file,
    /// returning the newest up to `limit` records in oldest→newest order.
    fn read_records(&self, limit: usize) -> Vec<Value> {
        let Ok(raw) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let mut records: Vec<Value> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect();
        let start = records.len().saturating_sub(limit);
        records.drain(..start);
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn appends_records_and_tails_in_order() {
        let path = std::env::temp_dir().join("ms-mcp-activity-test.jsonl");
        let _ = std::fs::remove_file(&path);
        let log = ActivityLog::with_path(path.clone());

        log.record(&json!({ "event": "connect", "method": "initialize", "id": 1 }));
        log.record(&json!({
            "event": "tool_done",
            "method": "tools/call",
            "id": 2,
            "tool": "run_command",
            "command": "ls -la",
        }));
        log.record(&json!({ "event": "tool_error", "method": "tools/call", "id": 3 }));

        let tail = log.tail(10);
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0]["event"], "connect");
        assert_eq!(tail[0]["id"], 1);
        assert_eq!(tail[1]["event"], "tool_done");
        assert_eq!(tail[1]["command"], "ls -la");
        assert_eq!(tail[2]["event"], "tool_error");
        assert_eq!(tail[2]["id"], 3);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trims_to_the_newest_records_when_over_cap() {
        let path = std::env::temp_dir().join("ms-mcp-activity-cap-test.jsonl");
        let _ = std::fs::remove_file(&path);
        let log = ActivityLog::with_path(path.clone());

        // Tiny cap forces the rewrite path from the second record on; a small
        // retain proves old records actually get dropped.
        for i in 0..20 {
            let line = serde_json::to_string(&json!({ "n": i })).unwrap();
            log.append_capped(&format!("{line}\n"), 16, 5).unwrap();
        }

        let tail = log.tail(1000);
        assert_eq!(tail.len(), 6);
        assert_eq!(tail[0]["n"], 14);
        assert_eq!(tail.last().unwrap()["n"], 19);

        let _ = std::fs::remove_file(&path);
    }
}
