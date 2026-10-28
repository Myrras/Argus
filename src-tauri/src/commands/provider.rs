//! Provider status for the status bar.

use tauri::State;

use argus_core::ProviderStatus;

use super::blocking;
use crate::error::IpcError;
use crate::state::AppState;

/// `gh` installed and logged in, GitLab configured with a token. Runs `gh
/// auth token` (local) and reads the keychain, so off the IPC thread.
#[tauri::command]
pub async fn provider_status(state: State<'_, AppState>) -> Result<ProviderStatus, IpcError> {
    blocking(state, |core| Ok(core.provider_status())).await
}
