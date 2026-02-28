use std::sync::Arc;
use tokio::sync::Mutex;

use crate::approval::ApprovalGate;
use crate::ledger::Ledger;
use crate::sandbox::SandboxEngine;

/// Shared application state passed to all axum handlers via State extractor.
pub struct AppState {
    pub ledger: Mutex<Ledger>,
    pub sys: Mutex<sysinfo::System>,
    pub state_dir: String,
    pub approval_gate: Option<Arc<ApprovalGate>>,
    pub sandbox_engine: Option<Arc<SandboxEngine>>,
}

/// Type alias for the shared state used across the application.
pub type SharedState = Arc<AppState>;
