//! Thread history: for each thread, the code as it was when the comment was
//! made ("then"), the same lines at the review's current head ("now"), a
//! line diff between the two, and one timeline of notes, the commits that
//! touched the lines, and resolve events.
//!
//! - [`walk`] follows the anchored lines forward from the commit the comment
//!   was made on to the head, commit by commit, through the commits' own
//!   diffs (no git needed), behind the [`LineHistorySource`] seam so a local
//!   clone can answer later (M6).
//! - [`patch`] maps a line through the hunks of one unified diff.
//! - [`snapshot`] cuts the ±10-line windows and diffs them.
//! - [`timeline`] merges the events.
//! - [`cache`] keeps the result in SQLite (`snapshots` + `timeline`), keyed
//!   by (thread, head): it is recomputed only when the head moves.
//! - [`api`] is the [`Core`](crate::Core) entry point
//!   ([`thread_history`](crate::Core::thread_history),
//!   [`review_histories`](crate::Core::review_histories)).
//!
//! Outdated and deleted lines are first-class: nothing is hidden, and a
//! mapping that cannot be trusted is [`LineStatus::Unlocated`], never a
//! guessed line.
//!
//! Every type here serialises camelCase; mirror them in `src/ipc/types.ts`.

pub mod api;
pub mod cache;
pub mod patch;
pub mod snapshot;
pub mod timeline;
pub mod walk;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ReviewId, ThreadId};

pub(crate) use api::compute_histories;
pub use walk::{walk_line, LineHistorySource, LineMapping, WalkStart};

/// What [`Core::thread_history`](crate::Core::thread_history) returns.
///
/// ```ts
/// type ThreadHistory = {
///   threadId: string; reviewId: string;
///   headSha: string;            // the head this was computed at
///   outdated: boolean;          // the platform's own flag
///   status: LineStatus;
///   touchedBy: string[];        // commit shas, oldest first
///   thenSnapshot: CodeSnapshot | null;
///   nowSnapshot: CodeSnapshot | null;
///   diffLines: DiffRow[];       // empty unless both snapshots exist
///   timeline: TimelineEntry[];  // oldest first
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistory {
    pub thread_id: ThreadId,
    pub review_id: ReviewId,
    /// The review head the status and the now snapshot are at.
    pub head_sha: String,
    /// The platform's outdated flag, passed through.
    pub outdated: bool,
    pub status: LineStatus,
    /// The commits that changed the anchored lines (their removal
    /// included), oldest first. Empty when nothing touched them, and when
    /// the change could not be attributed to single commits (a rebase).
    pub touched_by: Vec<String>,
    pub then_snapshot: Option<CodeSnapshot>,
    pub now_snapshot: Option<CodeSnapshot>,
    /// The then window against the now window, row by row.
    pub diff_lines: Vec<DiffRow>,
    pub timeline: Vec<TimelineEntry>,
}

/// Where the anchored lines are at head.
///
/// ```ts
/// type LineStatus =
///   | { kind: "located"; path: string; startLine: number; line: number;
///       moved: boolean; renamed: boolean; touched: boolean }
///   | { kind: "deleted"; path: string; removedBy: string | null }
///   | { kind: "fileGone"; path: string; removedBy: string | null }
///   | { kind: "unlocated"; reason: string }
///   | { kind: "notCode" };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LineStatus {
    /// The lines exist at head: `start_line..=line` in `path` (1-based).
    Located {
        path: String,
        start_line: u32,
        line: u32,
        /// The path or the line numbers differ from then.
        moved: bool,
        /// The file was renamed on the way (`path` is the new name).
        renamed: bool,
        /// A commit changed the lines (see `touched_by`).
        touched: bool,
    },
    /// Every anchored line was removed. `path` is the file it was in at
    /// head. `removed_by` is the commit that removed them; `None` when the
    /// comment is on the review's removed side (the review's own diff
    /// removes the line) or the removal has no single commit (a rebase).
    Deleted {
        path: String,
        removed_by: Option<String>,
    },
    /// The file was deleted; `path` is the name it had then.
    FileGone {
        path: String,
        removed_by: Option<String>,
    },
    /// The lines could not be followed reliably. `reason` is Argus-owned
    /// text for the UI.
    Unlocated { reason: String },
    /// A discussion on the review as a whole: no code, no snapshots.
    NotCode,
}

impl LineStatus {
    pub(crate) fn unlocated(reason: impl Into<String>) -> Self {
        LineStatus::Unlocated {
            reason: reason.into(),
        }
    }
}

/// Where a snapshot's lines come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotSource {
    /// The file at the snapshot's commit.
    File,
    /// GitHub's `diff_hunk` of the comment (the blob could not be fetched):
    /// only the lines up to the commented one.
    DiffHunk,
}

/// A window of code around the anchored lines.
///
/// ```ts
/// type CodeSnapshot = {
///   sha: string; path: string;
///   firstLine: number;          // 1-based number of lines[0]
///   lines: string[];
///   anchorStart: number | null; // the anchored range, file line numbers
///   anchorEnd: number | null;   // null: a deleted line's surroundings
///   source: "file" | "diffHunk";
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSnapshot {
    pub sha: String,
    pub path: String,
    pub first_line: u32,
    pub lines: Vec<String>,
    pub anchor_start: Option<u32>,
    pub anchor_end: Option<u32>,
    pub source: SnapshotSource,
}

/// One row of the then/now diff: two columns, either side may be empty.
///
/// ```ts
/// type DiffRow = { kind: "same" | "changed" | "removed" | "added";
///                  then: DiffCell | null; now: DiffCell | null };
/// type DiffCell = { line: number; text: string; anchored: boolean };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRow {
    pub kind: DiffKind,
    pub then: Option<DiffCell>,
    pub now: Option<DiffCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffKind {
    /// The same text on both sides.
    Same,
    /// Both sides, different text.
    Changed,
    /// Only then.
    Removed,
    /// Only now.
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffCell {
    /// The file line number.
    pub line: u32,
    pub text: String,
    /// Inside the anchored range of its side.
    pub anchored: bool,
}

/// One event of a thread's timeline.
///
/// ```ts
/// type TimelineEntry = { at: string | null } & (
///   | { kind: "note"; noteId: string; author: string; body: string;
///       system: boolean; root: boolean }
///   | { kind: "commit"; sha: string; title: string; author: string;
///       change: LineChange }
///   | { kind: "resolved"; by: string }
///   | { kind: "unresolved"; by: string });
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// `None` only for a commit the platform sent no date for (sorted last).
    pub at: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub event: TimelineEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TimelineEvent {
    Note {
        note_id: String,
        author: String,
        body: String,
        system: bool,
        /// The thread's first comment.
        root: bool,
    },
    Commit {
        sha: String,
        title: String,
        author: String,
        change: LineChange,
    },
    Resolved {
        by: String,
    },
    /// No provider reports this yet (GitLab only has the current state,
    /// GitHub's REST API none); the shape is ready for one that does.
    Unresolved {
        by: String,
    },
}

/// What one commit did to the anchored lines.
///
/// ```ts
/// type LineChange = { kind: "modified" } | { kind: "deleted" }
///   | { kind: "fileGone" } | { kind: "renamed"; from: string; to: string };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LineChange {
    Modified,
    Deleted,
    FileGone,
    /// The file was renamed (the lines may be untouched).
    Renamed {
        from: String,
        to: String,
    },
}

/// A resolve event as a provider reports it (see
/// [`ReviewProvider::thread_resolutions`](crate::provider::ReviewProvider::thread_resolutions)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResolution {
    /// The platform's thread id ([`Thread::id`](crate::provider::Thread::id)).
    pub thread_id: String,
    /// `true` for resolved, `false` for unresolved.
    pub resolved: bool,
    pub by: String,
    pub at: DateTime<Utc>,
}

/// What the cached walk says about one thread's lines, for the review's
/// "untouched" count (see [`crate::review::untouched_with`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTouch {
    /// The commit the comment was made on.
    pub then_sha: String,
    /// The review's commits when the walk ran, oldest first.
    pub commits: Vec<String>,
    /// The commits that changed the lines, oldest first.
    pub touched_by: Vec<String>,
    /// Whether the walk can answer at all: it located the lines (or saw
    /// them go) commit by commit.
    pub usable: bool,
}

impl LineTouch {
    /// Whether a commit after `watermark` (or, without one, after the
    /// commit the comment was made on) touched the lines; `None` when the
    /// walk cannot tell (unlocated, rebased, or the watermark is not one of
    /// the review's commits).
    pub fn touched_since(&self, watermark: Option<&str>) -> Option<bool> {
        if !self.usable {
            return None;
        }
        let since = watermark.unwrap_or(&self.then_sha);
        if since == self.then_sha {
            return Some(!self.touched_by.is_empty());
        }
        let pos = |sha: &str| self.commits.iter().position(|c| c == sha);
        let w = pos(since)?;
        Some(
            self.touched_by
                .iter()
                .any(|t| pos(t).is_some_and(|i| i > w)),
        )
    }
}

/// [`LineTouch`] per platform thread id.
pub type LineTouches = std::collections::HashMap<String, LineTouch>;

/// A thread's cached then/now code, flat, for consumers that only want the
/// text (the AI prompt). Read from the cache only, never computed: see
/// [`Core::cached_snapshot`](crate::Core::cached_snapshot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotText {
    /// The then window, lines joined with `\n`.
    pub then_code: Option<String>,
    /// The now window, lines joined with `\n`.
    pub now_code: Option<String>,
    /// Where the lines are at head (for deleted lines: the file they were
    /// in); `None` when gone or unlocated.
    pub now_path: Option<String>,
    /// The commented line at head; `None` unless located.
    pub now_line: Option<u32>,
    pub status: LineStatus,
}

#[cfg(test)]
mod tests;
