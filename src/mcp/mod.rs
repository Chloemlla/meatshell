//! Model Context Protocol feature.
//!
//! MCP transport, tools, resources, and protocol adapters belong in this module.

#[path = "impls/activity.rs"]
mod activity;
#[path = "impls/server.rs"]
mod server;
#[path = "impls/tools.rs"]
mod tools;

pub(crate) use activity::ActivityLog;
pub(crate) use server::run_stdio;

/// Read the newest `limit` audit records (oldest→newest). Consumed by the GUI
/// to show live MCP usage from the separate MCP stdio process.
#[allow(dead_code)]
pub(crate) fn tail_activity(limit: usize) -> Vec<serde_json::Value> {
    ActivityLog::open().tail(limit)
}

/// Truncate the audit log (not delete), so a running MCP process keeps appending.
#[allow(dead_code)]
pub(crate) fn clear_activity() {
    let _ = std::fs::File::create(crate::config::data_dir().join("mcp_activity.jsonl"));
}
