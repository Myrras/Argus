//! Application state shared across all IPC commands.
//!
//! One [`Core`] lives for the lifetime of the app, behind an [`AppState`] handed
//! to Tauri via `.manage(...)` and pulled into commands as
//! `State<'_, AppState>`. The `Arc` lets detached tasks hold the core without
//! borrowing Tauri state.

use std::sync::Arc;

use argus_core::Core;

/// Live application state: the argus-core entry point.
pub struct AppState {
    pub core: Arc<Core>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            core: Arc::new(Core::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
