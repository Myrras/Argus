//! The AI pass' public shapes. Serialised camelCase: mirror them in
//! `src/ipc/types.ts`.
//!
//! ```ts
//! type AiRunStatus = "notStarted" | "running" | "done" | "failed";
//! interface AiRunState {
//!   reviewId: string; enabled: boolean; status: AiRunStatus;
//!   queued: boolean; round: number | null; adapter: string | null;
//!   error: string | null; startedAt: string | null; finishedAt: string | null;
//!   lastDoneRound: number | null; droppedFindings: number; droppedVerdicts: number;
//! }
//! type Severity = "high" | "medium" | "low";
//! interface Finding {
//!   id: string; reviewId: string; round: number; path: string; line: number;
//!   side: "old" | "new"; severity: Severity; title: string; body: string;
//!   dismissed: boolean; postedAt: string | null; postedUrl: string | null;
//! }
//! interface ChecklistItem { id: string; reviewId: string; round: number; text: string; checked: boolean }
//! interface FindingPost { findingId: string; body?: string }  // body: the user's edit
//! type FindingPostResult = { findingId: string } & (
//!   | { status: "posted"; postedUrl: string; postedAt: string }
//!   | { status: "failed"; kind: PostErrorKind; message: string });
//! type VerdictLabel = "addressed" | "partly" | "untouched";
//! interface Verdict {
//!   threadId: string; reviewId: string; round: number;
//!   aiLabel: VerdictLabel | null; aiReason: string | null;
//!   userOverride: VerdictLabel | null; label: VerdictLabel | null;
//! }
//! ```

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ids::{ReviewId, ThreadId};
use crate::post::PostError;
use crate::provider::Side;

/// A plain UUID-string id, like the ones in [`crate::ids`] (whose macro is
/// private to that module).
macro_rules! ai_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::from_str(s)?))
            }
        }
    };
}

ai_id!(
    /// Identifies one finding.
    FindingId
);
ai_id!(
    /// Identifies one checklist item.
    ChecklistId
);

/// Where a review's AI run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiRunStatus {
    NotStarted,
    Running,
    Done,
    Failed,
}

impl AiRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AiRunStatus::NotStarted => "notStarted",
            AiRunStatus::Running => "running",
            AiRunStatus::Done => "done",
            AiRunStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "notStarted" => Some(AiRunStatus::NotStarted),
            "running" => Some(AiRunStatus::Running),
            "done" => Some(AiRunStatus::Done),
            "failed" => Some(AiRunStatus::Failed),
            _ => None,
        }
    }
}

/// What `ai_state` returns and the `ai:state` event carries: the latest run
/// of one review (persisted) plus whether another one waits for a slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRunState {
    pub review_id: ReviewId,
    /// The AI pass is turned on in settings. When off, `status` is
    /// `notStarted` and nothing runs.
    pub enabled: bool,
    pub status: AiRunStatus,
    /// A run is waiting for a free slot (at most two run at once, one per
    /// review). Runtime only: not persisted.
    pub queued: bool,
    /// The round the latest run was for.
    pub round: Option<u32>,
    /// The adapter id the latest run used (`claude`, `auggie`).
    pub adapter: Option<String>,
    /// Why the latest run failed (app-owned text; `interrupted` after a
    /// restart, `cancelled` when stopped).
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// The round of the last run that succeeded.
    pub last_done_round: Option<u32>,
    /// Findings the latest successful run dropped: their path is not in the
    /// diff it was given.
    pub dropped_findings: u32,
    /// Verdicts the latest successful run dropped: unknown thread ids.
    pub dropped_verdicts: u32,
}

impl AiRunState {
    pub(crate) fn not_started(review_id: ReviewId, enabled: bool) -> Self {
        Self {
            review_id,
            enabled,
            status: AiRunStatus::NotStarted,
            queued: false,
            round: None,
            adapter: None,
            error: None,
            started_at: None,
            finished_at: None,
            last_done_round: None,
            dropped_findings: 0,
            dropped_verdicts: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "high" => Some(Severity::High),
            "medium" => Some(Severity::Medium),
            "low" => Some(Severity::Low),
            _ => None,
        }
    }
}

/// One concrete problem the AI found, anchored to a line of the diff.
/// Findings of earlier rounds stay (with their round) until dismissed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: FindingId,
    pub review_id: ReviewId,
    pub round: u32,
    pub path: String,
    pub line: u32,
    pub side: Side,
    pub severity: Severity,
    pub title: String,
    pub body: String,
    pub dismissed: bool,
    /// Set once the user posted it as a comment (`Core::post_findings`).
    pub posted_at: Option<DateTime<Utc>>,
    pub posted_url: Option<String>,
}

/// One finding the user posts as a line comment, with their edit of its
/// text (`None`: the finding's own title and body).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingPost {
    pub finding_id: FindingId,
    #[serde(default)]
    pub body: Option<String>,
}

/// The outcome of one finding of a batch, in the batch's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingPostResult {
    pub finding_id: FindingId,
    #[serde(flatten)]
    pub outcome: FindingPostOutcome,
}

/// `{ "status": "posted", postedUrl, postedAt }` or `{ "status": "failed",
/// kind, message }` (a [`PostError`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FindingPostOutcome {
    Posted {
        #[serde(rename = "postedUrl")]
        posted_url: String,
        #[serde(rename = "postedAt")]
        posted_at: DateTime<Utc>,
    },
    Failed(PostError),
}

/// "Look here yourself": something the AI could not verify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub id: ChecklistId,
    pub review_id: ReviewId,
    pub round: u32,
    pub text: String,
    pub checked: bool,
}

/// How far a later round addressed one of the user's threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerdictLabel {
    Addressed,
    Partly,
    Untouched,
}

impl VerdictLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictLabel::Addressed => "addressed",
            VerdictLabel::Partly => "partly",
            VerdictLabel::Untouched => "untouched",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "addressed" => Some(VerdictLabel::Addressed),
            "partly" => Some(VerdictLabel::Partly),
            "untouched" => Some(VerdictLabel::Untouched),
            _ => None,
        }
    }
}

/// The current verdict on one thread: the AI's hint and the user's own
/// call, kept apart. Nothing here is ever written to the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub thread_id: ThreadId,
    pub review_id: ReviewId,
    pub round: u32,
    pub ai_label: Option<VerdictLabel>,
    pub ai_reason: Option<String>,
    pub user_override: Option<VerdictLabel>,
    /// What to show: the override when set, else the AI's label.
    pub label: Option<VerdictLabel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes_are_camel_case() {
        let s = AiRunState::not_started(ReviewId::from_key("x"), true);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["status"], "notStarted");
        assert_eq!(v["queued"], false);
        assert!(v.get("lastDoneRound").is_some());
        assert!(v.get("droppedFindings").is_some());
        assert_eq!(serde_json::to_value(Severity::High).unwrap(), "high");
        assert_eq!(
            serde_json::to_value(VerdictLabel::Partly).unwrap(),
            "partly"
        );
        for st in [
            AiRunStatus::NotStarted,
            AiRunStatus::Running,
            AiRunStatus::Done,
            AiRunStatus::Failed,
        ] {
            assert_eq!(AiRunStatus::parse(st.as_str()), Some(st));
            assert_eq!(serde_json::to_value(st).unwrap(), st.as_str());
        }
        let id = FindingId::new();
        assert_eq!(id.to_string().parse::<FindingId>().unwrap(), id);
        assert_eq!(serde_json::to_value(id).unwrap(), id.to_string());
    }
}
