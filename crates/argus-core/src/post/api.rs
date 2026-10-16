//! The post API on [`Core`]: the only code in Argus that writes to a
//! platform, and only when called (each call is one explicit user action).
//!
//! All of it blocks (network, `gh`): call it from `spawn_blocking`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::anchor;
use super::{
    commentable_hunks, CommentableLines, NewComment, PostError, PostErrorKind, PostResult,
    PostedComment, PostedNote,
};
use crate::error::{CoreError, CoreResult};
use crate::files;
use crate::ids::{ReviewId, ThreadId};
use crate::poll;
use crate::provider::{DiffRange, FileDiff, ReviewProvider, Side, Thread};
use crate::review::{self, ReviewStatus, RoundInput};
use crate::store::{ReviewRecord, StoredThread};
use crate::Core;

impl Core {
    /// Post `comments` on a review as new line comments, one by one in
    /// order; the results come back in the same order.
    ///
    /// The review's head and diff are read from the platform first, each
    /// comment is moved to that head and placed on its diff
    /// ([`resolve`](super::resolve)), then posted. A comment that fails (bad
    /// input, line no longer in the diff, the platform refused) is reported
    /// in its slot and the rest still go out. After at least one comment
    /// was posted the review's threads are refreshed and its watermark
    /// moves to the head the user was looking at (see
    /// [`advance_after_write`]).
    ///
    /// `Err` only for failures before anything was posted: the review or
    /// its provider is unknown, or its head or diff could not be read.
    pub fn post_comments(
        &self,
        id: ReviewId,
        comments: Vec<NewComment>,
    ) -> Result<Vec<PostResult>, PostError> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record)?;
        let mut results: Vec<Option<PostResult>> = comments
            .iter()
            .map(|c| anchor::validate(c).err().map(PostResult::Failed))
            .collect();
        if results.iter().all(Option::is_some) {
            return Ok(results.into_iter().flatten().collect());
        }
        // The platform's head now, not the stored one: comments go on it.
        let detail = provider.fetch_review(&record.review)?;
        let current = provider.fetch_files(
            &record.review,
            &DiffRange::Review {
                base: detail.base_sha.clone(),
                head: detail.head_sha.clone(),
            },
        )?;
        // Per head a comment was written at: the diff from it to now.
        let mut since: HashMap<String, Result<FileDiff, PostError>> = HashMap::new();
        let mut posted_any = false;
        for (c, slot) in comments.iter().zip(results.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            let written_at = c.head_sha.as_deref().unwrap_or(&record.head_sha);
            let outcome = post_one(
                provider.as_ref(),
                &record,
                c,
                written_at,
                &current,
                &mut since,
            );
            posted_any |= outcome.is_ok();
            *slot = Some(match outcome {
                Ok(p) => PostResult::Posted(p),
                Err(e) => PostResult::Failed(e),
            });
        }
        if posted_any {
            self.after_write_logged(provider.as_ref(), &record, false);
        }
        Ok(results.into_iter().flatten().collect())
    }

    /// Reply to a stored thread. Then refreshes the review and moves its
    /// watermark, as after posting.
    pub fn reply_to_thread(&self, thread: ThreadId, body: &str) -> Result<PostedNote, PostError> {
        if body.trim().is_empty() {
            return Err(PostError::new(
                PostErrorKind::InvalidInput,
                "the reply is empty",
            ));
        }
        let (record, stored) = self.find_thread(thread)?;
        let provider = self.provider_for(&record)?;
        let note = provider.reply(&record.review, &stored.thread.id, body)?;
        self.after_write_logged(provider.as_ref(), &record, false);
        Ok(note)
    }

    /// Approve a review on its platform (a separate, explicit action; never
    /// a merge), pinned to `seen_head`: the head the user reviewed and was
    /// shown in the confirmation. Fails with [`PostErrorKind::HeadMoved`]
    /// (nothing written) when the stored or the platform's head is another
    /// one, so new commits are never approved unseen.
    ///
    /// Then refreshes it: `approved_at` is set (by the platform's approval
    /// showing up, or right away when it does not show yet), which takes
    /// the review out of the active lists, and the watermark moves. Returns
    /// the review as stored afterwards.
    pub fn approve_review(&self, id: ReviewId, seen_head: &str) -> Result<ReviewRecord, PostError> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record)?;
        if seen_head.trim().is_empty() {
            return Err(PostError::new(
                PostErrorKind::InvalidInput,
                "no head to approve",
            ));
        }
        let moved = || {
            PostError::new(
                PostErrorKind::HeadMoved,
                "The review got new commits since you looked; review them before approving.",
            )
        };
        if seen_head != record.head_sha {
            return Err(moved());
        }
        // The platform's head now: GitHub would approve an older commit
        // without complaint (GitLab refuses by itself, see its `approve`).
        if provider.fetch_review(&record.review)?.head_sha != seen_head {
            return Err(moved());
        }
        provider.approve(&record.review, seen_head)?;
        Ok(match self.after_write(provider.as_ref(), &record, true) {
            Ok(next) => next,
            Err(e) => {
                // Approved on the platform all the same; the next refresh
                // catches up.
                tracing::debug!("storing the approval failed: {e}");
                record
            }
        })
    }

    /// The lines of `path` a new comment can go on, per hunk of the review's
    /// diff at its stored head (what the code view shows). Blocking.
    pub fn commentable_lines(&self, id: ReviewId, path: &str) -> CoreResult<CommentableLines> {
        let record = self.review(id)?;
        let diff = self
            .provider_for(&record)?
            .fetch_files(&record.review, &files::review_range(&record))?;
        let file =
            diff.files.iter().find(|f| f.path == path).ok_or_else(|| {
                CoreError::InvalidInput("not a changed file of this review".into())
            })?;
        Ok(CommentableLines {
            review_id: id,
            path: file.path.clone(),
            hunks: file
                .patch
                .as_deref()
                .map(commentable_hunks)
                .unwrap_or_default(),
            base_sha: diff.base_sha,
            head_sha: diff.head_sha,
        })
    }

    /// The stored thread `id` and its review.
    fn find_thread(&self, id: ThreadId) -> CoreResult<(ReviewRecord, StoredThread)> {
        for record in self.store.reviews()? {
            if let Some(t) = self
                .store
                .threads(record.id)?
                .into_iter()
                .find(|t| t.thread_id == id)
            {
                return Ok((record, t));
            }
        }
        Err(CoreError::NotFound)
    }

    /// [`after_write`](Self::after_write) where a failure must not turn a
    /// successful post into an error: logged, and the next poll catches up.
    fn after_write_logged(
        &self,
        provider: &dyn ReviewProvider,
        before: &ReviewRecord,
        approved: bool,
    ) {
        if let Err(e) = self.after_write(provider, before, approved) {
            tracing::debug!("updating the review after a post failed: {e}");
        }
    }

    /// After a successful write: refresh the review through the poll's own
    /// path (so the new thread or note is stored), then move the watermark.
    fn after_write(
        &self,
        provider: &dyn ReviewProvider,
        before: &ReviewRecord,
        approved: bool,
    ) -> CoreResult<ReviewRecord> {
        let now = Utc::now();
        if let Err(e) = self.refresh_review(provider, before.id, now) {
            // The write went through; the watermark still moves.
            tracing::debug!("refreshing after a post failed: {e}");
        }
        let record = self.review(before.id)?;
        let threads: Vec<Thread> = self
            .store
            .threads(before.id)?
            .into_iter()
            .map(|t| t.thread)
            .collect();
        let next = advance_after_write(&record, &threads, &before.head_sha, approved, now);
        self.store.upsert_review(&next)?;
        Ok(next)
    }

    /// Fetch one review and its threads and store them, as a poll would.
    fn refresh_review(
        &self,
        provider: &dyn ReviewProvider,
        id: ReviewId,
        now: DateTime<Utc>,
    ) -> CoreResult<()> {
        let prev = self.review(id)?;
        let me = provider.current_user()?;
        let detail = provider.fetch_review(&prev.review)?;
        let threads = provider.fetch_threads(&prev.review)?;
        poll::persist(
            &self.store,
            Some(&prev),
            &detail,
            &threads,
            &me,
            prev.listed,
            now,
        )?;
        Ok(())
    }
}

/// Resolve one comment against the current diff and post it.
fn post_one(
    provider: &dyn ReviewProvider,
    record: &ReviewRecord,
    c: &NewComment,
    written_at: &str,
    current: &FileDiff,
    since: &mut HashMap<String, Result<FileDiff, PostError>>,
) -> Result<PostedComment, PostError> {
    // Only a new-side line moves with the head (see `resolve`).
    let since_diff = if c.side == Side::New && written_at != current.head_sha {
        let diff = since.entry(written_at.to_string()).or_insert_with(|| {
            provider
                .fetch_files(
                    &record.review,
                    &DiffRange::Compare {
                        from: written_at.to_string(),
                        to: current.head_sha.clone(),
                    },
                )
                .map_err(PostError::from)
        });
        Some(diff.as_ref().map_err(Clone::clone)?)
    } else {
        None
    };
    let placed = anchor::resolve(c, written_at, current, since_diff)?;
    provider.post_comment(&record.review, &placed)
}

/// The record after a successful write (plan §7: the watermark moves when
/// the user marks the round reviewed, posts or approves).
///
/// [`review::mark_reviewed`] moves it to the record's head; the watermark
/// then goes to `seen_head` instead — the head the user was looking at when
/// they posted. When the platform's head moved past it, the commits in
/// between were never seen, so the review stays (or becomes) "needs
/// re-review". `approved` sets `approved_at` when the refresh did not show
/// the approval yet (the next refresh keeps or clears it by what the
/// platform says).
pub(crate) fn advance_after_write(
    record: &ReviewRecord,
    threads: &[Thread],
    seen_head: &str,
    approved: bool,
    now: DateTime<Utc>,
) -> ReviewRecord {
    let mut next = review::mark_reviewed(record, threads, now);
    let needs_approval = approved && next.approved_at.is_none();
    if next.last_reviewed_sha.as_deref() == Some(seen_head) && !needs_approval {
        return next;
    }
    next.last_reviewed_sha = Some(seen_head.to_string());
    if needs_approval {
        next.approved_at = Some(now);
    }
    let round = review::classify(&RoundInput {
        state: next.state,
        head_sha: &next.head_sha,
        last_reviewed_sha: next.last_reviewed_sha.as_deref(),
        last_reviewed_at: next.last_reviewed_at,
        approved_at: next.approved_at,
        rerequested_at: next.rerequested_at,
        threads,
        me: &next.viewer,
    });
    // As in `review::advance`: entering re-review starts a round.
    if round.status == ReviewStatus::NeedsReReview && record.status != ReviewStatus::NeedsReReview {
        next.round += 1;
    }
    next.status = round.status;
    next.round_reasons = round.reasons;
    next.untouched_threads = round.untouched_threads;
    next
}
