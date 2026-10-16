//! Round tracking: where each review stands for the user, as pure functions
//! over (what the store remembers, what a fresh fetch says).
//!
//! The watermark is `last_reviewed_sha` + `last_reviewed_at`. It only moves
//! when the user says so ([`mark_reviewed`]; posting and approving from
//! Argus join in M4) — a review submitted on the platform itself does not
//! move it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::provider::{ReviewDetail, ReviewState, Thread};
use crate::store::ReviewRecord;

/// Why a review needs the user again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoundReason {
    /// The head moved since the watermark.
    NewCommits,
    /// Someone else added a note to one of the user's threads after the
    /// watermark (the plan's "author replied"; any other participant counts).
    AuthorReplied,
    /// The user's review was requested again after the watermark.
    ReRequested,
}

impl RoundReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RoundReason::NewCommits => "newCommits",
            RoundReason::AuthorReplied => "authorReplied",
            RoundReason::ReRequested => "reRequested",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "newCommits" => Some(RoundReason::NewCommits),
            "authorReplied" => Some(RoundReason::AuthorReplied),
            "reRequested" => Some(RoundReason::ReRequested),
            _ => None,
        }
    }
}

/// Which list a review belongs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewStatus {
    /// First round: the user never marked it reviewed.
    NeedsReview,
    /// Reviewed before, and something happened since (see the reasons).
    NeedsReReview,
    /// Reviewed, nothing new since.
    WaitingOnAuthor,
    /// Merged or closed while the user had not approved it.
    ClosedWithoutApproval,
    /// The user approved it; hidden from the active lists.
    Approved,
}

impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::NeedsReview => "needsReview",
            ReviewStatus::NeedsReReview => "needsReReview",
            ReviewStatus::WaitingOnAuthor => "waitingOnAuthor",
            ReviewStatus::ClosedWithoutApproval => "closedWithoutApproval",
            ReviewStatus::Approved => "approved",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "needsReview" => Some(ReviewStatus::NeedsReview),
            "needsReReview" => Some(ReviewStatus::NeedsReReview),
            "waitingOnAuthor" => Some(ReviewStatus::WaitingOnAuthor),
            "closedWithoutApproval" => Some(ReviewStatus::ClosedWithoutApproval),
            "approved" => Some(ReviewStatus::Approved),
            _ => None,
        }
    }
}

/// The outcome of classifying one review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub status: ReviewStatus,
    /// Non-empty exactly when `status` is [`ReviewStatus::NeedsReReview`];
    /// sorted, no duplicates.
    pub reasons: Vec<RoundReason>,
    /// The user's open threads nobody touched since (see [`untouched`]).
    pub untouched_threads: u32,
}

/// The inputs [`classify`] reads.
#[derive(Debug, Clone, Copy)]
pub struct RoundInput<'a> {
    pub state: ReviewState,
    pub head_sha: &'a str,
    pub last_reviewed_sha: Option<&'a str>,
    pub last_reviewed_at: Option<DateTime<Utc>>,
    pub approved_at: Option<DateTime<Utc>>,
    pub rerequested_at: Option<DateTime<Utc>>,
    pub threads: &'a [Thread],
    /// The current user's username on the review's platform.
    pub me: &'a str,
}

fn same_user(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn is_mine(t: &Thread, me: &str) -> bool {
    same_user(&t.author, me)
}

/// Classify one review. Order matters: an approval hides the item whatever
/// else happened, a closed item is closed, and only then do rounds apply.
pub fn classify(input: &RoundInput<'_>) -> Round {
    let untouched_threads = untouched(input);
    let round = |status, reasons| Round {
        status,
        reasons,
        untouched_threads,
    };
    if input.approved_at.is_some() {
        return round(ReviewStatus::Approved, Vec::new());
    }
    if input.state != ReviewState::Open {
        return round(ReviewStatus::ClosedWithoutApproval, Vec::new());
    }
    let (Some(sha), Some(at)) = (input.last_reviewed_sha, input.last_reviewed_at) else {
        return round(ReviewStatus::NeedsReview, Vec::new());
    };
    let mut reasons = Vec::new();
    if input.head_sha != sha {
        reasons.push(RoundReason::NewCommits);
    }
    let replied = input
        .threads
        .iter()
        .filter(|t| is_mine(t, input.me))
        .flat_map(|t| &t.notes)
        .any(|n| !n.system && !same_user(&n.author, input.me) && n.created_at > at);
    if replied {
        reasons.push(RoundReason::AuthorReplied);
    }
    if input.rerequested_at.is_some_and(|r| r > at) {
        reasons.push(RoundReason::ReRequested);
    }
    if reasons.is_empty() {
        round(ReviewStatus::WaitingOnAuthor, reasons)
    } else {
        round(ReviewStatus::NeedsReReview, reasons)
    }
}

/// The user's unresolved threads that look untouched: no note from someone
/// else since the later of the watermark and the user's own last note in
/// the thread, and the head unchanged since the watermark (or, before any
/// watermark, since the commit the thread was left on).
///
/// An M2 approximation: any new commit counts as touching every thread,
/// because telling whether it touched the commented lines needs the line
/// mapping that lands in M3.
pub fn untouched(input: &RoundInput<'_>) -> u32 {
    let me = input.me;
    let count = input
        .threads
        .iter()
        .filter(|t| is_mine(t, me) && !t.resolved)
        .filter(|t| {
            let baseline = input
                .last_reviewed_sha
                .or(t.anchor.as_ref().map(|a| a.head_sha.as_str()))
                .unwrap_or(input.head_sha);
            if baseline != input.head_sha {
                return false;
            }
            let my_last = t
                .notes
                .iter()
                .filter(|n| same_user(&n.author, me))
                .map(|n| n.created_at)
                .max();
            let since = match (input.last_reviewed_at, my_last) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            !t.notes.iter().any(|n| {
                !n.system && !same_user(&n.author, me) && since.is_none_or(|s| n.created_at > s)
            })
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Fold a fresh fetch into the stored record: the bookkeeping fields
/// (`approved_at`, `rerequested_at`, `round`) move as the platform changed,
/// then the review is classified.
///
/// - `approved_at` is set (once) while the user's approval stands, and
///   cleared when the platform no longer shows it.
/// - A re-request is a transition: requested now, not requested at the last
///   fetch. A request that never went away (the user reviewed in Argus but
///   not on the platform) is not one.
/// - `round` starts at 1 and goes up each time the review enters
///   [`ReviewStatus::NeedsReReview`].
pub fn advance(
    prev: Option<&ReviewRecord>,
    detail: &ReviewDetail,
    threads: &[Thread],
    me: &str,
    listed: bool,
    now: DateTime<Utc>,
) -> ReviewRecord {
    let approved_at = if detail.approved_by_me {
        prev.and_then(|p| p.approved_at).or(Some(now))
    } else {
        None
    };
    let rerequested_at = match prev {
        Some(p) if detail.requested_from_me && !p.requested => Some(now),
        Some(p) => p.rerequested_at,
        None => None,
    };
    let last_reviewed_sha = prev.and_then(|p| p.last_reviewed_sha.clone());
    let last_reviewed_at = prev.and_then(|p| p.last_reviewed_at);
    let round = classify(&RoundInput {
        state: detail.state,
        head_sha: &detail.head_sha,
        last_reviewed_sha: last_reviewed_sha.as_deref(),
        last_reviewed_at,
        approved_at,
        rerequested_at,
        threads,
        me,
    });
    let prev_round = prev.map_or(1, |p| p.round);
    let entered = round.status == ReviewStatus::NeedsReReview
        && prev.is_none_or(|p| p.status != ReviewStatus::NeedsReReview);
    ReviewRecord {
        id: detail.review.id(),
        review: detail.review.clone(),
        state: detail.state,
        draft: detail.draft,
        head_sha: detail.head_sha.clone(),
        base_sha: detail.base_sha.clone(),
        last_reviewed_sha,
        last_reviewed_at,
        approved_at,
        round: if entered { prev_round + 1 } else { prev_round },
        round_reasons: round.reasons,
        status: round.status,
        untouched_threads: round.untouched_threads,
        requested: detail.requested_from_me,
        rerequested_at,
        listed,
        viewer: me.to_string(),
        fingerprint: String::new(),
        fetched_at: now,
    }
}

/// The user finished this round: the watermark moves to the current head
/// and `now`, and the review is classified again (normally to
/// [`ReviewStatus::WaitingOnAuthor`]).
pub fn mark_reviewed(
    record: &ReviewRecord,
    threads: &[Thread],
    now: DateTime<Utc>,
) -> ReviewRecord {
    let mut next = record.clone();
    next.last_reviewed_sha = Some(record.head_sha.clone());
    next.last_reviewed_at = Some(now);
    let round = classify(&RoundInput {
        state: next.state,
        head_sha: &next.head_sha,
        last_reviewed_sha: next.last_reviewed_sha.as_deref(),
        last_reviewed_at: next.last_reviewed_at,
        approved_at: next.approved_at,
        rerequested_at: next.rerequested_at,
        threads,
        me: &next.viewer,
    });
    next.status = round.status;
    next.round_reasons = round.reasons;
    next.untouched_threads = round.untouched_threads;
    next
}

#[cfg(test)]
mod tests;
