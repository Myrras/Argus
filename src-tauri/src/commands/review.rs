//! Review commands: the list, one review and refreshing. (Submitting a
//! review is a write: `commands::post::submit_review`.)

use tauri::{AppHandle, State};

use argus_core::{RefreshReport, ReviewRecord};

use super::{blocking, parse_review_id};
use crate::error::IpcError;
use crate::events;
use crate::state::AppState;

/// The reviews shown in the tabs (the core's `is_active`), most recently
/// updated first. The frontend splits them by `status`.
#[tauri::command]
pub async fn list_reviews(state: State<'_, AppState>) -> Result<Vec<ReviewRecord>, IpcError> {
    blocking(state, |core| {
        Ok(core
            .reviews()?
            .into_iter()
            .filter(ReviewRecord::is_active)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn get_review(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<ReviewRecord, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.review(id)).await
}

/// Refresh every provider now (blocking, off the IPC thread), emit the same
/// events a poll does and let the AI pass queue its runs, as after a poll.
/// Per-provider failures are in the report, not an error: one provider
/// failing never fails the call.
#[tauri::command]
pub async fn refresh_reviews(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RefreshReport, IpcError> {
    let last = state.last_refresh.clone();
    let report = blocking(state, |core| {
        let report = core.refresh();
        // Quick (SQLite reads, queueing): it never waits on a run.
        core.on_refresh_report(&report);
        Ok(report)
    })
    .await?;
    AppState::record(&last, &report);
    events::emit_report(&app, &report);
    Ok(report)
}

/// The last refresh, poll or manual; `null` before the first one finished.
#[tauri::command]
pub fn last_refresh(state: State<'_, AppState>) -> Option<RefreshReport> {
    state
        .last_refresh
        .lock()
        .expect("last refresh lock poisoned")
        .clone()
}
