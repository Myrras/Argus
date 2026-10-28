//! Posting commands: new line comments, replies, approvals and resolving
//! threads. The only
//! commands that write to a platform; each runs on one explicit user action
//! (a button or shortcut in the UI), never on a poll or a timer.
//!
//! After a successful write the review is re-read by the core; the shell
//! emits `review:changed` so the stores re-read it too.

use tauri::{AppHandle, State};

use argus_core::{
    CommentableLines, NewComment, PostErrorKind, PostResult, PostedNote, ReviewId, ReviewRecord,
    ReviewVerdict, StoredThread,
};

use super::{blocking, blocking_post, parse_review_id, parse_thread_id};
use crate::error::IpcError;
use crate::events;
use crate::state::AppState;

/// After a `headMoved` refusal the core stored the review's new head: the
/// stores re-read it, so the user retries from what is there now.
pub(crate) fn emit_if_head_moved<T>(app: &AppHandle, id: ReviewId, result: &Result<T, IpcError>) {
    if result
        .as_ref()
        .is_err_and(|e| e.kind == PostErrorKind::HeadMoved.as_str())
    {
        events::emit_changed(app, id);
    }
}

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
    let results = blocking_post(state, move |core| core.post_comments(id, comments)).await;
    emit_if_head_moved(&app, id, &results);
    let results = results?;
    if changes_review(&results) {
        events::emit_changed(&app, id);
    }
    Ok(results)
}

/// Whether the core stored something new for the review: a comment was
/// posted, or one was refused for a push (GitLab's version check; the core
/// stored the new head).
fn changes_review(results: &[PostResult]) -> bool {
    results.iter().any(|r| match r {
        PostResult::Posted(_) => true,
        PostResult::Failed(e) => e.kind == PostErrorKind::HeadMoved,
    })
}

/// Reply to a stored thread of the review `review_id`. The thread's review
/// is looked up, not taken from the caller: a thread of another review is
/// refused before anything is posted, and `review:changed` names the review
/// that really changed.
#[tauri::command]
pub async fn reply_to_thread(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    thread_id: String,
    body: String,
) -> Result<PostedNote, IpcError> {
    let claimed = parse_review_id(&review_id)?;
    let thread = parse_thread_id(&thread_id)?;
    let (id, note) = blocking_post(state, move |core| {
        let id = core.thread_review(thread)?;
        check_thread_review(claimed, id)?;
        Ok((id, core.reply_to_thread(thread, &body)?))
    })
    .await?;
    events::emit_changed(&app, id);
    Ok(note)
}

fn check_thread_review(claimed: ReviewId, actual: ReviewId) -> Result<(), argus_core::PostError> {
    if claimed == actual {
        Ok(())
    } else {
        Err(argus_core::PostError::new(
            PostErrorKind::InvalidInput,
            "the thread is not on this review",
        ))
    }
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
    let record = blocking_post(state, move |core| core.approve_review(id, &head_sha)).await;
    emit_if_head_moved(&app, id, &record);
    let record = record?;
    events::emit_changed(&app, record.id);
    Ok(record)
}

/// Resolve (`resolved: true`) or reopen a stored thread of the review
/// `review_id` on its platform; the thread as stored after the refresh. A
/// thread of another review, or one its platform cannot resolve, is refused
/// before anything is written.
#[tauri::command]
pub async fn resolve_thread(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    thread_id: String,
    resolved: bool,
) -> Result<StoredThread, IpcError> {
    let id = parse_review_id(&review_id)?;
    let thread = parse_thread_id(&thread_id)?;
    let stored =
        blocking_post(state, move |core| core.resolve_thread(id, thread, resolved)).await?;
    events::emit_changed(&app, stored.review_id);
    Ok(stored)
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

// --- Submit review ---------------------------------------------------------

/// Submit the user's review: `verdict` (approve, request changes, or on
/// GitHub a comment review) pinned to `head_sha` (the head shown in the
/// dialog), with an optional markdown `summary`. One review-state write on
/// the platform (plus the summary note on GitLab); `headMoved` when the
/// review got new commits since, `unsupported` when the platform cannot.
/// The pending comments are posted before it by `post_comments`.
#[tauri::command]
pub async fn submit_review(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    head_sha: String,
    verdict: ReviewVerdict,
    summary: Option<String>,
) -> Result<ReviewRecord, IpcError> {
    let id = parse_review_id(&review_id)?;
    let record = blocking_post(state, move |core| {
        core.submit_review(id, verdict, &head_sha, summary.as_deref())
    })
    .await;
    emit_if_head_moved(&app, id, &record);
    let record = record?;
    events::emit_changed(&app, record.id);
    Ok(record)
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
    fn a_reply_names_the_threads_own_review() {
        let a = argus_core::ReviewId::from_key("github|github.com|acme-fixture/widgets|12");
        let b = argus_core::ReviewId::from_key("github|github.com|acme-fixture/widgets|13");
        assert!(check_thread_review(a, a).is_ok());
        let err = IpcError::from(check_thread_review(a, b).unwrap_err());
        assert_eq!(err.kind, "invalidInput");
    }

    #[test]
    fn a_posted_comment_or_a_moved_head_changes_the_review() {
        let failed = |kind| PostResult::Failed(argus_core::PostError::new(kind, "x"));
        assert!(!changes_review(&[]));
        assert!(!changes_review(&[failed(PostErrorKind::LineNotInDiff)]));
        assert!(changes_review(&[
            failed(PostErrorKind::LineNotInDiff),
            failed(PostErrorKind::HeadMoved),
        ]));
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
