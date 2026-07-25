use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::terminal_struct::TabRenderGate;

impl TabRenderGate {
    pub(crate) fn new(min_interval: std::time::Duration) -> Self {
        Self {
            scheduled: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            last_render: Mutex::new(std::time::Instant::now() - min_interval),
        }
    }
}
