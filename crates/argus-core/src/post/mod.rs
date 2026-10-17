//! Posting: new line comments, replies and approvals, written to the
//! platform only when the user asks ([`Core`](crate::Core)'s post API in
//! [`api`]; nothing in the poll loop or anywhere else writes).
//!
//! v1 posts comments one by one, so a failure is reported per comment and
//! never loses the others. Argus never merges, never resolves threads and
//! never changes code.
//!
//! Errors are [`PostError`] (`{ kind, message }`, the shape of the shell's
//! `IpcError`) rather than [`CoreError`]: a rejected comment carries the
//! platform's own reason, and its kinds (`postRejected`, `lineNotInDiff`,
//! `headMoved`) sit beside the [`CoreError`] ones.

mod anchor;
mod api;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::ids::ReviewId;
use crate::provider::Side;

pub use anchor::{
    commentable_hunks, map_forward, place, resolve, CommentableHunk, DiffLine, LineKind, Placement,
};

/// A new line comment as the user (or, later, an AI finding) wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewComment {
    /// The file's path at head.
    pub path: String,
    /// The (last) line, on [`side`](Self::side).
    pub line: u32,
    pub side: Side,
    /// First line of a multi-line comment, on the same side.
    #[serde(default)]
    pub start_line: Option<u32>,
    pub body: String,
    /// The head the line numbers were read at; `None` means the review's
    /// head as Argus last stored it (what the code view shows). When the
    /// platform's head moved since, the line is mapped forward.
    #[serde(default)]
    pub head_sha: Option<String>,
    /// The diff's left side the old-side line numbers were read at
    /// (`ReviewFiles::base_sha`). When given and the left side moved since
    /// (a rebase), an old-side comment fails instead of landing elsewhere.
    #[serde(default)]
    pub base_sha: Option<String>,
}

/// A comment placed on the review's current diff: what a provider posts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedComment {
    /// The path at head (for a removed file, the path it had).
    pub path: String,
    /// The path at the base: [`path`](Self::path) unless renamed.
    pub old_path: String,
    pub side: Side,
    /// The (last) row the comment is on.
    pub line: DiffLine,
    /// The first row of a range, in the same hunk.
    pub start: Option<DiffLine>,
    /// The head the diff was read at; GitHub's `commit_id`, and on GitLab
    /// the head its latest version must still have.
    pub head_sha: String,
    /// The diff's left side.
    pub base_sha: String,
    pub body: String,
}

impl PlacedComment {
    /// The (last) line on [`side`](Self::side).
    pub fn line_number(&self) -> u32 {
        side_line(&self.line, self.side)
    }

    /// The first line of a range on [`side`](Self::side).
    pub fn start_number(&self) -> Option<u32> {
        self.start.as_ref().map(|s| side_line(s, self.side))
    }
}

fn side_line(row: &DiffLine, side: Side) -> u32 {
    // A placed row is on its side by construction (see `anchor::place`).
    row.line_on(side).unwrap_or(0)
}

/// A comment the platform accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostedComment {
    /// The comment on the platform's web UI.
    pub url: String,
    /// The platform's thread id (GitHub: the comment id, which roots the
    /// thread; GitLab: the discussion id), as in `Thread::id`.
    pub provider_thread_id: String,
    /// The platform's note/comment id, as in `Note::id`.
    pub provider_note_id: String,
    /// Where it landed, after the move to the current head.
    pub path: String,
    pub side: Side,
    pub line: u32,
    pub start_line: Option<u32>,
    /// The head it was posted against.
    pub head_sha: String,
}

/// A reply the platform accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostedNote {
    pub url: String,
    pub provider_thread_id: String,
    pub provider_note_id: String,
}

/// The outcome of one comment of a batch, in the batch's order.
///
/// Wire shape: `{ "status": "posted", url, providerThreadId, ... }` or
/// `{ "status": "failed", kind, message }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PostResult {
    Posted(PostedComment),
    Failed(PostError),
}

impl PostResult {
    pub fn is_posted(&self) -> bool {
        matches!(self, PostResult::Posted(_))
    }
}

/// The current diff's commentable lines of one file, for the code view's
/// gutter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentableLines {
    pub review_id: ReviewId,
    pub path: String,
    /// The diff these lines are of.
    pub base_sha: String,
    pub head_sha: String,
    /// Empty for a file without a text patch (binary, too large).
    pub hunks: Vec<CommentableHunk>,
}

/// [`Core::commentable_lines`](crate::Core::commentable_lines)'s cache: one
/// review's hunks per path, at the diff they were read from.
#[derive(Debug, Clone)]
pub(crate) struct CommentableCache {
    pub review: ReviewId,
    pub base_sha: String,
    pub head_sha: String,
    /// `base_sha` is still the platform's stand-in: the local clone lacked
    /// a commit when last asked, and may have it after a fetch.
    pub base_approximate: bool,
    /// The head and base the review was stored at when read (the cache key):
    /// a retarget leaves the head alone but changes the diff.
    pub stored_head: String,
    pub stored_base: String,
    pub files: std::collections::HashMap<String, Vec<CommentableHunk>>,
}

/// What a post failure is, for the UI to branch on. The [`CoreError`] kinds
/// keep the names the shell's `IpcError` gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PostErrorKind {
    NotFound,
    InvalidInput,
    GithubUnavailable,
    GitlabNotConfigured,
    GitlabAuth,
    Provider,
    Store,
    Io,
    /// The platform refused the write; the message carries its reason.
    PostRejected,
    /// The line is not (any more) on a hunk row of the current diff.
    LineNotInDiff,
    /// The review got new commits between reading its diff and posting.
    HeadMoved,
}

impl PostErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PostErrorKind::NotFound => "notFound",
            PostErrorKind::InvalidInput => "invalidInput",
            PostErrorKind::GithubUnavailable => "githubUnavailable",
            PostErrorKind::GitlabNotConfigured => "gitlabNotConfigured",
            PostErrorKind::GitlabAuth => "gitlabAuth",
            PostErrorKind::Provider => "provider",
            PostErrorKind::Store => "store",
            PostErrorKind::Io => "io",
            PostErrorKind::PostRejected => "postRejected",
            PostErrorKind::LineNotInDiff => "lineNotInDiff",
            PostErrorKind::HeadMoved => "headMoved",
        }
    }
}

/// A failed write (or a failure before it).
///
/// `message` is app-owned text, except for [`PostErrorKind::PostRejected`]
/// (and a GitLab 404 on a write), where it is the platform's own error
/// message — read from its JSON error body only, run through [`sanitize`]
/// (the token removed, one line, capped), never a raw response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct PostError {
    pub kind: PostErrorKind,
    pub message: String,
}

impl PostError {
    pub fn new(kind: PostErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<CoreError> for PostError {
    fn from(e: CoreError) -> Self {
        let kind = match &e {
            CoreError::NotFound => PostErrorKind::NotFound,
            CoreError::InvalidInput(_) => PostErrorKind::InvalidInput,
            CoreError::GithubUnavailable => PostErrorKind::GithubUnavailable,
            CoreError::GitlabNotConfigured => PostErrorKind::GitlabNotConfigured,
            CoreError::GitlabAuth => PostErrorKind::GitlabAuth,
            CoreError::Provider(_) => PostErrorKind::Provider,
            CoreError::Store(_) => PostErrorKind::Store,
            CoreError::Io(_) => PostErrorKind::Io,
        };
        Self::new(kind, e.to_string())
    }
}

/// The longest platform message forwarded.
const MAX_MESSAGE: usize = 300;

/// Make a platform's error text safe to show: every `secrets` occurrence
/// redacted, control characters and line breaks folded into single spaces,
/// capped at [`MAX_MESSAGE`] characters.
pub(crate) fn sanitize(text: &str, secrets: &[&str]) -> String {
    let mut text = text.to_string();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(s, "[redacted]");
    }
    let folded: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_MESSAGE {
        out = out.chars().take(MAX_MESSAGE).collect::<String>() + "…";
    }
    out
}
