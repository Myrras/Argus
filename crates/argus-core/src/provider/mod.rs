//! Review providers: one [`ReviewProvider`] per platform, normalising into
//! the canonical [`model`].
//!
//! Every call is blocking (as in Podium): run them on a poll thread or a
//! `spawn_blocking` task, never on the UI thread.

pub mod github;
pub mod gitlab;
pub mod model;

pub use github::GithubProvider;
pub use gitlab::{GitlabProvider, KeychainToken, StaticToken, TokenSource};
pub use model::{
    Anchor, CommitInfo, DiffVersion, Note, ProviderId, ReviewDetail, ReviewRef, ReviewState,
    Reviewer, ReviewerState, Side, Thread,
};

use std::sync::Arc;

use crate::error::CoreResult;
use crate::settings::Settings;

/// Read access to one platform, for the current user.
pub trait ReviewProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// The host this provider talks to (`github.com`, a GHE or GitLab host);
    /// matches [`ReviewRef::host`] of what it lists.
    fn host(&self) -> &str;

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

/// The providers `settings` turns on: GitHub unless disabled, GitLab when a
/// base URL is set (its token read from the keychain on each call).
pub fn from_settings(settings: &Settings) -> Vec<Arc<dyn ReviewProvider>> {
    let mut out: Vec<Arc<dyn ReviewProvider>> = Vec::new();
    if settings.github.enabled {
        out.push(Arc::new(GithubProvider::new(
            settings.github.host.clone(),
            settings.github.account.clone(),
        )));
    }
    if settings.gitlab_enabled() {
        let url = &settings.gitlab.base_url;
        match GitlabProvider::new(url, KeychainToken::for_base_url(url)) {
            Ok(p) => out.push(Arc::new(p)),
            // `Settings::validated` rejects such a URL before it is saved;
            // a hand-edited file lands here.
            Err(_) => tracing::warn!("ignoring the GitLab URL in settings: not http(s)"),
        }
    }
    out
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
