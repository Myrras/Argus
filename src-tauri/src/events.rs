//! Refresh results to the webview as global Tauri events.
//!
//! After every refresh (the background poll or the refresh button) the shell
//! emits `review:changed` once per review whose stored state changed, then
//! one `reviews:refreshed` carrying the whole report (its time and the
//! per-provider errors). The frontend stores re-read from the backend on
//! these; the payloads are only ids and the report.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use argus_core::{RefreshReport, ReviewId};

pub const REVIEW_CHANGED: &str = "review:changed";
pub const REVIEWS_REFRESHED: &str = "reviews:refreshed";

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
