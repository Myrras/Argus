//! Refresh results to the webview as global Tauri events.
//!
//! After every refresh (the background poll or the refresh button) the shell
//! emits `review:changed` once per review whose stored state changed, then
//! one `reviews:refreshed` carrying the whole report (its time and the
//! per-provider errors). The frontend stores re-read from the backend on
//! these; the payloads are only ids and the report.
//!
//! The AI pass reports every state change of a run (queued, running, done,
//! failed, cancelled) as `ai:state` with the review's [`AiRunState`]. A run
//! that ended also emits `review:changed`: its history pass may have
//! rewritten the review's untouched count.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use argus_core::{AiRunState, AiRunStatus, RefreshReport, ReviewId};

pub const REVIEW_CHANGED: &str = "review:changed";
pub const REVIEWS_REFRESHED: &str = "reviews:refreshed";
pub const AI_STATE: &str = "ai:state";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRefPayload {
    pub review_id: ReviewId,
}

/// The `review:changed` payloads for `report`, in its order, without
/// duplicates (two providers never share a review, but a manual refresh
/// racing a poll could list one twice).
pub fn changed_payloads(report: &RefreshReport) -> Vec<ReviewRefPayload> {
    let mut out: Vec<ReviewRefPayload> = Vec::with_capacity(report.changed.len());
    for &review_id in &report.changed {
        if !out.iter().any(|p| p.review_id == review_id) {
            out.push(ReviewRefPayload { review_id });
        }
    }
    out
}

/// Emit one review's `review:changed`.
pub fn emit_changed(app: &AppHandle, review_id: ReviewId) {
    warn_on_err(app.emit(REVIEW_CHANGED, ReviewRefPayload { review_id }));
}

/// Emit everything one refresh produced.
pub fn emit_report(app: &AppHandle, report: &RefreshReport) {
    for payload in changed_payloads(report) {
        warn_on_err(app.emit(REVIEW_CHANGED, payload));
    }
    warn_on_err(app.emit(REVIEWS_REFRESHED, report));
}

/// Emit one AI run's `ai:state` (on the run's thread; emitting is
/// thread-safe), plus `review:changed` once the run ended.
pub fn emit_ai_state(app: &AppHandle, state: &AiRunState) {
    warn_on_err(app.emit(AI_STATE, state));
    if run_ended(state) {
        emit_changed(app, state.review_id);
    }
}

/// A run finished (or failed, or was stopped) and none waits behind it. A
/// state that only says "queued" keeps the previous run's status, hence
/// the `queued` check.
fn run_ended(state: &AiRunState) -> bool {
    matches!(state.status, AiRunStatus::Done | AiRunStatus::Failed) && !state.queued
}

fn warn_on_err(result: tauri::Result<()>) {
    if let Err(e) = result {
        tracing::warn!("failed to emit event to webview: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argus_core::provider::ProviderId;
    use argus_core::RefreshError;

    #[test]
    fn one_changed_payload_per_review_in_order() {
        let a = ReviewId::from_key("github|github.com|acme-fixture/widgets|1");
        let b = ReviewId::from_key("gitlab|gitlab.example.test|grp/proj|2");
        let report = RefreshReport {
            changed: vec![a, b, a],
            errors: vec![],
            refreshed_at: chrono::Utc::now(),
        };
        let payloads = changed_payloads(&report);
        assert_eq!(
            payloads,
            [
                ReviewRefPayload { review_id: a },
                ReviewRefPayload { review_id: b }
            ]
        );
        let json = serde_json::to_value(&payloads[0]).unwrap();
        assert_eq!(json, serde_json::json!({ "reviewId": a.to_string() }));
    }

    fn ai_state(status: AiRunStatus, queued: bool) -> AiRunState {
        AiRunState {
            review_id: ReviewId::from_key("github|github.com|acme-fixture/widgets|1"),
            enabled: true,
            status,
            queued,
            round: Some(2),
            adapter: Some("claude".into()),
            error: None,
            started_at: None,
            finished_at: None,
            last_done_round: Some(1),
            dropped_findings: 0,
            dropped_verdicts: 0,
        }
    }

    #[test]
    fn only_an_ended_run_changes_the_review() {
        assert!(run_ended(&ai_state(AiRunStatus::Done, false)));
        assert!(run_ended(&ai_state(AiRunStatus::Failed, false)));
        assert!(!run_ended(&ai_state(AiRunStatus::Running, false)));
        assert!(!run_ended(&ai_state(AiRunStatus::NotStarted, false)));
        // Queued behind an earlier run: the status is the earlier one's.
        assert!(!run_ended(&ai_state(AiRunStatus::Done, true)));
    }

    #[test]
    fn ai_state_payload_is_camel_case() {
        let json = serde_json::to_value(ai_state(AiRunStatus::Running, false)).unwrap();
        assert_eq!(json["status"], "running");
        assert_eq!(json["lastDoneRound"], 1);
        assert_eq!(json["droppedFindings"], 0);
        assert_eq!(json["startedAt"], serde_json::Value::Null);
    }

    #[test]
    fn refreshed_payload_is_the_camel_case_report() {
        let report = RefreshReport {
            changed: vec![],
            errors: vec![RefreshError {
                provider: ProviderId::Gitlab,
                review: None,
                message: "GitLab is not set up: add its URL and a token in Settings.".into(),
                unavailable: true,
            }],
            refreshed_at: chrono::Utc::now(),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("refreshedAt").is_some());
        assert_eq!(json["errors"][0]["provider"], "gitlab");
        assert_eq!(json["errors"][0]["review"], serde_json::Value::Null);
        assert_eq!(json["errors"][0]["unavailable"], true);
    }
}
