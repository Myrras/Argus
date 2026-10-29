//! Open in editor: a review's files in its local clone. Paths are the
//! review's repository paths; the core resolves them inside the clone and
//! never takes an absolute path from the webview.

use std::path::PathBuf;

use tauri::State;

use argus_core::{CloneStatus, CloneSuggestion, LocalFile, Settings};

use super::{blocking, parse_review_id};
use crate::error::IpcError;
use crate::state::AppState;

/// The review's `paths` in its local clone: whether each is in the working
/// tree, and whether the clone is on the review's head. `None` when no
/// clone is set for the repository.
#[tauri::command]
pub async fn local_files(
    state: State<'_, AppState>,
    review_id: String,
    paths: Vec<String>,
) -> Result<Option<Vec<LocalFile>>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.local_files(id, &paths)).await
}

/// Open the review's file `path` from the clone in its editor (Settings →
/// Editors), or with `useDefault` in the OS default app.
#[tauri::command]
pub async fn open_in_editor(
    state: State<'_, AppState>,
    review_id: String,
    path: String,
    use_default: Option<bool>,
) -> Result<(), IpcError> {
    let id = parse_review_id(&review_id)?;
    let use_default = use_default.unwrap_or(false);
    blocking(state, move |core| {
        core.open_in_editor(id, &path, use_default)
    })
    .await
}

/// Show the review's file `path` from the clone in Finder / Explorer.
#[tauri::command]
pub async fn reveal_local_file(
    state: State<'_, AppState>,
    review_id: String,
    path: String,
) -> Result<(), IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.reveal_local_file(id, &path)).await
}

/// What is set and found for the review's repository: not configured, the
/// folder gone or no repository, or a repository with its head. Read-only.
#[tauri::command]
pub async fn clone_status(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<CloneStatus, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.clone_status(id)).await
}

/// Show the review's clone folder in Finder / Explorer.
#[tauri::command]
pub async fn reveal_clone(state: State<'_, AppState>, review_id: String) -> Result<(), IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.reveal_clone(id)).await
}

/// A clone of the review's repository found on disk, for a repository with
/// none set that the user did not refuse; `null` otherwise. Reads the disk,
/// so it runs off the UI thread like the rest.
#[tauri::command]
pub async fn suggest_clone(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Option<CloneSuggestion>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.suggest_clone(id)).await
}

/// Store `path` as the clone of the review's repository ("Use it"); the
/// settings afterwards. The core checks the folder is that repository's.
#[tauri::command]
pub async fn use_clone_suggestion(
    state: State<'_, AppState>,
    review_id: String,
    path: String,
) -> Result<Settings, IpcError> {
    let id = parse_review_id(&review_id)?;
    let path = PathBuf::from(path);
    blocking(state, move |core| core.use_clone_suggestion(id, &path)).await
}

/// Never offer a found clone for the review's repository again ("No
/// thanks"); the settings afterwards.
#[tauri::command]
pub async fn ignore_clone_suggestion(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Settings, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.ignore_clone_suggestion(id)).await
}
