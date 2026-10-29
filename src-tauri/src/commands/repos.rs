//! Settings → Repositories: every repository Argus knows with its setup.
//! The setup itself is saved through `update_settings`.

use tauri::State;

use argus_core::RepoSetup;

use super::blocking;
use crate::error::IpcError;
use crate::state::AppState;

/// Every repository of a stored review or of the settings, with its clone,
/// review skill and review language. Read-only; looks at the clones' folders,
/// so it runs off the IPC thread.
#[tauri::command]
pub async fn list_repositories(state: State<'_, AppState>) -> Result<Vec<RepoSetup>, IpcError> {
    blocking(state, |core| core.list_repositories()).await
}

/// Show the clone set for `repo` (`host/repo`) in Finder / Explorer. The
/// core resolves the folder from the settings.
#[tauri::command]
pub async fn reveal_repo_clone(state: State<'_, AppState>, repo: String) -> Result<(), IpcError> {
    blocking(state, move |core| core.reveal_repo_clone(&repo)).await
}
