//! Review providers: one [`ReviewProvider`] per platform, normalising into
//! the canonical [`model`].
//!
//! Every call is blocking (as in Podium): run them on a poll thread or a
//! `spawn_blocking` task, never on the UI thread.

pub mod github;
pub mod model;

pub use github::GithubProvider;
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

/// Percent-encode one URL path segment (RFC 3986 unreserved kept).
pub(crate) fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-encode a slash-separated path, keeping the slashes.
pub(crate) fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// The first line of a commit message.
pub(crate) fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_paths() {
        assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
        assert_eq!(encode_path("src/my file.rs"), "src/my%20file.rs");
        assert_eq!(first_line("Fix it\n\nbody"), "Fix it");
    }
}
