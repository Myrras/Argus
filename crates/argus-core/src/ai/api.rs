//! The AI pass on [`Core`]. Blocking but quick (SQLite); runs themselves
//! happen on their own threads and report through [`Core::on_ai_state`].

use std::sync::Arc;

use super::job::JobContext;
use super::model::{
    AiRunState, ChecklistId, ChecklistItem, Finding, FindingId, Verdict, VerdictLabel,
};
use super::store as ai_store;
use super::{needs_run, RUN_TIMEOUT};
use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::poll::RefreshReport;
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

    fn ai_context(&self, record: &ReviewRecord) -> CoreResult<JobContext> {
        Ok(JobContext {
            settings: self.settings(),
            provider: self.provider_for(record)?,
            timeout: RUN_TIMEOUT,
        })
    }
}
