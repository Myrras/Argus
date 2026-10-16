//! The AI pass (milestone 5): a local agent CLI (`claude`, `auggie`) reviews
//! the diff headless and returns findings, a "look here yourself" checklist
//! and, in later rounds, a verdict per open thread of the user.
//!
//! - [`adapter`]: the CLIs, their read-only flags, the token-free env, the
//!   spawn/timeout/kill loop.
//! - [`prompt`]: the prompt text (consts) and its assembly.
//! - [`output`]: JSON extraction and strict validation of the answer.
//! - [`job`]: one run end to end (diff, workspace, prompt, retry).
//! - [`runtime`]: queueing (per review serial, two at once) and `ai:state`.
//! - `store`: SQLite (the v1 tables plus `ai_runs`, migration 5).
//! - `api`: the [`Core`](crate::Core) methods.
//!
//! The AI pass only reads from the platforms (`fetch_files`); nothing it
//! produces is ever posted by this module.

pub(crate) mod adapter;
mod api;
pub(crate) mod job;
pub(crate) mod output;
pub mod prompt;
pub(crate) mod runtime;
pub(crate) mod store;

pub mod model;

pub use model::{
    AiRunState, AiRunStatus, ChecklistId, ChecklistItem, Finding, FindingId, Severity, Verdict,
    VerdictLabel,
};
pub use prompt::ThreadSnapshot;
pub use runtime::AiStateCallback;

use std::time::Duration;

use crate::provider::ReviewState;
use crate::store::ReviewRecord;
use store::RunRow;

/// At most this many runs at once, across all reviews.
pub const MAX_CONCURRENT: usize = 2;

/// A run that takes longer is killed and fails.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
