//! AI pass commands (M5): a review's run state, findings, checklist and
//! verdicts, starting and stopping a run, the user's marks, posting
//! findings, and the review skills a repository can follow.
//!
//! A run happens on the core's own threads and reports through `ai:state`
//! (see [`crate::events`]); these commands only queue, cancel and read.
//! `post_findings` is the only one that writes to a platform: call it from
//! an explicit user action only.

use tauri::{AppHandle, State};

use argus_core::{
    AiRunState, ChecklistId, ChecklistItem, Finding, FindingId, FindingPost, FindingPostOutcome,
    FindingPostResult, PostErrorKind, ReviewSkillList, Verdict, VerdictLabel,
};

use super::{blocking, blocking_post, parse_id, parse_review_id, parse_thread_id};
use crate::error::IpcError;
use crate::events;
use crate::state::AppState;

/// The review's latest run (`notStarted` when none, or the AI pass is off).
#[tauri::command]
pub async fn ai_state(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<AiRunState, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.ai_state(id)).await
}

/// The review skills the Settings picker offers for `repo` (`host/repo`):
/// the user's and installed plugins' whose name says "review", and the
/// repository's own from its clone. Read-only (files and `git` object
/// reads).
#[tauri::command]
pub async fn list_review_skills(
    state: State<'_, AppState>,
    repo: Option<String>,
) -> Result<ReviewSkillList, IpcError> {
    blocking(state, move |core| core.list_review_skills(repo.as_deref())).await
}

/// The review's findings, every round: high severity first.
#[tauri::command]
pub async fn list_findings(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<Finding>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.findings(id)).await
}

/// The review's "look here yourself" items, newest round first.
#[tauri::command]
pub async fn list_checklist(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<ChecklistItem>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.checklist(id)).await
}

/// The current verdict per thread of the review.
#[tauri::command]
pub async fn list_verdicts(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<Verdict>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.verdicts(id)).await
}

/// The review's findings set aside as already raised in the review, with
/// what each repeats.
#[tauri::command]
pub async fn list_skipped_findings(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<Finding>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.skipped_findings(id)).await
}

/// The review's checklist items set aside as already raised in the review,
/// with what each repeats.
#[tauri::command]
pub async fn list_skipped_checklist(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<Vec<ChecklistItem>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.skipped_checklist(id)).await
}

/// Queue a run of the review now; `invalidInput` when the AI pass is off.
#[tauri::command]
pub async fn run_ai(state: State<'_, AppState>, review_id: String) -> Result<AiRunState, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.run_ai(id)).await
}

/// Drop the review's queued run or stop its running one.
#[tauri::command]
pub async fn cancel_ai(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<AiRunState, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.cancel_ai(id)).await
}

/// Dismiss a finding, or bring it back.
#[tauri::command]
pub async fn dismiss_finding(
    state: State<'_, AppState>,
    finding_id: String,
    dismissed: bool,
) -> Result<Finding, IpcError> {
    let id: FindingId = parse_id(&finding_id, "findingId")?;
    blocking(state, move |core| core.dismiss_finding(id, dismissed)).await
}

/// Show a finding set aside as already raised anyway: it becomes a normal
/// finding. Local only.
#[tauri::command]
pub async fn show_finding_anyway(
    state: State<'_, AppState>,
    finding_id: String,
) -> Result<Finding, IpcError> {
    let id: FindingId = parse_id(&finding_id, "findingId")?;
    blocking(state, move |core| core.show_finding_anyway(id)).await
}

/// Show a checklist item set aside as already raised anyway: it becomes a
/// normal item. Local only.
#[tauri::command]
pub async fn show_checklist_item_anyway(
    state: State<'_, AppState>,
    checklist_id: String,
) -> Result<ChecklistItem, IpcError> {
    let id: ChecklistId = parse_id(&checklist_id, "checklistId")?;
    blocking(state, move |core| core.show_checklist_item_anyway(id)).await
}

#[tauri::command]
pub async fn set_checklist_checked(
    state: State<'_, AppState>,
    checklist_id: String,
    checked: bool,
) -> Result<ChecklistItem, IpcError> {
    let id: ChecklistId = parse_id(&checklist_id, "checklistId")?;
    blocking(state, move |core| core.set_checklist_checked(id, checked)).await
}

/// The user's own label on a thread (`null` clears it). Local only.
#[tauri::command]
pub async fn set_verdict_override(
    state: State<'_, AppState>,
    thread_id: String,
    label: Option<VerdictLabel>,
) -> Result<Verdict, IpcError> {
    let thread = parse_thread_id(&thread_id)?;
    blocking(state, move |core| core.set_verdict_override(thread, label)).await
}

/// Post findings as line comments, one by one; one result per finding in
/// the same order. `Err` only when nothing could be tried.
#[tauri::command]
pub async fn post_findings(
    app: AppHandle,
    state: State<'_, AppState>,
    review_id: String,
    posts: Vec<FindingPost>,
) -> Result<Vec<FindingPostResult>, IpcError> {
    let id = parse_review_id(&review_id)?;
    let results = blocking_post(state, move |core| core.post_findings(id, posts)).await;
    super::post::emit_if_head_moved(&app, id, &results);
    let results = results?;
    if changes_review(&results) {
        events::emit_changed(&app, id);
    }
    Ok(results)
}

/// Whether the core stored something new for the review: a finding was
/// posted, or one was refused for a push (it stored the new head).
fn changes_review(results: &[FindingPostResult]) -> bool {
    results.iter().any(|r| match &r.outcome {
        FindingPostOutcome::Posted { .. } => true,
        FindingPostOutcome::Failed(e) => e.kind == PostErrorKind::HeadMoved,
    })
}

#[cfg(test)]
mod tests {
    use argus_core::PostError;

    use super::*;

    fn failed(id: FindingId) -> FindingPostResult {
        FindingPostResult {
            finding_id: id,
            outcome: FindingPostOutcome::Failed(PostError::new(
                PostErrorKind::LineNotInDiff,
                "That line is not in the diff any more.",
            )),
        }
    }

    fn posted(id: FindingId) -> FindingPostResult {
        FindingPostResult {
            finding_id: id,
            outcome: FindingPostOutcome::Posted {
                posted_url: "https://github.com/acme-fixture/widgets/pull/12#r1".into(),
                posted_at: chrono::Utc::now(),
            },
        }
    }

    #[test]
    fn a_posted_finding_or_a_moved_head_changes_the_review() {
        let (a, b) = (FindingId::new(), FindingId::new());
        assert!(!changes_review(&[]));
        assert!(!changes_review(&[failed(a), failed(b)]));
        assert!(changes_review(&[failed(a), posted(b)]));
        // GitLab refused one for a push: the core stored the new head.
        let moved = FindingPostResult {
            finding_id: b,
            outcome: FindingPostOutcome::Failed(PostError::new(PostErrorKind::HeadMoved, "moved")),
        };
        assert!(changes_review(&[failed(a), moved]));
    }

    #[test]
    fn post_results_flatten_the_outcome() {
        let id = FindingId::new();
        let json = serde_json::to_value(posted(id)).unwrap();
        assert_eq!(json["findingId"], id.to_string());
        assert_eq!(json["status"], "posted");
        assert!(json.get("postedUrl").is_some() && json.get("postedAt").is_some());
        let json = serde_json::to_value(failed(id)).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["kind"], "lineNotInDiff");
        assert_eq!(json["message"], "That line is not in the diff any more.");
    }

    #[test]
    fn inputs_deserialize_from_camel_case() {
        let id = FindingId::new();
        let post: FindingPost =
            serde_json::from_value(serde_json::json!({ "findingId": id.to_string() })).unwrap();
        assert_eq!(post.finding_id, id);
        assert_eq!(post.body, None);
        let label: Option<VerdictLabel> = serde_json::from_value(serde_json::json!(null)).unwrap();
        assert_eq!(label, None);
        let label: Option<VerdictLabel> =
            serde_json::from_value(serde_json::json!("partly")).unwrap();
        assert_eq!(label, Some(VerdictLabel::Partly));
    }

    #[test]
    fn ai_ids_are_parsed_without_echoing_input() {
        let id = ChecklistId::new();
        let parsed: ChecklistId = parse_id(&id.to_string(), "checklistId").unwrap();
        assert_eq!(parsed, id);
        let err = parse_id::<FindingId>("../x", "findingId").unwrap_err();
        assert_eq!(err.kind, "invalidInput");
        assert_eq!(err.message, "findingId is not a valid id");
    }
}
