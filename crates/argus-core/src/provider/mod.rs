//! Review providers: one [`ReviewProvider`] per platform, normalising into
//! the canonical [`model`].
//!
//! Every call is blocking (as in Podium): run them on a poll thread or a
//! `spawn_blocking` task, never on the UI thread. Every call is bounded by a
//! timeout (`gh` is killed, an HTTP request gives up), so a hung call never
//! blocks the poll loop for good.
//!
//! Reads are conditional where the platform allows it (see [`conditional`]):
//! a poll that finds nothing new costs a 304 per call. A rate-limited
//! provider stops calling until the platform's reset (see [`rate_limit`]).

mod conditional;
pub mod github;
pub mod gitlab;
pub mod model;
mod rate_limit;

pub use github::{GhAuth, GithubProvider};
pub use gitlab::{GitlabProvider, KeychainToken, StaticToken, TokenSource};
pub use model::{
    Anchor, ChangedFile, CommitInfo, DiffRange, DiffVersion, FileDiff, FileStatus, Note,
    ProviderId, ReviewDetail, ReviewRef, ReviewState, Reviewer, ReviewerState, Side, Thread,
};

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::CoreResult;
use crate::post::{PlacedComment, PostError, PostErrorKind, PostedComment, PostedNote};
use crate::settings::Settings;

/// Read access to one platform for the current user, plus the explicit
/// writes of M4 (posting, replying, approving).
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

    /// The changed files of `range`, with the commit each side is read at.
    ///
    /// [`DiffRange::Review`]: GitHub `pulls/{n}/files` (up to 3000 files;
    /// large patches are left out) with the merge base of base and head from
    /// the compare API as the left side, as GitHub's own Files tab diffs
    /// three-dot. GitLab `merge_requests/:iid/diffs` (`changes` on instances
    /// before 15.7) with the latest version's `base_sha` (the merge base
    /// GitLab diffs from) as the left side.
    ///
    /// [`DiffRange::Compare`]: GitHub `compare/{from}...{to}` (up to 300
    /// files; left side its merge base, which is `from` unless the branch was
    /// rebased); GitLab `repository/compare?straight=true` (left side `from`).
    fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff>;

    // --- M4: writes. Only `Core`'s post API calls these, each on an explicit
    // user action; nothing else in Argus writes to a platform. The defaults
    // refuse, so a read-only provider (or a test double) needs none of them.

    /// Post one new line comment, already placed on the current diff (see
    /// [`post::resolve`](crate::post::resolve)).
    ///
    /// GitHub: `POST repos/{o}/{r}/pulls/{n}/comments` with `commit_id`,
    /// `path`, `line`, `side` (+ `start_line`/`start_side`). GitLab: `POST
    /// merge_requests/:iid/discussions` with a text position from the latest
    /// diff version, read right before posting.
    fn post_comment(&self, _r: &ReviewRef, _c: &PlacedComment) -> PostResultOf<PostedComment> {
        Err(read_only())
    }

    /// Reply to the platform thread `thread` (GitHub: the root comment id;
    /// GitLab: the discussion id).
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

    // --- M3: thread history ---

    /// What commit `sha` changed, against its first parent (the thread
    /// history's line walk). GitHub `repos/{o}/{r}/commits/{sha}` (`files`,
    /// up to 3000; `base_sha` is the first parent); GitLab
    /// `repository/commits/:sha/diff` (`base_sha` empty: it sends no
    /// parent). `head_sha` is `sha`. The default is for providers without
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

/// The result of a provider write.
pub type PostResultOf<T> = Result<T, PostError>;

fn read_only() -> PostError {
    PostError::new(
        PostErrorKind::Provider,
        "this provider cannot post".to_string(),
    )
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

/// Whether each provider can be used right now, for the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub github: GithubStatus,
    pub gitlab: GitlabStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubStatus {
    /// Turned on in settings; when off, the other flags are `false`.
    pub enabled: bool,
    pub host: String,
    /// The `gh` CLI could be started.
    pub installed: bool,
    /// `gh` has a token for the host (and the configured account).
    pub logged_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitlabStatus {
    /// A base URL is set.
    pub configured: bool,
    pub base_url: String,
    /// A token for that instance is in the keychain (never the token itself).
    pub token_present: bool,
}

/// The [`ProviderStatus`] for `settings`. Blocking: runs `gh auth token`
/// (local, no network) and reads the keychain.
pub fn status(settings: &Settings) -> ProviderStatus {
    let gh = &settings.github;
    let auth = if gh.enabled {
        GithubProvider::new(gh.host.clone(), gh.account.clone()).auth_status()
    } else {
        GhAuth {
            installed: false,
            logged_in: false,
        }
    };
    let url = settings.gitlab.base_url.trim().to_string();
    ProviderStatus {
        github: GithubStatus {
            enabled: gh.enabled,
            host: gh.host.clone(),
            installed: auth.installed,
            logged_in: auth.logged_in,
        },
        gitlab: GitlabStatus {
            configured: !url.is_empty(),
            token_present: !url.is_empty() && gitlab::gitlab_token_present(&url),
            base_url: url,
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
