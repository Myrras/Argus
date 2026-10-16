//! The AI pass on [`Core`]. Blocking but quick (SQLite); runs themselves
//! happen on their own threads and report through [`Core::on_ai_state`].

use std::sync::Arc;

use chrono::Utc;

use super::job::JobContext;
use super::model::{
    AiRunState, ChecklistId, ChecklistItem, Finding, FindingId, FindingPost, FindingPostOutcome,
    FindingPostResult, Verdict, VerdictLabel,
};
use super::store as ai_store;
use super::{needs_run, RUN_TIMEOUT};
use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::poll::RefreshReport;
use crate::post::{NewComment, PostError, PostErrorKind, PostResult};
use crate::provider::Side;
use crate::store::ReviewRecord;
use crate::Core;

impl Core {
    /// The review's latest AI run (`notStarted` when there is none, or the
    /// AI pass is off).
    pub fn ai_state(&self, id: ReviewId) -> CoreResult<AiRunState> {
        self.review(id)?;
        self.ai.state(id)
    }

    /// The review's findings, every round: high severity first, then path
    /// and line. Empty when the AI pass is off.
    pub fn findings(&self, id: ReviewId) -> CoreResult<Vec<Finding>> {
        self.review(id)?;
        if !self.ai.enabled() {
            return Ok(Vec::new());
        }
        ai_store::findings(&self.store, id)
    }

    /// The review's "look here yourself" items, newest round first. Empty
    /// when the AI pass is off.
    pub fn checklist(&self, id: ReviewId) -> CoreResult<Vec<ChecklistItem>> {
        self.review(id)?;
        if !self.ai.enabled() {
            return Ok(Vec::new());
        }
        ai_store::checklist(&self.store, id)
    }

    /// The current verdict per thread of the review (its latest round's).
    /// Empty when the AI pass is off.
    pub fn verdicts(&self, id: ReviewId) -> CoreResult<Vec<Verdict>> {
        self.review(id)?;
        if !self.ai.enabled() {
            return Ok(Vec::new());
        }
        ai_store::verdicts(&self.store, id)
    }

    /// Start (queue) a run of the review now, whatever ran before.
    pub fn run_ai(&self, id: ReviewId) -> CoreResult<AiRunState> {
        let record = self.review(id)?;
        if !self.ai.enabled() {
            return Err(CoreError::InvalidInput(
                "the AI pass is turned off in Settings".into(),
            ));
        }
        let ctx = self.ai_context(&record)?;
        self.ai.enqueue(id, ctx);
        self.ai.state(id)
    }

    /// Drop a queued run of the review, or stop a running one.
    pub fn cancel_ai(&self, id: ReviewId) -> CoreResult<AiRunState> {
        self.review(id)?;
        self.ai.cancel(id);
        self.ai.state(id)
    }

    /// The poll hook: queue a run for every review that needs one (see
    /// [`needs_run`]: new, or in a round past its last successful run).
    /// Never blocks on a run. Returns the reviews queued; nothing when the
    /// AI pass is off.
    pub fn on_refresh_report(&self, _report: &RefreshReport) -> Vec<ReviewId> {
        if !self.ai.enabled() {
            return Vec::new();
        }
        // Every stored review, not only `report.changed`: a run cut off by a
        // restart, or one skipped while the AI pass was off, is picked up
        // too. Few reviews, one query each.
        let (Ok(reviews), Ok(runs)) = (self.store.reviews(), ai_store::runs(&self.store)) else {
            return Vec::new();
        };
        let mut queued = Vec::new();
        for record in reviews {
            if !needs_run(&record, runs.get(&record.id)) || self.ai.busy(record.id) {
                continue;
            }
            // A review whose provider is not set up is skipped quietly.
            let Ok(ctx) = self.ai_context(&record) else {
                continue;
            };
            if self.ai.enqueue(record.id, ctx) {
                queued.push(record.id);
            }
        }
        queued
    }

    /// Register the `ai:state` hook: called (on a run's thread) after every
    /// state change of a run, with the review's new [`AiRunState`].
    pub fn on_ai_state(&self, callback: impl Fn(&AiRunState) + Send + Sync + 'static) {
        self.ai.set_callback(Arc::new(callback));
    }

    /// Dismiss a finding (or bring it back).
    pub fn dismiss_finding(&self, id: FindingId, dismissed: bool) -> CoreResult<Finding> {
        ai_store::set_dismissed(&self.store, id, dismissed)
    }

    pub fn set_checklist_checked(
        &self,
        id: ChecklistId,
        checked: bool,
    ) -> CoreResult<ChecklistItem> {
        ai_store::set_checked(&self.store, id, checked)
    }

    /// The user's own label on a thread (`None` clears it), stored apart
    /// from the AI's hint. Local only: never written to the platform.
    pub fn set_verdict_override(
        &self,
        thread: ThreadId,
        label: Option<VerdictLabel>,
    ) -> CoreResult<Verdict> {
        ai_store::set_override(&self.store, thread, label)
    }

    /// Post findings of the review as line comments, one by one in order
    /// (an explicit user action, through [`post_comments`](Core::post_comments)).
    ///
    /// Each [`FindingPost`] names a finding and may carry the user's edit of
    /// its text; without one the comment is `**title**` and the body. A
    /// finding that cannot go out (unknown, of another review, already
    /// posted, line no longer in the diff, refused by the platform) is
    /// reported in its slot and the rest still go out, so the user can fix
    /// or retry what is left. Each posted finding gets its `posted_at` and
    /// `posted_url`; a failed one stays unposted.
    ///
    /// `Err` only for failures before anything was posted (see
    /// [`post_comments`](Core::post_comments)). Blocking: use
    /// `spawn_blocking`.
    pub fn post_findings(
        &self,
        id: ReviewId,
        posts: Vec<FindingPost>,
    ) -> Result<Vec<FindingPostResult>, PostError> {
        self.review(id)?;
        let failed =
            |kind, message: &str| FindingPostOutcome::Failed(PostError::new(kind, message));
        let mut outcomes: Vec<Option<FindingPostOutcome>> = Vec::with_capacity(posts.len());
        let mut found: Vec<(usize, Finding)> = Vec::new();
        let mut comments = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, post) in posts.iter().enumerate() {
            if !seen.insert(post.finding_id) {
                outcomes.push(Some(failed(
                    PostErrorKind::InvalidInput,
                    "this finding is listed twice",
                )));
                continue;
            }
            match ai_store::finding(&self.store, post.finding_id)? {
                Some(f) if f.review_id == id && f.posted_at.is_some() => {
                    outcomes.push(Some(failed(
                        PostErrorKind::InvalidInput,
                        "this finding was already posted",
                    )))
                }
                Some(f) if f.review_id == id => {
                    let (head_sha, base_sha) = ai_store::finding_shas(&self.store, f.id)?;
                    // Old-side lines are numbered against the left side the
                    // agent read; without it (stored before migration 6)
                    // they could land on another line of the base.
                    if f.side == Side::Old && base_sha.is_none() {
                        outcomes.push(Some(failed(
                            PostErrorKind::LineNotInDiff,
                            "this finding is from before Argus kept the diff it was read at; re-run the AI pass to post it",
                        )));
                        continue;
                    }
                    comments.push(NewComment {
                        path: f.path.clone(),
                        line: f.line,
                        side: f.side,
                        start_line: None,
                        body: post
                            .body
                            .clone()
                            .unwrap_or_else(|| format!("**{}**\n\n{}", f.title, f.body)),
                        // The diff the agent read: the post API moves a
                        // new-side line forward from its head, and refuses
                        // an old-side one whose left side is not the
                        // review's diff's (a run since the last review).
                        // `None` (an older finding): the stored head.
                        head_sha,
                        base_sha,
                    });
                    found.push((i, f));
                    outcomes.push(None);
                }
                _ => outcomes.push(Some(failed(
                    PostErrorKind::NotFound,
                    "no such finding on this review",
                ))),
            }
        }
        if !comments.is_empty() {
            let results = self.post_comments(id, comments)?;
            for ((i, f), result) in found.into_iter().zip(results) {
                outcomes[i] = Some(match result {
                    PostResult::Posted(p) => {
                        let now = Utc::now();
                        // The comment is on the platform either way: a
                        // failed bookkeeping write is only logged.
                        if let Err(e) = ai_store::mark_posted(&self.store, f.id, &p.url, now) {
                            tracing::warn!("could not mark a finding as posted: {e}");
                        }
                        FindingPostOutcome::Posted {
                            posted_url: p.url,
                            posted_at: now,
                        }
                    }
                    PostResult::Failed(e) => FindingPostOutcome::Failed(e),
                });
            }
        }
        Ok(posts
            .iter()
            .zip(outcomes)
            .map(|(post, outcome)| FindingPostResult {
                finding_id: post.finding_id,
                outcome: outcome.expect("every finding has an outcome"),
            })
            .collect())
    }

    fn ai_context(&self, record: &ReviewRecord) -> CoreResult<JobContext> {
        Ok(JobContext {
            settings: self.settings(),
            provider: self.provider_for(record)?,
            timeout: RUN_TIMEOUT,
        })
    }
}
