//! Thread history commands (M3): a thread's then/now code, the diff between
//! them and its timeline. Read only towards the platforms.
//!
//! A history call that computes walks also rewrites the review's untouched
//! count (the walk knows better than the refresh's approximation). The
//! review list shows that count, so when it moved the shell emits
//! `review:changed` and the stores re-read the review.

use tauri::{AppHandle, State};

use argus_core::{CoreResult, ThreadHistory};

use super::{blocking, parse_review_id, parse_thread_id};
use crate::error::IpcError;
use crate::events;
use crate::state::AppState;

/// The history of one thread at its review's current head. Computed on a
/// cache miss (provider calls), so off the IPC thread.
#[tauri::command]
pub async fn thread_history(
    app: AppHandle,
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<ThreadHistory, IpcError> {
    let thread = parse_thread_id(&thread_id)?;
    let (history, changed) = blocking(state, move |core| {
        let review = core.thread_review(thread)?;
        watch_untouched(
            || Ok(core.review(review)?.untouched_threads),
            || core.thread_history(thread),
        )
    })
    .await?;
    if changed {
        events::emit_changed(&app, history.review_id);
    }
    Ok(history)
}

/// The histories of every thread of a review, oldest thread first.
#[tauri::command]
pub async fn review_histories(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<ThreadHistory>, IpcError> {
    let id = parse_review_id(&review_id)?;
    let (histories, changed) = blocking(state, move |core| {
        watch_untouched(
            || Ok(core.review(id)?.untouched_threads),
            || core.review_histories(id),
        )
    })
    .await?;
    if changed {
        events::emit_changed(&app, id);
    }
    Ok(histories)
}

/// Run `call` and tell whether the untouched count (read by `count`) moved
/// meanwhile. Comparing before and after keeps the core's API as it is; a
/// poll racing the call can only cause one extra re-read.
fn watch_untouched<T>(
    count: impl Fn() -> CoreResult<u32>,
    call: impl FnOnce() -> CoreResult<T>,
) -> CoreResult<(T, bool)> {
    let before = count()?;
    let out = call()?;
    let after = count()?;
    Ok((out, before != after))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use argus_core::CoreError;

    use super::*;

    #[test]
    fn reports_whether_the_call_moved_the_count() {
        let untouched = Cell::new(2);
        let (out, changed) = watch_untouched(
            || Ok(untouched.get()),
            || {
                untouched.set(1);
                Ok("history")
            },
        )
        .unwrap();
        assert_eq!((out, changed), ("history", true));

        let (_, changed) = watch_untouched(|| Ok(untouched.get()), || Ok(())).unwrap();
        assert!(!changed);
    }

    #[test]
    fn a_failing_call_or_read_is_the_error() {
        let err = watch_untouched(|| Ok(0), || -> CoreResult<()> { Err(CoreError::NotFound) })
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound));
        let err = watch_untouched(|| Err(CoreError::NotFound), || Ok(())).unwrap_err();
        assert!(matches!(err, CoreError::NotFound));
    }

    #[test]
    fn history_serializes_camel_case_with_a_tagged_status() {
        let review = argus_core::ReviewId::from_key("github|github.com|acme-fixture/widgets|12");
        let history = ThreadHistory {
            thread_id: argus_core::ThreadId::from_key(review, "4001"),
            review_id: review,
            head_sha: "c".repeat(40),
            outdated: false,
            status: argus_core::LineStatus::Located {
                path: "src/a.rs".into(),
                start_line: 3,
                line: 4,
                moved: true,
                renamed: false,
                touched: false,
            },
            touched_by: vec![],
            then_snapshot: None,
            now_snapshot: None,
            diff_lines: vec![],
            timeline: vec![],
        };
        let json = serde_json::to_value(&history).unwrap();
        assert_eq!(json["reviewId"], review.to_string());
        assert_eq!(json["thenSnapshot"], serde_json::Value::Null);
        assert_eq!(json["status"]["kind"], "located");
        assert_eq!(json["status"]["startLine"], 3);
        assert!(json.get("diffLines").is_some() && json.get("touchedBy").is_some());
    }
}
