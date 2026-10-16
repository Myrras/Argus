//! Posting commands: new line comments, replies and approvals. The only
//! commands that write to a platform; each runs on one explicit user action
//! (a button or shortcut in the UI), never on a poll or a timer.
//!
//! After a successful write the review is re-read by the core; the shell
//! emits `review:changed` so the stores re-read it too.

use tauri::{AppHandle, State};

use argus_core::{CommentableLines, NewComment, PostResult, PostedNote, ReviewRecord};

use super::{blocking, blocking_post, parse_review_id, parse_thread_id};
use crate::error::IpcError;
use crate::events;
use crate::state::AppState;

/// Post `comments` on a review, one by one; one result per comment in the
/// same order (`posted` or `failed`). `Err` only when nothing could be
/// tried (unknown review, the platform's head or diff unreadable).
#[tauri::command]
pub async fn post_comments(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    comments: Vec<NewComment>,
) -> Result<Vec<PostResult>, IpcError> {
    let id = parse_review_id(&review_id)?;
    let results = blocking_post(state, move |core| core.post_comments(id, comments)).await?;
    if results.iter().any(PostResult::is_posted) {
        events::emit_changed(&app, id);
    }
    Ok(results)
}

/// Reply to a stored thread of the review `review_id` (the review is only
/// used to tell the stores which review changed).
#[tauri::command]
pub async fn reply_to_thread(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    thread_id: String,
    body: String,
) -> Result<PostedNote, IpcError> {
    let id = parse_review_id(&review_id)?;
    let thread = parse_thread_id(&thread_id)?;
    let note = blocking_post(state, move |core| core.reply_to_thread(thread, &body)).await?;
    events::emit_changed(&app, id);
    Ok(note)
}

/// Approve a review, pinned to `head_sha` (the head the user confirmed);
/// `headMoved` when the review got new commits since. Never merges.
#[tauri::command]
pub async fn approve_review(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    head_sha: String,
) -> Result<ReviewRecord, IpcError> {
    let id = parse_review_id(&review_id)?;
    let record = blocking_post(state, move |core| core.approve_review(id, &head_sha)).await?;
    events::emit_changed(&app, record.id);
    Ok(record)
}

/// The lines of `path` a new comment can go on, per hunk of the review's
/// diff (what the code view shows).
#[tauri::command]
pub async fn commentable_lines(
    state: State<'_, AppState>,
    review_id: String,
    path: String,
) -> Result<CommentableLines, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.commentable_lines(id, &path)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use argus_core::ThreadId;

    #[test]
    fn parses_thread_ids_without_echoing_input() {
        let review = argus_core::ReviewId::from_key("github|github.com|acme-fixture/widgets|12");
        let id = ThreadId::from_key(review, "4001");
        assert_eq!(parse_thread_id(&id.to_string()).unwrap(), id);
        let err = parse_thread_id("../4001").unwrap_err();
        assert_eq!(err.kind, "invalidInput");
        assert!(!err.message.contains("4001"));
    }

    #[test]
    fn post_results_keep_their_wire_shape() {
        let failed = PostResult::Failed(argus_core::PostError::new(
            argus_core::PostErrorKind::LineNotInDiff,
            "That line is not in the diff any more.",
        ));
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["kind"], "lineNotInDiff");
        let input: NewComment = serde_json::from_value(serde_json::json!({
            "path": "src/a.rs", "line": 4, "side": "new", "body": "x"
        }))
        .unwrap();
        assert_eq!(input.start_line, None);
        assert_eq!(input.head_sha, None);
    }
}
