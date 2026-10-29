//! The AI pass (milestone 5): a local agent CLI (`claude`, `auggie`) reviews
//! the diff headless and returns findings, a "look here yourself" checklist
//! and, in later rounds, a verdict per open thread of the user.
//!
//! - [`adapter`]: the CLIs, their read-only flags, the token-free env, the
//!   spawn/timeout/kill loop.
//! - [`auggie`]: which auggie a binary is (from its `--help`), and its
//!   argv for a review or a "Write with AI" turn.
//! - [`prompt`]: the prompt text (consts) and its assembly.
//! - [`output`]: JSON extraction and strict validation of the answer.
//! - [`dedupe`]: sets aside findings and checklist items the review
//!   already makes (an existing thread, a posted finding): the agent's own
//!   `relatedThreadIds`, checked against the text, plus a text net.
//! - [`context`]: the repository's rules files, read at the target branch.
//! - [`skills`]: review skills (the user's, plugins', the project's) and
//!   the one a run follows.
//! - [`job`]: one run end to end (diff, review worktree, rules and skill,
//!   prompt, retry).
//! - [`runtime`]: queueing (per review serial, two at once), `ai:state`,
//!   and the shutdown that kills running agents on quit.
//! - `store`: SQLite (the v1 tables plus `ai_runs`, migration 5, and the
//!   repeat marks of migrations 9 and 10).
//! - `api`: the [`Core`](crate::Core) methods.
//!
//! A run only reads from the platforms (`fetch_files`) and posts nothing.
//! A finding reaches a platform only when the user posts it
//! (`Core::post_findings`, through the post API).

pub(crate) mod adapter;
mod api;
pub(crate) mod auggie;
pub(crate) mod context;
pub(crate) mod dedupe;
pub(crate) mod job;
pub(crate) mod output;
pub mod prompt;
pub(crate) mod runtime;
pub mod skills;
pub(crate) mod store;

pub mod model;

pub use job::sweep_stale_temp_dirs;
pub use model::{
    AiRunState, AiRunStatus, ChecklistId, ChecklistItem, Finding, FindingId, FindingPost,
    FindingPostOutcome, FindingPostResult, RelatedRef, RepeatOf, RepeatReason, RunContext,
    Severity, UsedSkill, Verdict, VerdictLabel,
};
pub use prompt::ThreadSnapshot;
pub use runtime::AiStateCallback;
pub use skills::{ReviewSkill, ReviewSkillList};

use std::time::Duration;

use crate::provider::ReviewState;
use crate::store::ReviewRecord;
use store::RunRow;

/// At most this many runs at once, across all reviews.
pub const MAX_CONCURRENT: usize = 2;

/// A run that takes longer is killed and fails.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How long quitting waits for cancelled runs to end (their agents are
/// killed at once; this is for their threads to clean up).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Error text of a run the user (or turning the AI pass off) stopped.
pub(crate) const CANCELLED: &str = "cancelled";

/// Whether a refresh should start a run of `record` on its own: an open,
/// active review that never had a run, or whose round moved past the last
/// successful one. A run that failed is not retried for the same round
/// (the user can start one) unless it was cut off by Argus stopping.
pub(crate) fn needs_run(record: &ReviewRecord, run: Option<&RunRow>) -> bool {
    if record.state != ReviewState::Open || !record.is_active() {
        return false;
    }
    let Some(run) = run else {
        return true;
    };
    match run.status {
        model::AiRunStatus::Running => false,
        model::AiRunStatus::Failed if run.interrupted => true,
        model::AiRunStatus::Failed => {
            record.round > run.round && run.last_done_round.is_none_or(|d| record.round > d)
        }
        model::AiRunStatus::Done | model::AiRunStatus::NotStarted => {
            run.last_done_round.is_none_or(|d| record.round > d)
        }
    }
}

#[cfg(test)]
mod guarantees;
#[cfg(test)]
mod tests;
