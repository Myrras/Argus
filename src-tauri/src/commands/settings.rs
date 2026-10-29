//! Settings commands. No token travels here: GitHub's stays with `gh`,
//! GitLab's with `glab`.

use tauri::State;

use argus_core::Settings;

use super::blocking;
use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.core.settings()
}

/// Validate, save and apply; returns what was stored. Off the IPC thread:
/// restarting the poll loop waits for an in-flight refresh.
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, IpcError> {
    blocking(state, move |core| core.update_settings(settings)).await
}
