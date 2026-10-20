//! Round tracking: where each review stands for the user, as pure functions
//! over (what the store remembers, what a fresh fetch says).
//!
//! The platform's reviewer state is the source of truth for "waiting on
//! me" ([`MyReviewState`]): while it says the user's review is owed
//! (GitLab "Awaiting review", GitHub a pending review request), the review
//! needs the user whatever Argus remembers locally: "Needs review" until
//! the user submitted a review on it, "Needs re-review" (re-requested)
//! after. Once the user requested changes or commented, it waits on the
//! author until new commits or a reply after that review; an approval
//! takes it off the active lists until the platform drops it.
//!
//! The watermark is the later of Argus' own (`last_reviewed_sha` +
//! `last_reviewed_at`, moved by Submit review, posting and replying from
//! Argus) and the user's latest review submitted on the platform itself
//! (`my_reviewed_sha` + `my_reviewed_at`): a review given on GitHub or
//! GitLab counts too.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::history::LineTouches;
use crate::provider::model::same_user;
use crate::provider::{MyReviewState, ReviewDetail, ReviewState, Thread};
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
    /// When the view the user's newest notes were written over was fetched,
    /// if known: another's note after it was not on the user's screen, so
    /// a later note of the user's does not answer it. Known right after a
    /// post from Argus (the stored fetch the post started from); `None`
    /// otherwise, where a note (posted on the platform, with everything
    /// before it on screen) answers what came before it.
    pub seen_at: Option<DateTime<Utc>>,
    /// The user's own review state on the platform.
    pub my_state: MyReviewState,
    /// The user's latest review submitted on the platform: its head and
    /// time. The later of it and Argus' watermark is the watermark.
    pub platform_reviewed_sha: Option<&'a str>,
    pub platform_reviewed_at: Option<DateTime<Utc>>,
}

impl<'a> RoundInput<'a> {
    /// The input for a stored review over `threads` (`seen_at` unknown).
    pub(crate) fn of(r: &'a ReviewRecord, threads: &'a [Thread]) -> Self {
        Self {
            state: r.state,
            head_sha: &r.head_sha,
            last_reviewed_sha: r.last_reviewed_sha.as_deref(),
            last_reviewed_at: r.last_reviewed_at,
            approved_at: r.approved_at,
            rerequested_at: r.rerequested_at,
            threads,
            me: &r.viewer,
            seen_at: None,
            my_state: r.my_review_state,
            platform_reviewed_sha: r.my_reviewed_sha.as_deref(),
            platform_reviewed_at: r.my_reviewed_at,
        }
    }

    /// This input with the watermark taken as the later of Argus' own and
    /// the user's latest review on the platform. Idempotent.
    fn effective(&self) -> Self {
        let platform = self.platform_reviewed_at.zip(self.platform_reviewed_sha);
        match (self.last_reviewed_at, platform) {
            (Some(local), Some((at, _))) if local >= at => *self,
            (_, Some((at, sha))) if !sha.is_empty() => Self {
                last_reviewed_sha: Some(sha),
                last_reviewed_at: Some(at),
                ..*self
            },
            _ => *self,
        }
    }

    /// Whether the platform shows the user's review was asked for again
    /// after they gave one: they submitted a review on the platform before
    /// (GitHub drops a reviewer from the requested list on any review and
    /// GitLab keeps a given state, so owing one again means a re-request),
    /// or the newest request came after Argus' watermark (posting or a
    /// local mark from an earlier round).
    fn re_requested(&self) -> bool {
        if self.platform_reviewed_at.is_some() {
            return true;
        }
        matches!(
            (self.rerequested_at, self.last_reviewed_at),
            (Some(r), Some(at)) if r > at
        )
    }
}

/// Where the notes of a thread start being new to the user: the later of
/// `watermark` and the user's own last note in it (a reply the user
/// answered was seen), that note capped at `seen_at` (what was on screen
/// when it was written). A system note (resolving, say) answers nothing.
fn new_since(
    t: &Thread,
    me: &str,
    watermark: Option<DateTime<Utc>>,
    seen_at: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let my_last = t
        .notes
        .iter()
        .filter(|n| !n.system && same_user(&n.author, me))
        .map(|n| n.created_at)
        .max()
        .map(|m| seen_at.map_or(m, |seen| m.min(seen)));
    match (watermark, my_last) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// A note from someone else after `since` (all of them without one).
fn replied_since(t: &Thread, me: &str, since: Option<DateTime<Utc>>) -> bool {
    t.notes
        .iter()
        .any(|n| !n.system && !same_user(&n.author, me) && since.is_none_or(|s| n.created_at > s))
}

/// Classify one review. Order matters: an approval hides the item whatever
/// else happened, a closed item is closed, and only then do rounds apply.
pub fn classify(input: &RoundInput<'_>) -> Round {
    classify_with(input, None)
}

/// [`classify`] with the cached line walks of the review's threads at its
/// head, for the untouched count (see [`untouched_with`]).
pub fn classify_with(input: &RoundInput<'_>, touches: Option<&LineTouches>) -> Round {
    let raw = input;
    let input = &input.effective();
    let untouched_threads = untouched_with(input, touches);
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
    let owed = input.my_state.owes_review();
    // Owed and never given (nothing on the platform, and no request since
    // Argus' watermark): the first round, whatever was posted meanwhile.
    if owed && !raw.re_requested() {
        return round(ReviewStatus::NeedsReview, Vec::new());
    }
    let (Some(sha), Some(at)) = (input.last_reviewed_sha, input.last_reviewed_at) else {
        // A verdict the platform shows without a time to compare against
        // (a GitLab "reviewed" with nothing dated): the author's move.
        if input.my_state.submitted_without_approval() {
            return round(ReviewStatus::WaitingOnAuthor, Vec::new());
        }
        return round(ReviewStatus::NeedsReview, Vec::new());
    };
    let mut reasons = Vec::new();
    if input.head_sha != sha {
        reasons.push(RoundReason::NewCommits);
    }
    if author_replied(input) {
        reasons.push(RoundReason::AuthorReplied);
    }
    if owed || input.rerequested_at.is_some_and(|r| r > at) {
        reasons.push(RoundReason::ReRequested);
    }
    if reasons.is_empty() {
        round(ReviewStatus::WaitingOnAuthor, reasons)
    } else {
        round(ReviewStatus::NeedsReReview, reasons)
    }
}

/// Someone else's note in one of the user's threads (started or replied
/// in) after the watermark and after the user's own last note there.
/// `false` before any watermark.
fn author_replied(input: &RoundInput<'_>) -> bool {
    let input = &input.effective();
    let Some(at) = input.last_reviewed_at else {
        return false;
    };
    input
        .threads
        .iter()
        .filter(|t| t.is_mine(input.me))
        .any(|t| replied_since(t, input.me, new_since(t, input.me, Some(at), input.seen_at)))
}

/// [`untouched_with`] as it agrees with the stored `reasons`. A
/// [`RoundReason::AuthorReplied`] that the notes alone (`seen_at` unknown)
/// no longer show was kept by [`keep_unanswered`]: the user's later notes
/// were written over the view at the watermark (a post sets the watermark
/// to that view), so they are read that way here too, and the thread with
/// the reply is not counted untouched.
pub(crate) fn untouched_agreeing(
    input: &RoundInput<'_>,
    reasons: &[RoundReason],
    touches: Option<&LineTouches>,
) -> u32 {
    if input.seen_at.is_none()
        && reasons.contains(&RoundReason::AuthorReplied)
        && !author_replied(input)
    {
        let as_seen = RoundInput {
            seen_at: input.last_reviewed_at,
            ..*input
        };
        return untouched_with(&as_seen, touches);
    }
    untouched_with(input, touches)
}

/// The user's unresolved threads that look untouched, without line walks:
/// [`untouched_with`] with none, so any new commit counts as touching every
/// thread.
pub fn untouched(input: &RoundInput<'_>) -> u32 {
    untouched_with(input, None)
}

/// The user's unresolved threads ([`Thread::is_mine`]: started or replied
/// in) that look untouched: no note from someone else since the later of the watermark and the user's own last note in
/// the thread, and no commit since the watermark (or, before any
/// watermark, since the commit the thread was left on) touched its lines.
///
/// Whether a commit touched the lines comes from the thread's cached line
/// walk at the current head (`touches`, by platform thread id). Without one
/// that can answer (none cached for this head, unlocated, rebased) the M2
/// approximation stands: any new commit counts as touching the thread.
pub fn untouched_with(input: &RoundInput<'_>, touches: Option<&LineTouches>) -> u32 {
    let input = &input.effective();
    let me = input.me;
    let count = input
        .threads
        .iter()
        .filter(|t| t.is_mine(me) && !t.resolved)
        .filter(|t| {
            let walked = touches
                .and_then(|m| m.get(&t.id))
                .and_then(|lt| lt.touched_since(input.last_reviewed_sha));
            let code_touched = walked.unwrap_or_else(|| {
                let baseline = input
                    .last_reviewed_sha
                    .or(t.anchor.as_ref().map(|a| a.head_sha.as_str()))
                    .unwrap_or(input.head_sha);
                baseline != input.head_sha
            });
            if code_touched {
                return false;
            }
            !replied_since(
                t,
                me,
                new_since(t, me, input.last_reviewed_at, input.seen_at),
            )
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Fold a fresh fetch into the stored record: the bookkeeping fields
/// (`approved_at`, `rerequested_at`, `round`) move as the platform changed,
/// then the review is classified.
///
/// - `approved_at` is set (once) while the user's approval stands, and
///   cleared when the platform no longer shows it — unless it was stored
///   after `now` (the fetch's start): an approval from Argus the fetch could
///   not see yet.
/// - `rerequested_at` is the newest request the platform dates (GitLab's
///   "requested review from @me" note), else a transition seen between two
///   fetches (requested now, not at the last fetch); it never moves back.
///   Being owed a review counts by itself (see [`classify`]): this only
///   tells a re-request from the first request when the platform keeps no
///   earlier review of the user's.
/// - The user's platform review state and latest platform review are
///   stored as fetched.
/// - `round` starts at 1 and goes up each time the review enters
///   [`ReviewStatus::NeedsReReview`].
///
/// `prev`'s stored threads are taken as none: every note of the user's is
/// new to it (see [`advance_with`]).
pub fn advance(
    prev: Option<&ReviewRecord>,
    detail: &ReviewDetail,
    threads: &[Thread],
    me: &str,
    listed: bool,
    now: DateTime<Utc>,
) -> ReviewRecord {
    advance_with(
        prev.map(|p| (p, &[][..])),
        detail,
        threads,
        me,
        listed,
        now,
        None,
    )
}

/// [`advance`] from the stored record with the threads stored with it
/// (what tells a note of the user's as new, see [`keep_unanswered`]), and
/// the cached line walks at the fetched head (see [`untouched_with`]).
pub fn advance_with(
    prev: Option<(&ReviewRecord, &[Thread])>,
    detail: &ReviewDetail,
    threads: &[Thread],
    me: &str,
    listed: bool,
    now: DateTime<Utc>,
    touches: Option<&LineTouches>,
) -> ReviewRecord {
    let stored = prev.map_or(&[][..], |(_, t)| t);
    let prev = prev.map(|(p, _)| p);
    // `now` is when the fetch started: an approval stored after that (from
    // Argus, while this fetch was on its way) is newer than what the fetch
    // saw, so it stands until a fetch that started later disagrees.
    let approved_at = if detail.approved_by_me {
        prev.and_then(|p| p.approved_at).or(Some(now))
    } else {
        prev.and_then(|p| p.approved_at).filter(|a| *a > now)
    };
    let edge = match prev {
        Some(p) if detail.requested_from_me && !p.requested => Some(now),
        Some(p) => p.rerequested_at,
        None => None,
    };
    let rerequested_at = edge.max(detail.review_requested_at);
    let last_reviewed_sha = prev.and_then(|p| p.last_reviewed_sha.clone());
    let last_reviewed_at = prev.and_then(|p| p.last_reviewed_at);
    let input = RoundInput {
        state: detail.state,
        head_sha: &detail.head_sha,
        last_reviewed_sha: last_reviewed_sha.as_deref(),
        last_reviewed_at,
        approved_at,
        rerequested_at,
        threads,
        me,
        seen_at: None,
        my_state: detail.my_review_state,
        platform_reviewed_sha: detail.my_reviewed_sha.as_deref(),
        platform_reviewed_at: detail.my_reviewed_at,
    };
    let round = classify_with(&input, touches);
    let round = keep_unanswered(prev, stored, round, &input, touches);
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
        my_review_state: detail.my_review_state,
        my_reviewed_at: detail.my_reviewed_at,
        my_reviewed_sha: detail.my_reviewed_sha.clone(),
        listed,
        viewer: me.to_string(),
        fingerprint: String::new(),
        fetched_at: now,
    }
}

/// The user finished this round: the watermark moves to the current head,
/// and the review is classified again (normally to
/// [`ReviewStatus::WaitingOnAuthor`]).
///
/// The watermark's time is what the user saw, not the click: the start of
/// the last stored fetch (`fetched_at`), never past `now`. A reply that came
/// in after that is after the watermark and starts a round once fetched,
/// instead of being taken as seen: one made between the last fetch and the
/// click, and one a fetch missed because it landed on a page already read.
/// Not the newest stored note either, for that second case: a later note
/// on another page proves nothing about the pages read before it. The cost
/// is the other way round and rare: a note made while the fetch was on the
/// network that it did bring counts as new (a round the user can mark
/// again), never a reply hidden.
pub fn mark_reviewed(
    record: &ReviewRecord,
    threads: &[Thread],
    now: DateTime<Utc>,
) -> ReviewRecord {
    let mut next = record.clone();
    next.last_reviewed_sha = Some(record.head_sha.clone());
    next.last_reviewed_at = Some(record.fetched_at.min(now));
    let round = classify(&RoundInput::of(&next, threads));
    next.status = round.status;
    next.round_reasons = round.reasons;
    next.untouched_threads = round.untouched_threads;
    next
}

/// A reply found unanswered stays so until the user acts: a fetch knows
/// only what the user saw when a note was posted right after the post
/// ([`RoundInput::seen_at`]); the fetches after it would take the user's
/// note as the answer. So [`RoundReason::AuthorReplied`] stands while the
/// watermark is where it was and the user wrote nothing the stored threads
/// lack (a new note of theirs is classified afresh). New is by note id, not
/// time: the platform's clock and this machine's need not agree. The
/// untouched count is then read as the kept reason says
/// ([`untouched_agreeing`]).
fn keep_unanswered(
    prev: Option<&ReviewRecord>,
    stored: &[Thread],
    mut round: Round,
    input: &RoundInput<'_>,
    touches: Option<&LineTouches>,
) -> Round {
    let Some(p) = prev else {
        return round;
    };
    let reviewed = matches!(
        round.status,
        ReviewStatus::NeedsReReview | ReviewStatus::WaitingOnAuthor
    );
    let known: HashSet<&str> = stored
        .iter()
        .flat_map(|t| &t.notes)
        .map(|n| n.id.as_str())
        .collect();
    let wrote_since = input
        .threads
        .iter()
        .flat_map(|t| &t.notes)
        .any(|n| !n.system && same_user(&n.author, input.me) && !known.contains(n.id.as_str()));
    if !reviewed
        || wrote_since
        || p.status != ReviewStatus::NeedsReReview
        || !p.round_reasons.contains(&RoundReason::AuthorReplied)
        || round.reasons.contains(&RoundReason::AuthorReplied)
    {
        return round;
    }
    round.reasons.push(RoundReason::AuthorReplied);
    round.reasons.sort();
    round.status = ReviewStatus::NeedsReReview;
    round.untouched_threads = untouched_agreeing(input, &round.reasons, touches);
    round
}

#[cfg(test)]
mod tests;
