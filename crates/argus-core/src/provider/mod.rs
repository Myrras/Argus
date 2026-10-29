//! Review providers: one [`ReviewProvider`] per platform, normalising into
//! the canonical [`model`].
//!
//! Both go through the user's own CLI and its login: GitHub through `gh`,
//! GitLab through `glab`. Argus holds no token of either platform.
//!
//! Every call is blocking (as in Podium): run them on a poll thread or a
//! `spawn_blocking` task, never on the UI thread. Every call is bounded by a
//! timeout (`gh` or `glab` is killed), so a hung call never blocks the poll
//! loop for good.
//!
//! Reads are conditional where the platform allows it (see [`conditional`]):
//! a poll that finds nothing new costs a 304 per call. A rate-limited
//! provider stops calling until the platform's reset (see [`rate_limit`]).

mod conditional;
pub mod github;
mod github_conversation;
mod github_outside;
pub mod gitlab;
mod http;
pub mod model;
mod rate_limit;

pub use github::GithubProvider;
pub use gitlab::GitlabProvider;
pub use model::{
    Anchor, ChangedFile, CommitInfo, DiffRange, DiffVersion, FileDiff, FileStatus, MyReviewState,
    Note, ProviderId, ReviewDetail, ReviewHead, ReviewRef, ReviewState, ReviewVerdict, Reviewer,
    ReviewerState, Side, Thread,
};

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::accounts::ProviderSpec;
use crate::error::{CoreError, CoreResult};
use crate::post::{
    OutsideDiff, PlacedComment, PostError, PostErrorKind, PostedComment, PostedNote,
};

/// Read access to one platform for the current user, plus the explicit
/// writes of M4 (posting, replying, approving, resolving threads).
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

    /// The review's head, base and commits only: for the head checks before
    /// posting and approving and for the history walk, which must not fail
    /// on what they do not use (GitLab's `/reviewers` and `/approvals`,
    /// GitHub's reviews, which fail [`fetch_review`](Self::fetch_review)
    /// rather than guess). The default reads the whole detail.
    fn fetch_head(&self, r: &ReviewRef) -> CoreResult<ReviewHead> {
        self.fetch_review(r).map(ReviewHead::from)
    }

    /// All line threads and the discussions on the review as a whole
    /// (GitLab MR-level discussions; GitHub's conversation comments and
    /// review summaries, threaded as `github_conversation` describes; both
    /// without an anchor), oldest first.
    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>>;

    /// The content of `path` at commit `sha`.
    fn file_at(&self, r: &ReviewRef, sha: &str, path: &str) -> CoreResult<String>;

    /// The changed files of `range`, with the commit each side is read at:
    /// the commits the diff was really computed at (see [`FileDiff`]'s
    /// contract), which for [`DiffRange::Review`] may be newer than the
    /// shas asked for.
    ///
    /// [`DiffRange::Review`]: GitHub `pulls/{n}/files` (up to 3000 files;
    /// large patches are left out) at the PR's current head, with the merge
    /// base of the PR's base and head from the compare API as the left side,
    /// as GitHub's own Files tab diffs three-dot. GitLab
    /// `merge_requests/:iid/diffs` (`changes` on instances before 15.7): the
    /// latest version, with its `base_sha` (the merge base GitLab diffs
    /// from) as the left side.
    ///
    /// [`DiffRange::Compare`]: GitHub `compare/{from}...{to}` (up to 300
    /// files; left side its merge base, which is `from` unless the branch was
    /// rebased); GitLab `repository/compare?straight=true` (left side `from`).
    fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff>;

    // --- M4: writes. Only `Core`'s post API calls these, each on an explicit
    // user action; nothing else in Argus writes to a platform. The defaults
    // refuse, so a read-only provider (or a test double) needs none of them.

    /// What this platform does with a comment on unchanged code outside
    /// the diff's hunks. The default takes none (a read-only provider, a
    /// test double).
    fn outside_diff(&self) -> OutsideDiff {
        OutsideDiff::None
    }

    /// Post one new line comment, already placed on the current diff (see
    /// [`post::resolve_in`](crate::post::resolve_in)).
    ///
    /// GitHub: `POST repos/{o}/{r}/pulls/{n}/comments` with `commit_id`,
    /// `path`, `line`, `side` (+ `start_line`/`start_side`); for lines
    /// outside the diff ([`PlacedComment::in_diff`] `false`) a file comment
    /// (`subject_type: "file"`) quoting them instead. GitLab: `POST
    /// merge_requests/:iid/discussions` with a text position from the latest
    /// diff version, read right before posting (an unchanged line, in a hunk
    /// or not, with both `old_line` and `new_line`).
    fn post_comment(&self, _r: &ReviewRef, _c: &PlacedComment) -> PostResultOf<PostedComment> {
        Err(read_only())
    }

    /// Reply to the platform thread `thread` (GitHub: the root comment id,
    /// or for a conversation thread a new PR comment quoting its first
    /// entry; GitLab: the discussion id).
    fn reply(&self, _r: &ReviewRef, _thread: &str, _body: &str) -> PostResultOf<PostedNote> {
        Err(read_only())
    }

    /// Approve the MR/PR as the current user, pinned to `head_sha` (the
    /// head the user reviewed): GitHub's review `commit_id`, GitLab's
    /// approve `sha` (which GitLab refuses when the head moved: then
    /// [`PostErrorKind::HeadMoved`]). Never merges.
    fn approve(&self, _r: &ReviewRef, _head_sha: &str) -> PostResultOf<()> {
        Err(read_only())
    }

    /// Resolve (`resolved`) or reopen the platform thread `thread` (its id
    /// as in [`reply`](Self::reply)); only for a [`Thread::resolvable`] one.
    ///
    /// GitHub: GraphQL `resolveReviewThread` / `unresolveReviewThread` with
    /// the review thread's node id, read right before from the PR's
    /// `reviewThreads` (never guessed); the PR conversation cannot be
    /// resolved. GitLab: `PUT merge_requests/:iid/discussions/:id` with
    /// `resolved`. A platform refusal (no permission) is
    /// [`PostErrorKind::PostRejected`] with the platform's message.
    fn set_thread_resolved(
        &self,
        _r: &ReviewRef,
        _thread: &str,
        _resolved: bool,
    ) -> PostResultOf<()> {
        Err(not_resolvable())
    }

    // --- Submit review: the user's verdict, set on the platform. Only
    // `Core::submit_review` calls it, on an explicit user action.

    /// Submit the user's review of the MR/PR: `verdict`, pinned to
    /// `head_sha` (the head the user reviewed), with an optional `summary`
    /// (markdown, already trimmed and non-empty).
    ///
    /// GitHub: one `POST pulls/{n}/reviews` with `event` (`APPROVE`,
    /// `REQUEST_CHANGES`, `COMMENT`), `commit_id` and the summary as
    /// `body`. GitLab: approve is [`approve`](Self::approve); request
    /// changes is the `/request_changes` quick action (GitLab 17.2+, else
    /// [`PostErrorKind::Unsupported`] before anything is written), which
    /// GitLab takes only from a reviewer: a user who is not one is added
    /// first (GraphQL `mergeRequestSetReviewers`, `APPEND`); then the
    /// summary as an MR note.
    ///
    /// The default approves through [`approve`](Self::approve) when there
    /// is no summary and refuses everything else.
    fn submit_review(
        &self,
        r: &ReviewRef,
        verdict: ReviewVerdict,
        head_sha: &str,
        summary: Option<&str>,
    ) -> PostResultOf<()> {
        match (verdict, summary) {
            (ReviewVerdict::Approve, None) => self.approve(r, head_sha),
            _ => Err(read_only()),
        }
    }

    // --- M3: thread history ---

    /// What commit `sha` changed, against its first parent (the thread
    /// history's line walk). GitHub `repos/{o}/{r}/commits/{sha}` (`files`,
    /// up to 3000; `base_sha` is the first parent); GitLab
    /// `repository/commits/:sha/diff` with `repository/commits/:sha` for
    /// the first parent (`base_sha` empty when that cannot be read).
    /// `head_sha` is `sha`. The default is for providers without
    /// commit diffs (test doubles).
    fn commit_diff(&self, _r: &ReviewRef, _sha: &str) -> CoreResult<FileDiff> {
        Err(crate::error::CoreError::Provider(
            "commit diffs are not available for this provider".into(),
        ))
    }

    /// The resolve events of the review's threads, as far as the platform
    /// reports them: GitLab the current resolution of each resolved
    /// discussion (`resolved_by`, `resolved_at`). GitHub's GraphQL gives a
    /// thread's resolved state (see [`Thread::resolved`]) but no time it was
    /// resolved, so there is no event to report and the default (nothing)
    /// stands there.
    fn thread_resolutions(
        &self,
        _r: &ReviewRef,
    ) -> CoreResult<Vec<crate::history::ThreadResolution>> {
        Ok(Vec::new())
    }

    // --- M6: polling politeness ---

    /// While the platform has rate-limited this provider: when the limit
    /// lifts. Every call fails fast until then, and the poll loop skips the
    /// provider with a "rate limited until HH:MM" error.
    fn rate_limited_until(&self) -> Option<DateTime<Utc>> {
        None
    }
}

pub(crate) use rate_limit::{is_rate_limited, rate_limited};

/// Ends every "could not process" (HTTP 422) error's message: a message
/// check, as for rate limits, because the error has no variant of its own.
const UNPROCESSABLE: &str = "could not process the request (HTTP 422)";

/// `platform`'s HTTP 422: on a list page, one past what it serves (GitHub's
/// pagination cap).
pub(crate) fn unprocessable(platform: &str) -> CoreError {
    CoreError::Provider(format!("{platform} {UNPROCESSABLE}"))
}

/// Whether `e` says a list page does not exist (a 404, or a 422 past the
/// platform's cap), so a page asked for without a next link ends the list
/// there. Any other failure (a timeout, a 5xx, auth) leaves it unknown
/// whether more came, and fails the list rather than cutting it silently.
pub(crate) fn ends_list(e: &CoreError) -> bool {
    match e {
        CoreError::NotFound => true,
        CoreError::Provider(m) => m.ends_with(UNPROCESSABLE),
        _ => false,
    }
}

/// The result of a provider write.
pub type PostResultOf<T> = Result<T, PostError>;

fn read_only() -> PostError {
    PostError::new(
        PostErrorKind::Provider,
        "this provider cannot post".to_string(),
    )
}

/// A thread its platform has no resolve for (GitHub's PR conversation, a
/// GitLab single comment), or a provider without the write.
pub(crate) fn not_resolvable() -> PostError {
    PostError::new(
        PostErrorKind::InvalidInput,
        "This thread cannot be resolved on its platform.",
    )
}

/// The provider for `spec`, running `gh` / `glab` from `programs`; `None`
/// for a GitLab host that names no instance (hand-edited settings).
pub fn build(spec: &ProviderSpec, programs: (&str, &str)) -> Option<Arc<dyn ReviewProvider>> {
    let (gh, glab) = programs;
    match spec.provider {
        ProviderId::Github => Some(Arc::new(GithubProvider::with_binary(
            gh,
            spec.host.clone(),
            spec.account.clone(),
        ))),
        ProviderId::Gitlab => match GitlabProvider::with_binary(glab, &spec.base_url) {
            Ok(p) => Some(Arc::new(p)),
            Err(_) => {
                tracing::warn!("ignoring a GitLab host that names no instance");
                None
            }
        },
    }
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

/// Extensions whose files are binary even when the platform cannot say so
/// (a binary file has no patch, as does a too-large or a renamed-only one).
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "icns", "webp", "tiff", "psd", "pdf", "zip", "gz",
    "tgz", "bz2", "xz", "7z", "rar", "jar", "war", "class", "dll", "exe", "so", "dylib", "a", "o",
    "bin", "dat", "db", "sqlite", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov", "avi",
    "wav", "flac", "ogg", "webm", "docx", "xlsx", "pptx", "doc", "xls", "ppt", "pyc", "wasm",
];

/// Whether `path` names a file type that is binary.
pub(crate) fn looks_binary(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| BINARY_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Added and removed lines of a unified patch (hunk bodies only; the
/// `---`/`+++` headers some platforms include are skipped).
pub(crate) fn count_patch(patch: &str) -> (u32, u32) {
    let (mut add, mut del) = (0, 0);
    for line in patch.lines() {
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => add += 1,
            Some(b'-') => del += 1,
            _ => {}
        }
    }
    (add, del)
}

/// A stable digest of `parts` (UUID v5, SHA-1 based: the same input gives
/// the same value on every build and platform, unlike `std`'s hasher).
pub(crate) fn digest(parts: &[&str]) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, parts.join("\0").as_bytes())
        .simple()
        .to_string()
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

    #[test]
    fn counts_patches_and_spots_binaries() {
        let patch = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n+more\n ctx";
        assert_eq!(count_patch(patch), (2, 1));
        assert!(looks_binary("assets/Logo.PNG"));
        assert!(!looks_binary("src/main.rs"));
        assert!(!looks_binary("Makefile"));
        assert_eq!(digest(&["a", "b"]), digest(&["a", "b"]));
        assert_ne!(digest(&["a", "b"]), digest(&["ab"]));
    }
}
