//! Notes commands: a minimal add / list / remove trio over the core's state.

use tauri::State;

use argus_core::{Note, NoteId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
pub fn note_add(state: State<'_, AppState>, text: String) -> Result<Note, IpcError> {
    Ok(state.core.add_note(&text)?)
}

#[tauri::command]
pub fn note_list(state: State<'_, AppState>) -> Vec<Note> {
    state.core.list_notes()
}

#[tauri::command]
pub fn note_remove(state: State<'_, AppState>, note_id: String) -> Result<(), IpcError> {
    // Ids arrive as plain UUID strings; a malformed one is an IPC-layer input
    // error with no `CoreError` counterpart.
    let id: NoteId = note_id
        .parse()
        .map_err(|_| IpcError::invalid_input("note_id is not a valid id"))?;
    Ok(state.core.remove_note(id)?)
}
