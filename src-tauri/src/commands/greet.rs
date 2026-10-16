//! The canonical sample command: a pure call into the core.

use tauri::State;

use crate::error::IpcError;
use crate::state::AppState;

/// Greet `name`. Returns the core's greeting, or an `invalidInput` error for
/// blank input — the smallest end-to-end example of the IPC contract:
/// deserialize args → call core → map `CoreError` → serialize result.
#[tauri::command]
pub fn greet(state: State<'_, AppState>, name: String) -> Result<String, IpcError> {
    Ok(state.core.greet(&name)?)
}
