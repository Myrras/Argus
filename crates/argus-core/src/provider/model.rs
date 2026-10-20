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

/// Where the current user's own review stands on the platform: what
/// decides whether a review is waiting on them (see `review::classify`).
///
/// GitLab: the user's `/reviewers` entry (`unreviewed`, `review_started`,
/// `requested_changes`, `reviewed`/`unapproved`, `approved`). GitHub: in
/// `requested_reviewers` means awaiting (GitHub drops a reviewer from it on
/// any submitted review, and puts them back on a re-request); otherwise
/// the user's latest standing review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MyReviewState {
    /// Requested and not given yet ("Awaiting review" on GitLab), also
    /// after a re-request.
    AwaitingReview,
    /// GitLab: the user has a review in progress (pending comments).
    ReviewStarted,
    RequestedChanges,
    /// Reviewed without a verdict (GitHub "commented" or a dismissed
    /// review; GitLab "reviewed" or an approval taken back).
    Commented,
    Approved,
    /// Not a reviewer of it (Argus lists it because the user commented).
    #[default]
    NotReviewer,
}

impl MyReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            MyReviewState::AwaitingReview => "awaitingReview",
            MyReviewState::ReviewStarted => "reviewStarted",
            MyReviewState::RequestedChanges => "requestedChanges",
            MyReviewState::Commented => "commented",
            MyReviewState::Approved => "approved",
            MyReviewState::NotReviewer => "notReviewer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "awaitingReview" => Some(MyReviewState::AwaitingReview),
            "reviewStarted" => Some(MyReviewState::ReviewStarted),
            "requestedChanges" => Some(MyReviewState::RequestedChanges),
            "commented" => Some(MyReviewState::Commented),
            "approved" => Some(MyReviewState::Approved),
            "notReviewer" => Some(MyReviewState::NotReviewer),
            _ => None,
        }
    }

    /// The platform says the user's review is owed (again).
    pub fn owes_review(self) -> bool {
        matches!(
            self,
            MyReviewState::AwaitingReview | MyReviewState::ReviewStarted
        )
    }

    /// The user submitted a verdict that leaves the next move to the
    /// author.
    pub fn submitted_without_approval(self) -> bool {
        matches!(
            self,
            MyReviewState::RequestedChanges | MyReviewState::Commented
        )
    }
}

/// What "Submit review" sets on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewVerdict {
    Approve,
    RequestChanges,
    /// GitHub only: a review without a verdict (its summary required).
    Comment,
}

impl ReviewVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewVerdict::Approve => "approve",
            ReviewVerdict::RequestChanges => "requestChanges",
            ReviewVerdict::Comment => "comment",
        }
    }
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
    /// The user's own review state on the platform.
    #[serde(default)]
    pub my_review_state: MyReviewState,
    /// When the user last submitted a review on the platform (an approval,
    /// a change request, a comment review), and the head it was given at:
    /// a review given on GitHub or GitLab itself counts as reviewed, like
    /// one from Argus. GitHub: the latest submitted review's `submitted_at`
    /// and `commit_id`. GitLab: the latest "approved this merge request" /
    /// "requested changes" system note of the user's (for a `reviewed`
    /// state without one, their latest comment), at the diff version that
    /// was current then.
    #[serde(default)]
    pub my_reviewed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub my_reviewed_sha: Option<String>,
    /// When the user's review was last requested, as the platform records
    /// it (GitLab: the newest "requested review from @me" system note).
    /// `None` when the platform does not say (GitHub).
    #[serde(default)]
    pub review_requested_at: Option<DateTime<Utc>>,
}

/// A review's head, base and commits: what a head check and the history
/// walk need, without the reviewer and approval state a [`ReviewDetail`]
/// also reads (and fails on). See
/// [`ReviewProvider::fetch_head`](super::ReviewProvider::fetch_head).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewHead {
    pub head_sha: String,
    pub base_sha: String,
    /// Oldest first.
    pub commits: Vec<CommitInfo>,
}

impl From<ReviewDetail> for ReviewHead {
    fn from(d: ReviewDetail) -> Self {
        Self {
            head_sha: d.head_sha,
            base_sha: d.base_sha,
            commits: d.commits,
        }
    }
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
    /// The platform's thread id (GitHub: root comment id, or for the PR
    /// conversation `issuecomment-<id>` / `pullrequestreview-<id>` of its
    /// first entry; GitLab: discussion id).
    pub id: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    /// GitHub: GraphQL's `isResolved` (REST has no such field); `false`
    /// when that query fails.
    pub resolved: bool,
    pub outdated: bool,
    /// `None` for a discussion on the review as a whole (a GitLab MR-level
    /// discussion, GitHub's PR conversation).
    pub anchor: Option<Anchor>,
    pub notes: Vec<Note>,
}

/// Usernames compare without case on both platforms.
pub(crate) fn same_user(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Thread {
    /// Started by `me`.
    pub fn started_by(&self, me: &str) -> bool {
        same_user(&self.author, me)
    }

    /// The user's thread: started by `me` or with a comment from `me` in it.
    /// A system note (a GitLab "resolved", "changed this line") is the
    /// platform's, not a comment, so it does not make a thread anyone's.
    /// The one rule for "mine": stored as [`StoredThread::mine`], it orders
    /// and opens threads in the UI, scopes the round reasons and the
    /// untouched count, and picks the threads the AI pass judges.
    ///
    /// [`StoredThread::mine`]: crate::store::StoredThread::mine
    pub fn is_mine(&self, me: &str) -> bool {
        self.started_by(me)
            || self
                .notes
                .iter()
                .any(|n| !n.system && same_user(&n.author, me))
    }
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
    /// `base_sha` is a stand-in, not the commit the patches are relative
    /// to: GitHub could not compute a large PR's merge base, and the PR's
    /// base branch commit took its place (see
    /// [`ReviewProvider::fetch_files`](super::ReviewProvider::fetch_files)).
    /// The old side read at it may not line up with the patches.
    #[serde(default)]
    pub base_approximate: bool,
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

    fn note(author: &str, system: bool) -> Note {
        Note {
            id: "n".into(),
            author: author.into(),
            body: "b".into(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            system,
        }
    }

    fn thread(author: &str, notes: Vec<Note>) -> Thread {
        Thread {
            id: "t".into(),
            author: author.into(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            resolved: false,
            outdated: false,
            anchor: None,
            notes,
        }
    }

    #[test]
    fn a_thread_is_mine_when_i_started_it_or_commented_in_it() {
        let me = "rev-fixture";
        let started = thread(me, vec![note(me, false)]);
        assert!(started.is_mine(me) && started.started_by(me));
        // Case does not matter on either platform.
        assert!(started.is_mine("Rev-Fixture"));

        let replied = thread(
            "dana-fixture",
            vec![note("dana-fixture", false), note(me, false)],
        );
        assert!(replied.is_mine(me));
        assert!(!replied.started_by(me));

        // Only a system note of mine (a resolve): still someone else's.
        let resolved = thread(
            "dana-fixture",
            vec![note("dana-fixture", false), note(me, true)],
        );
        assert!(!resolved.is_mine(me));

        let theirs = thread(
            "dana-fixture",
            vec![note("dana-fixture", false), note("alice-fixture", false)],
        );
        assert!(!theirs.is_mine(me));
    }
}
