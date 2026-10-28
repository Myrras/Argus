//! Submit review: the user's verdict (approve, request changes, or on
//! GitHub a comment review) set on the platform, with an optional summary.
//! One explicit user action, one review-state write (plus, on GitLab, the
//! summary note, and adding the user as a reviewer when requesting changes
//! as a non-reviewer); the pending comments go out before it through
//! [`Core::post_comments`], from the UI.
//!
//! Blocking (network, `gh`): call it from `spawn_blocking`.

use chrono::Utc;

use super::{PostError, PostErrorKind};
use crate::error::CoreResult;
use crate::files;
use crate::ids::ReviewId;
use crate::provider::{ProviderId, ReviewVerdict};
use crate::store::ReviewRecord;
use crate::Core;

impl Core {
    /// Submit the user's review of `id` on its platform: `verdict`, pinned
    /// to `seen_head` (the head the user reviewed and was shown in the
    /// dialog), with `summary` (markdown; blank means none). Fails with
    /// [`PostErrorKind::HeadMoved`] (nothing written) when the stored or the
    /// platform's head is another one, so new commits are never judged
    /// unseen.
    ///
    /// GitHub needs a summary to request changes or comment; GitLab has no
    /// comment-only review. An approval also marks every file of the
    /// review's diff at `seen_head` viewed (local only; skipped when the
    /// diff moved on meanwhile).
    ///
    /// Then the review is refreshed and the watermark moves to `seen_head`;
    /// an approval sets `approved_at` (taking the review off the active
    /// lists until the platform drops the approval). Returns the review as
    /// stored afterwards.
    pub fn submit_review(
        &self,
        id: ReviewId,
        verdict: ReviewVerdict,
        seen_head: &str,
        summary: Option<&str>,
    ) -> Result<ReviewRecord, PostError> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record)?;
        let summary = summary.map(str::trim).filter(|s| !s.is_empty());
        check_submission(record.review.provider, verdict, seen_head, summary)?;
        let moved = || {
            PostError::new(
                PostErrorKind::HeadMoved,
                "The review got new commits since you looked; review them before submitting.",
            )
        };
        // Either way the stored review is refreshed, so the view the user
        // retries from shows the head that is there now.
        if seen_head != record.head_sha {
            self.refresh_after_head_moved(provider.as_ref(), &record);
            return Err(moved());
        }
        // The platform's head now: GitHub would take a review of an older
        // commit without complaint (GitLab's approve refuses by itself).
        if provider.fetch_head(&record.review)?.head_sha != seen_head {
            self.refresh_after_head_moved(provider.as_ref(), &record);
            return Err(moved());
        }
        if let Err(e) = provider.submit_review(&record.review, verdict, seen_head, summary) {
            if e.kind == PostErrorKind::HeadMoved {
                self.refresh_after_head_moved(provider.as_ref(), &record);
            }
            return Err(e);
        }
        let approved = verdict == ReviewVerdict::Approve;
        if approved {
            if let Err(e) = self.mark_all_viewed(&record, seen_head) {
                // Approved all the same; the marks are a convenience.
                tracing::debug!("marking the files viewed after approving failed: {e}");
            }
        }
        Ok(
            match self.after_write(provider.as_ref(), &record, approved) {
                Ok(next) => next,
                Err(e) => {
                    // Submitted on the platform all the same; the next refresh
                    // catches up.
                    tracing::debug!("storing the submitted review failed: {e}");
                    record
                }
            },
        )
    }

    /// Mark every file of the review's own diff viewed at its content at
    /// `head`; nothing when the platform's diff is of another head (a push
    /// came in: its files were not what the user approved).
    fn mark_all_viewed(&self, record: &ReviewRecord, head: &str) -> CoreResult<()> {
        let diff = self
            .provider_for(record)?
            .fetch_files(&record.review, &files::review_range(record))?;
        if diff.head_sha != head {
            return Ok(());
        }
        let keys = files::content_keys(&diff);
        self.store.set_viewed_all(record.id, &keys, Utc::now())?;
        self.remember_keys(record.id, head, keys);
        Ok(())
    }
}

/// What a submission must carry before anything is read or written.
fn check_submission(
    platform: ProviderId,
    verdict: ReviewVerdict,
    seen_head: &str,
    summary: Option<&str>,
) -> Result<(), PostError> {
    let invalid = |m: &str| Err(PostError::new(PostErrorKind::InvalidInput, m));
    if seen_head.trim().is_empty() {
        return invalid("no head to submit the review for");
    }
    match (platform, verdict, summary) {
        (ProviderId::Gitlab, ReviewVerdict::Comment, _) => Err(PostError::new(
            PostErrorKind::Unsupported,
            "GitLab has no comment-only review: approve or request changes",
        )),
        (ProviderId::Github, ReviewVerdict::RequestChanges, None) => {
            invalid("GitHub needs a summary to request changes")
        }
        (ProviderId::Github, ReviewVerdict::Comment, None) => {
            invalid("GitHub needs a summary for a comment review")
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_submission_carries_what_its_platform_needs() {
        use ProviderId::{Github, Gitlab};
        use ReviewVerdict::{Approve, Comment, RequestChanges};
        let kind = |p, v, s| check_submission(p, v, "c1", s).map_err(|e| e.kind);
        assert_eq!(kind(Github, Approve, None), Ok(()));
        assert_eq!(kind(Gitlab, Approve, None), Ok(()));
        assert_eq!(kind(Gitlab, RequestChanges, None), Ok(()));
        assert_eq!(kind(Github, RequestChanges, Some("Fix the TTL")), Ok(()));
        assert_eq!(kind(Github, Comment, Some("Looks fine")), Ok(()));
        assert_eq!(
            kind(Github, RequestChanges, None),
            Err(PostErrorKind::InvalidInput)
        );
        assert_eq!(
            kind(Github, Comment, None),
            Err(PostErrorKind::InvalidInput)
        );
        assert_eq!(
            kind(Gitlab, Comment, Some("x")),
            Err(PostErrorKind::Unsupported)
        );
        assert_eq!(
            check_submission(Github, Approve, " ", None)
                .unwrap_err()
                .kind,
            PostErrorKind::InvalidInput
        );
    }
}
