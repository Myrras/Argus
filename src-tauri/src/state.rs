//! Application state shared across all IPC commands.
//!
//! One [`Core`] lives for the lifetime of the app, behind an [`AppState`]
//! handed to Tauri via `.manage(...)` in the setup hook (the data dir is only
//! known there) and pulled into commands as `State<'_, AppState>`. The `Arc`s
//! let detached tasks (`spawn_blocking`, the poll callback) hold the core
//! without borrowing Tauri state.

use std::path::Path;
use std::sync::{Arc, Mutex};

use argus_core::{Core, CoreResult, RefreshReport};

/// The SQLite review store, in the app data dir.
pub const DB_FILE: &str = "argus.db";
/// Hosts, polling, AI on/off. Never secrets.
pub const SETTINGS_FILE: &str = "settings.json";

/// Live application state.
pub struct AppState {
    pub core: Arc<Core>,
    /// The last refresh (poll or manual). Kept here so a webview that starts
    /// listening after the first poll finished still learns its time and
    /// errors (`last_refresh`).
    pub last_refresh: Arc<Mutex<Option<RefreshReport>>>,
}

impl AppState {
    /// Open the core over `data_dir/argus.db` and `data_dir/settings.json`.
    pub fn open(data_dir: &Path) -> CoreResult<Self> {
        let core = Core::open(&data_dir.join(DB_FILE), &data_dir.join(SETTINGS_FILE))?;
        Ok(Self::with_core(core))
    }

    pub fn with_core(core: Core) -> Self {
        Self {
            core: Arc::new(core),
            last_refresh: Arc::new(Mutex::new(None)),
        }
    }

    /// Remember `report` as the last refresh.
    pub fn record(last: &Mutex<Option<RefreshReport>>, report: &RefreshReport) {
        *last.lock().expect("last refresh lock poisoned") = Some(report.clone());
    }
}
