//! Thread commands.

use tauri::State;

use argus_core::StoredThread;

use super::{blocking, parse_review_id};
use crate::error::IpcError;
use crate::state::AppState;

/// The stored threads of a review (as of the last refresh), oldest first.
#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<StoredThread>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.threads(id)).await
}
