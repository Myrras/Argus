//! The canonical review model both providers normalise into.
//!
//! Serialised camelCase: the frontend consumes these shapes directly (mirror
//! them in `src/ipc/types.ts`).

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::ReviewId;

/// Which platform a review lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    Github,
    Gitlab,
}

impl ProviderId {
    /// The stable lowercase key used in the store and in natural keys.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Github => "github",
            ProviderId::Gitlab => "gitlab",
        }
    }

    /// Inverse of [`as_str`](Self::as_str).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(ProviderId::Github),
            "gitlab" => Some(ProviderId::Gitlab),
            _ => None,
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ProviderId::Github => "GitHub",
            ProviderId::Gitlab => "GitLab",
        })
    }
}

/// One MR/PR as the list knows it: enough to fetch the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRef {
    pub provider: ProviderId,
    /// `github.com`, or the GitLab/GHE host (no scheme).
    pub host: String,
    /// `owner/repo` on GitHub, the full project path on GitLab.
    pub repo: String,
    /// PR number / MR iid.
    pub number: u64,
    pub url: String,
    pub title: String,
    /// Username of the MR/PR author.
    pub author: String,
    pub updated_at: Option<DateTime<Utc>>,
}

impl ReviewRef {
    /// Natural key: `provider|host|repo|number`.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.provider.as_str(),
            self.host,
            self.repo,
            self.number
        )
    }

    /// The stable id derived from [`key`](Self::key).
    pub fn id(&self) -> ReviewId {
        ReviewId::from_key(&self.key())
    }
}

/// Platform state of the MR/PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    Open,
    Merged,
    Closed,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::Open => "open",
            ReviewState::Merged => "merged",
            ReviewState::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(ReviewState::Open),
            "merged" => Some(ReviewState::Merged),
            "closed" => Some(ReviewState::Closed),
            _ => None,
        }
    }
}

/// Where a reviewer stands on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewerState {
    /// Review requested and not (re)given yet.
    Requested,
    Commented,
    ChangesRequested,
    Approved,
    /// GitHub: the review was dismissed.
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    pub username: String,
    pub state: ReviewerState,
    /// Whether this is the current user.
    pub is_me: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    /// First line of the message.
    pub title: String,
    pub author: String,
    pub authored_at: Option<DateTime<Utc>>,
}

/// One GitLab diff version (a push round). GitHub has no equivalent; empty
/// there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffVersion {
    pub head_sha: String,
    pub base_sha: String,
    pub start_sha: String,
    pub created_at: Option<DateTime<Utc>>,
}

/// Everything about one MR/PR except its threads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDetail {
    /// The ref, refreshed (title, author, url, updated time).
    pub review: ReviewRef,
    pub description: String,
    pub state: ReviewState,
    pub draft: bool,
    pub head_sha: String,
    pub base_sha: String,
    /// GitLab's `start_sha` (the target branch head the diff starts at).
    pub start_sha: Option<String>,
    /// Oldest first.
    pub commits: Vec<CommitInfo>,
    /// GitLab only; oldest first.
    pub versions: Vec<DiffVersion>,
    pub reviewers: Vec<Reviewer>,
    /// The current user's review is requested right now (GitHub: in
    /// `requested_reviewers`; GitLab: reviewer entry `unreviewed`).
    pub requested_from_me: bool,
    /// The current user's approval stands on the platform (and was not
    /// re-requested since).
    pub approved_by_me: bool,
}

/// Which side of the diff a line anchor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// The base (removed / old) side.
    Old,
    /// The head (added / new) side.
    New,
}

/// Where a thread was left, as of its first comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub path_old: Option<String>,
    pub path_new: Option<String>,
    pub line_old: Option<u32>,
    pub line_new: Option<u32>,
    /// First line of a multi-line comment, on [`side`](Self::side).
    pub start_line: Option<u32>,
    pub side: Side,
    pub base_sha: Option<String>,
    pub start_sha: Option<String>,
    /// The head commit the comment was made against.
    pub head_sha: String,
    /// GitHub's hunk excerpt; GitLab does not send one.
    pub diff_hunk: Option<String>,
    /// GitHub: the line in the PR's current diff (`line`), on
    /// [`side`](Self::side); `None` once outdated. `line_old`/`line_new`
    /// hold the original line, which drifts when lines are added above it.
    /// GitLab sends no such field (a current position is as of the head), so
    /// `None` there. Absent in anchors stored before it existed.
    #[serde(default)]
    pub current_line: Option<u32>,
}

/// One comment in a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// The platform's note/comment id.
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    /// A platform-generated note (GitLab system notes).
    pub system: bool,
}

/// A discussion: a root comment plus its replies, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// The platform's thread id (GitHub: root comment id; GitLab: discussion
    /// id).
    pub id: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    /// GitHub: GraphQL's `isResolved` (REST has no such field); `false`
    /// when that query fails.
    pub resolved: bool,
    pub outdated: bool,
    /// `None` for a GitLab discussion on the MR as a whole.
    pub anchor: Option<Anchor>,
    pub notes: Vec<Note>,
}

/// How a file changed in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Added,
    Modified,
    Removed,
    Renamed,
}

/// One file of a diff, as the platform reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// The path at head (for a removed file, the path it had).
    pub path: String,
    /// The path before a rename; `None` unless [`FileStatus::Renamed`].
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// Added / removed lines; 0 when the platform sent no patch.
    pub additions: u32,
    pub deletions: u32,
    /// The unified patch hunks, when the platform sent them (not for binary
    /// or too-large files).
    pub patch: Option<String>,
    /// What the file is at head, to tell whether it changed since it was
    /// marked viewed: GitHub's blob sha. GitLab sends no blob sha in a diff,
    /// so there it is a digest of the file's diff from the review's base
    /// (the same base and diff mean the same content at head).
    pub content_sha: String,
    pub binary: bool,
    /// The platform left the patch out because it is too large.
    pub too_large: bool,
}

/// Which diff [`ReviewProvider::fetch_files`](super::ReviewProvider::fetch_files)
/// reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRange {
    /// The MR/PR's own diff (what the platform's Files/Changes tab shows),
    /// as it is on the platform now. `base` and `head` are the review's
    /// stored shas; the diff may be of a newer head, and
    /// [`FileDiff::head_sha`] says which.
    Review { base: String, head: String },
    /// Two commits of the review, e.g. "since my last review".
    Compare { from: String, to: String },
}

/// The files of one diff and the commits its two sides are read at.
///
/// **Contract:** `base_sha` and `head_sha` are the commits this diff was
/// actually computed at, as the platform reports them, never just the shas
/// the caller passed in. A platform diffs a review as it is now, so for
/// [`DiffRange::Review`] `head_sha` is the platform's current head (GitHub:
/// the PR's `head.sha` read around the files; GitLab: the latest diff
/// version's `head_commit_sha`), which is newer than the stored head when
/// the review moved since the last poll: compare it with the stored head to
/// tell. `base_sha` is the left side the patches are relative to (GitHub:
/// the merge base of the PR's base and head; GitLab: that version's
/// `base_commit_sha`). For [`DiffRange::Compare`] they are what the compare
/// was taken between (GitHub: its merge base and `to`; GitLab: `from` and
/// `to`). For a commit diff (`commit_diff`), `head_sha` is the commit and
/// `base_sha` its first parent, or empty when the platform names none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    /// The left side: the commit the patches are relative to.
    pub base_sha: String,
    /// The right side: the commit the patches lead to.
    pub head_sha: String,
    pub files: Vec<ChangedFile>,
    /// The platform's file limit was hit; more files changed than listed.
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_camel_case() {
        let r = ReviewRef {
            provider: ProviderId::Gitlab,
            host: "gitlab.example.test".into(),
            repo: "grp/proj".into(),
            number: 7,
            url: "u".into(),
            title: "t".into(),
            author: "a".into(),
            updated_at: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["provider"], "gitlab");
        assert!(v.get("updatedAt").is_some());
        assert_eq!(
            serde_json::to_value(ReviewerState::ChangesRequested).unwrap(),
            "changesRequested"
        );
        assert_eq!(r.key(), "gitlab|gitlab.example.test|grp/proj|7");
    }
}
