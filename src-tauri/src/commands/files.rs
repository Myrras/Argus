//! Code view commands: the changed files, a file's contents and viewed
//! marks. All read from the provider, so all run off the IPC thread.

use tauri::State;

use argus_core::ReviewFiles;

use super::{blocking, parse_review_id};
use crate::error::IpcError;
use crate::state::AppState;

/// The review's changed files with their viewed marks: its own diff, or the
/// diff since the commit last marked reviewed.
#[tauri::command]
pub async fn list_files(
    state: State<'_, AppState>,
    review_id: String,
    since_last_review: bool,
) -> Result<ReviewFiles, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.list_files(id, since_last_review)).await
}

/// The content of `path` at `sha` (one of the shas `list_files` returned).
#[tauri::command]
pub async fn file_contents(
    state: State<'_, AppState>,
    review_id: String,
    sha: String,
    path: String,
) -> Result<String, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.file_at(id, &sha, &path)).await
}

/// Mark a changed file viewed or not; returns the review's viewed paths.
#[tauri::command]
pub async fn set_file_viewed(
    state: State<'_, AppState>,
    review_id: String,
    path: String,
    viewed: bool,
) -> Result<Vec<String>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.set_file_viewed(id, &path, viewed)).await
}
