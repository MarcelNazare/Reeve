use crate::catalog::AppRecord;
use anyhow::Result;

pub enum Lifecycle {
    Spawned,
    Ready,
    Backgrounded,
    Suspended,
    Killed,
}

pub struct AppInstance {
    pub app: AppRecord,
    pub lifecycle: Lifecycle,
    pub session_id: i64,
}

impl AppInstance {
    pub fn spawn(app: AppRecord, session_id: i64) -> Result<Self> {
        Ok(Self { app, lifecycle: Lifecycle::Spawned, session_id })
    }

    pub fn transition(&mut self, next: Lifecycle) {
        // Later: hook Job Object attach here when the renderer PID is known.
        self.lifecycle = next;
    }
}