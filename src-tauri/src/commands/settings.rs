//! Settings commands. The GitLab token has its own command and never travels
//! in `Settings`; only whether one is stored ever comes back.

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

/// Store the token for the configured GitLab URL in the keychain; a blank
/// token deletes it.
#[tauri::command]
pub async fn set_gitlab_token(state: State<'_, AppState>, token: String) -> Result<(), IpcError> {
    blocking(state, move |core| core.set_gitlab_token(&token)).await
}

#[tauri::command]
pub async fn gitlab_token_present(state: State<'_, AppState>) -> Result<bool, IpcError> {
    blocking(state, |core| Ok(core.gitlab_token_present())).await
}
