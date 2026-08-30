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

/// Clear the audit log. The old file is moved aside and a fresh one created, so
/// a running MCP process keeps appending and a reader never sees a half-written
/// file (#57).
#[allow(dead_code)]
pub(crate) fn clear_activity() {
    ActivityLog::open().clear();
}
