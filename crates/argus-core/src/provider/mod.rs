//! Review providers: one [`ReviewProvider`] per platform, normalising into
//! the canonical [`model`].
//!
//! Every call is blocking (as in Podium): run them on a poll thread or a
//! `spawn_blocking` task, never on the UI thread.

pub mod model;

pub use model::{
    Anchor, CommitInfo, DiffVersion, Note, ProviderId, ReviewDetail, ReviewRef, ReviewState,
    Reviewer, ReviewerState, Side, Thread,
};

use crate::error::CoreResult;

/// Read access to one platform, for the current user.
pub trait ReviewProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// The current user's username on this platform.
    fn current_user(&self) -> CoreResult<String>;

    /// Open MRs/PRs where the user is a requested reviewer, has reviewed, or
    /// has commented; never the user's own.
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>>;

    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail>;

    /// All line threads (and, on GitLab, MR-level discussions), oldest first.
    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>>;

    /// The content of `path` at commit `sha`.
    fn file_at(&self, r: &ReviewRef, sha: &str, path: &str) -> CoreResult<String>;

    // M4: post_comment(&self, r, c: &NewComment) -> CoreResult<PostedComment>,
    // reply(&self, r, thread: &str, body) and approve(&self, r) go here.
}
