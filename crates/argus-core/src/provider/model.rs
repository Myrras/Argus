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
    /// GitHub's REST API does not expose this; always `false` there for now.
    pub resolved: bool,
    pub outdated: bool,
    /// `None` for a GitLab discussion on the MR as a whole.
    pub anchor: Option<Anchor>,
    pub notes: Vec<Note>,
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
