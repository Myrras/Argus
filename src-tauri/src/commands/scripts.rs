//! Submit scripts: "Write with AI" drafts (start, follow up, stop, list,
//! discard, link; whether it can run),
//! saving a generated script in Argus' own `scripts/` folder, reading one
//! back to change it, and that folder. None of these runs a script or
//! writes to a platform: scripts run only after a successful
//! `submit_review` with their ids (see `post.rs`).

use tauri::State;

use argus_core::{ScriptDraft, ScriptGenerationStatus};

use super::blocking;
use crate::error::IpcError;
use crate::state::AppState;

/// Whether "Write with AI" can run (the AI pass on, its agent found), and
/// if not, why.
#[tauri::command]
pub async fn script_generation_status(
    state: State<'_, AppState>,
) -> Result<ScriptGenerationStatus, IpcError> {
    blocking(state, |core| Ok(core.script_generation_status())).await
}

/// Start a "Write with AI" draft whose first request is `description`;
/// with `scriptId` ("Edit with AI") it starts from that saved script.
/// Returns at once with the draft running; `script-draft:state` reports
/// the rest. Writes nothing.
#[tauri::command]
pub async fn start_script_draft(
    state: State<'_, AppState>,
    description: String,
    script_id: Option<String>,
) -> Result<ScriptDraft, IpcError> {
    blocking(state, move |core| {
        core.start_script_draft(&description, script_id.as_deref())
    })
    .await
}

/// A follow-up request on a draft (kind `conflict` while it runs).
#[tauri::command]
pub async fn send_script_draft(
    state: State<'_, AppState>,
    draft_id: String,
    message: String,
) -> Result<ScriptDraft, IpcError> {
    blocking(state, move |core| {
        core.send_script_draft(&draft_id, &message)
    })
    .await
}

/// Send a draft's last request again after it failed or was stopped
/// (Retry; kind `conflict` while it runs or when it has its script).
#[tauri::command]
pub async fn retry_script_draft(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<ScriptDraft, IpcError> {
    blocking(state, move |core| core.retry_script_draft(&draft_id)).await
}

/// Stop a draft's request under way (it ends `cancelled`).
#[tauri::command]
pub async fn cancel_script_draft(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<ScriptDraft, IpcError> {
    blocking(state, move |core| core.cancel_script_draft(&draft_id)).await
}

/// Every draft of this session, oldest first.
#[tauri::command]
pub async fn list_script_drafts(state: State<'_, AppState>) -> Result<Vec<ScriptDraft>, IpcError> {
    blocking(state, |core| Ok(core.list_script_drafts())).await
}

/// Forget a draft, stopping it first when it runs.
#[tauri::command]
pub async fn discard_script_draft(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<(), IpcError> {
    blocking(state, move |core| core.discard_script_draft(&draft_id)).await
}

/// Link a draft to the submit script saved from it.
#[tauri::command]
pub async fn link_script_draft(
    state: State<'_, AppState>,
    draft_id: String,
    script_id: String,
) -> Result<ScriptDraft, IpcError> {
    blocking(state, move |core| {
        core.link_script_draft(&draft_id, &script_id)
    })
    .await
}

/// Save a script in Argus' scripts folder as `<slug of name>.sh`,
/// executable; kind `conflict` when that file exists and `overwrite` is not
/// set. The saved file's path.
#[tauri::command]
pub async fn save_submit_script(
    state: State<'_, AppState>,
    name: String,
    content: String,
    overwrite: Option<bool>,
) -> Result<String, IpcError> {
    let overwrite = overwrite.unwrap_or(false);
    blocking(state, move |core| {
        core.save_submit_script(&name, &content, overwrite)
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
}

/// The text of a script Argus saved (to change it with the AI's help).
#[tauri::command]
pub async fn read_submit_script(
    state: State<'_, AppState>,
    script_id: String,
) -> Result<String, IpcError> {
    blocking(state, move |core| core.read_submit_script(&script_id)).await
}

/// Where Argus saves scripts; `null` when it has no folder.
#[tauri::command]
pub async fn submit_scripts_dir(state: State<'_, AppState>) -> Result<Option<String>, IpcError> {
    blocking(state, |core| {
        Ok(core
            .submit_scripts_dir()
            .map(|p| p.to_string_lossy().into_owned()))
    })
    .await
}

/// Show the scripts folder in Finder / Explorer.
#[tauri::command]
pub async fn reveal_submit_scripts_dir(state: State<'_, AppState>) -> Result<(), IpcError> {
    blocking(state, |core| core.reveal_submit_scripts_dir()).await
}
