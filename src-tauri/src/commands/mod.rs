//! IPC command modules. Each command is a thin (de)serialization shim over
//! [`argus_core::Core`]; no domain logic lives here.

pub mod ai;
pub mod files;
pub mod history;
pub mod post;
pub mod provider;
pub mod review;
pub mod settings;
pub mod thread;

use std::sync::Arc;

use tauri::State;

use argus_core::{Core, CoreResult, PostError, ReviewId, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

/// Run a core call on a blocking thread: provider calls (network, `gh`), the
/// keychain, SQLite (which may wait on a poll's write) and stopping the poll
/// loop (which waits for an in-flight refresh) must never hold up the
/// webview's IPC thread.
pub(crate) async fn blocking<T: Send + 'static>(
    state: State<'_, AppState>,
    f: impl FnOnce(&Core) -> CoreResult<T> + Send + 'static,
) -> Result<T, IpcError> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(join_failed)?
        .map_err(IpcError::from)
}

/// [`blocking`] for the post API, whose calls fail with a [`PostError`].
pub(crate) async fn blocking_post<T: Send + 'static>(
    state: State<'_, AppState>,
    f: impl FnOnce(&Core) -> Result<T, PostError> + Send + 'static,
) -> Result<T, IpcError> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(join_failed)?
        .map_err(IpcError::from)
}

/// A blocking task that panicked (or was cancelled). Its `Display` carries
/// the panic message, which is not app-owned text: it stays in the log at
/// `DEBUG`, and the webview gets a fixed message of kind `internal`.
fn join_failed(e: impl std::fmt::Display) -> IpcError {
    tracing::debug!("a background task failed: {e}");
    IpcError::new("internal", "Something went wrong inside Argus; try again.")
}

/// Ids arrive as plain UUID strings; a malformed one is an IPC-layer input
/// error with no `CoreError` counterpart. `field` names the argument (the
/// raw input is never echoed back).
pub(crate) fn parse_id<T: std::str::FromStr>(raw: &str, field: &str) -> Result<T, IpcError> {
    raw.trim()
        .parse()
        .map_err(|_| IpcError::invalid_input(format!("{field} is not a valid id")))
}

pub(crate) fn parse_review_id(raw: &str) -> Result<ReviewId, IpcError> {
    parse_id(raw, "reviewId")
}

pub(crate) fn parse_thread_id(raw: &str) -> Result<ThreadId, IpcError> {
    parse_id(raw, "threadId")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_task_never_shows_its_panic_message() {
        let err = join_failed("panicked at 'token ghp_fixture in a path'");
        assert_eq!(err.kind, "internal");
        assert!(!err.message.contains("ghp_fixture"));
        assert!(!err.message.contains("panicked"));
    }

    #[test]
    fn parses_review_ids() {
        let id = ReviewId::from_key("github|github.com|acme-fixture/widgets|12");
        assert_eq!(parse_review_id(&id.to_string()).unwrap(), id);
        assert_eq!(parse_review_id(&format!(" {id} ")).unwrap(), id);
        let err = parse_review_id("not-a-uuid").unwrap_err();
        assert_eq!(err.kind, "invalidInput");
        // The raw input is never echoed back.
        assert!(!err.message.contains("not-a-uuid"));
    }
}
