use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ledger::Ledger;

/// Shared application state passed to all axum handlers via State extractor.
pub struct AppState {
    pub ledger: Mutex<Ledger>,
    pub sys: Mutex<sysinfo::System>,
    pub state_dir: String,
}

/// Type alias for the shared state used across the application.
pub type SharedState = Arc<AppState>;
