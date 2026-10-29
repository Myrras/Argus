//! The user's accounts (Settings → Accounts, the sidebar's Providers).

use tauri::State;

use argus_core::Account;

use super::blocking;
use crate::error::IpcError;
use crate::state::AppState;

/// Every host `gh` and `glab` are logged in to (plus the ones the settings
/// name), with the user's choice. The CLIs are asked (a request per host)
/// only when the core's answer is not current, or when `recheck` (Check
/// again); see `Core::list_accounts`. Off the IPC thread.
#[tauri::command]
pub async fn list_accounts(
    state: State<'_, AppState>,
    recheck: bool,
) -> Result<Vec<Account>, IpcError> {
    blocking(state, move |core| Ok(core.list_accounts(recheck))).await
}
