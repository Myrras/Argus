//! Following anchored lines from the commit a comment was made on to the
//! review's head, without assuming a local git: through each commit's own
//! diff, restricted to the thread's file and following renames.
//!
//! Where the diffs come from is behind [`LineHistorySource`]: the platform
//! API now ([`ApiSource`]), a local clone (`git log -L` / `--follow`) in M6.

use std::collections::HashMap;
use std::sync::Mutex;

use super::patch::{map_line, parse_blocks, touches, Fate};
use super::{LineChange, LineStatus};
use crate::error::CoreResult;
use crate::provider::{
    ChangedFile, CommitInfo, DiffRange, FileDiff, FileStatus, ReviewProvider, ReviewRef,
};

/// Where the per-commit diffs of a walk come from.
pub trait LineHistorySource: Sync {
    /// The commits after `from` up to and including `to`, oldest first;
    /// `None` when `from` is not on the line of commits that leads to `to`
    /// (rebased or force-pushed away), so there is no per-commit walk.
    fn commits_between(&self, from: &str, to: &str) -> CoreResult<Option<Vec<String>>>;

    /// What one commit changed, against its first parent.
    /// [`FileDiff::base_sha`] is that parent when known (empty otherwise):
    /// a parent that is not the previous commit of the walk, or none named,
    /// means the commits are not proven to form one line (a merged side
    /// branch) and the walk falls back to [`direct_diff`](Self::direct_diff).
    fn commit_diff(&self, sha: &str) -> CoreResult<FileDiff>;

    /// The plain diff from `from` to `to`, or `None` when the source cannot
    /// give one taken exactly from `from`.
    fn direct_diff(&self, from: &str, to: &str) -> CoreResult<Option<FileDiff>>;
}

/// The anchored lines at the commit the comment was made on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStart {
    pub sha: String,
    pub path: String,
    /// First line of the range (1-based); equal to `line` for one line.
    pub start_line: u32,
    /// The commented line (the range's last).
    pub line: u32,
}

/// One thing a step of the walk did to the lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEvent {
    /// The commit; `None` for a direct diff (no single commit to name).
    pub sha: Option<String>,
    pub change: LineChange,
}

/// The outcome of [`walk_line`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMapping {
    pub status: LineStatus,
    /// What happened on the way, oldest first.
    pub events: Vec<LineEvent>,
    /// The commits that changed the lines (removal included), oldest first.
    pub touched_by: Vec<String>,
    /// Followed commit by commit, so `touched_by` is complete.
    pub attributed: bool,
    /// For [`LineStatus::Deleted`]: the head line just above where the lines
    /// were (0: the top), to show their surroundings; `None` when lost.
    pub near: Option<u32>,
}

impl LineMapping {
    fn unlocated(reason: &str) -> Self {
        Self {
            status: LineStatus::unlocated(reason),
            events: Vec::new(),
            touched_by: Vec::new(),
            attributed: false,
            near: None,
        }
    }
}

/// Follow `start` to `head`. Data the walk cannot follow (a too-large or
/// binary diff, a truncated file list, a rewritten history with no direct
/// diff) gives [`LineStatus::Unlocated`]; an error is a source failure
/// (network, auth) worth retrying.
pub fn walk_line(
    source: &dyn LineHistorySource,
    start: &WalkStart,
    head: &str,
) -> CoreResult<LineMapping> {
    if start.sha == head {
        return Ok(Tracker::new(start).finish(start, true));
    }
    if let Some(commits) = source.commits_between(&start.sha, head)? {
        let mut t = Tracker::new(start);
        let mut prev = start.sha.as_str();
        let mut linear = true;
        for sha in &commits {
            if t.state == State::FileGone {
                break;
            }
            let diff = source.commit_diff(sha)?;
            // Only a named parent that is the previous commit proves one
            // line of commits. A merge of a side branch repeats the
            // branch's changes in its own diff, so taking an unnamed parent
            // for linear would count them twice.
            if diff.base_sha != prev {
                linear = false;
                break;
            }
            if let Err(reason) = t.apply(Some(sha), &diff) {
                return Ok(LineMapping::unlocated(reason));
            }
            prev = sha;
        }
        if linear {
            return Ok(t.finish(start, true));
        }
    }
    // No single line of commits: one diff from then to head still places
    // the lines, it just cannot say which commit moved them.
    match source.direct_diff(&start.sha, head)? {
        Some(diff) => {
            let mut t = Tracker::new(start);
            match t.apply(None, &diff) {
                Ok(()) => Ok(t.finish(start, false)),
                Err(reason) => Ok(LineMapping::unlocated(reason)),
            }
        }
        None => Ok(LineMapping::unlocated(
            "the commit the comment was made on is no longer part of the review \
             (rebased or force-pushed)",
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Alive,
    Deleted,
    FileGone,
}

/// The lines as the walk carries them.
struct Tracker {
    path: String,
    /// The current numbers of the anchored lines still there, ascending.
    lines: Vec<u32>,
    state: State,
    /// Deleted: the line just above where they were; `None` once lost.
    ghost: Option<u32>,
    renamed: bool,
    touched: bool,
    events: Vec<LineEvent>,
    touched_by: Vec<String>,
}

impl Tracker {
    fn new(start: &WalkStart) -> Self {
        let first = start.start_line.min(start.line).max(1);
        Self {
            path: start.path.clone(),
            lines: (first..=start.line.max(first)).collect(),
            state: State::Alive,
            ghost: None,
            renamed: false,
            touched: false,
            events: Vec::new(),
            touched_by: Vec::new(),
        }
    }

    fn record(&mut self, sha: Option<&str>, change: LineChange, touches: bool) {
        if touches {
            self.touched = true;
            if let Some(sha) = sha {
                self.touched_by.push(sha.to_string());
            }
        }
        self.events.push(LineEvent {
            sha: sha.map(str::to_string),
            change,
        });
    }

    /// Apply one diff; `Err` is why the lines cannot be followed through it.
    fn apply(&mut self, sha: Option<&str>, diff: &FileDiff) -> Result<(), &'static str> {
        if self.state == State::FileGone {
            return Ok(());
        }
        let Some(file) = find_file(&diff.files, &self.path) else {
            if diff.truncated {
                return Err("a commit changed more files than the platform lists");
            }
            return Ok(());
        };
        match file.status {
            FileStatus::Removed => {
                let alive = self.state == State::Alive;
                self.state = State::FileGone;
                self.ghost = None;
                self.record(sha, LineChange::FileGone, alive);
                return Ok(());
            }
            FileStatus::Renamed if file.path != self.path => {
                let from = std::mem::replace(&mut self.path, file.path.clone());
                self.renamed = true;
                self.record(
                    sha,
                    LineChange::Renamed {
                        from,
                        to: file.path.clone(),
                    },
                    false,
                );
            }
            _ => {}
        }
        let Some(patch) = file.patch.as_deref() else {
            if file.binary || file.too_large {
                if self.state == State::Deleted {
                    self.ghost = None;
                    return Ok(());
                }
                return Err(if file.binary {
                    "the file became binary"
                } else {
                    "a commit's diff of the file is too large to follow"
                });
            }
            // A pure rename or a mode change.
            return Ok(());
        };
        let blocks = parse_blocks(patch).ok_or("a commit's diff of the file could not be read")?;
        match self.state {
            State::Alive => {
                let (lo, hi) = (self.lines[0], self.lines[self.lines.len() - 1]);
                let touched = touches(&blocks, lo, hi);
                let mut next = Vec::with_capacity(self.lines.len());
                let mut near: Option<u32> = None;
                for &l in &self.lines {
                    match map_line(&blocks, l) {
                        Fate::Kept(n) | Fate::Changed(n) => next.push(n),
                        Fate::Removed { near: n } => near = Some(near.map_or(n, |m| m.min(n))),
                    }
                }
                next.sort_unstable();
                next.dedup();
                if next.is_empty() {
                    self.state = State::Deleted;
                    self.ghost = near;
                    self.record(sha, LineChange::Deleted, true);
                } else {
                    self.lines = next;
                    if touched {
                        self.record(sha, LineChange::Modified, true);
                    }
                }
            }
            State::Deleted => {
                self.ghost = self.ghost.map(|g| match g {
                    0 => 0,
                    g => match map_line(&blocks, g) {
                        Fate::Kept(n) | Fate::Changed(n) => n,
                        Fate::Removed { near } => near,
                    },
                });
            }
            State::FileGone => {}
        }
        Ok(())
    }

    fn finish(self, start: &WalkStart, attributed: bool) -> LineMapping {
        let removed_by = |change: fn(&LineChange) -> bool| {
            self.events
                .iter()
                .rev()
                .find(|e| change(&e.change))
                .and_then(|e| e.sha.clone())
        };
        let deleted_first = self.events.iter().any(|e| e.change == LineChange::Deleted);
        let (status, near) = match self.state {
            // The lines went before the file did: that is what happened to
            // them.
            State::FileGone if deleted_first => (
                LineStatus::Deleted {
                    path: self.path.clone(),
                    removed_by: removed_by(|c| *c == LineChange::Deleted),
                },
                None,
            ),
            State::Alive => {
                let (first, last) = (self.lines[0], self.lines[self.lines.len() - 1]);
                let first_then = start.start_line.min(start.line).max(1);
                (
                    LineStatus::Located {
                        moved: self.path != start.path || first != first_then || last != start.line,
                        path: self.path.clone(),
                        start_line: first,
                        line: last,
                        renamed: self.renamed,
                        touched: self.touched,
                    },
                    None,
                )
            }
            State::Deleted => (
                LineStatus::Deleted {
                    path: self.path.clone(),
                    removed_by: removed_by(|c| *c == LineChange::Deleted),
                },
                self.ghost,
            ),
            State::FileGone => (
                LineStatus::FileGone {
                    path: self.path.clone(),
                    removed_by: removed_by(|c| *c == LineChange::FileGone),
                },
                None,
            ),
        };
        LineMapping {
            status,
            events: self.events,
            touched_by: self.touched_by,
            attributed,
            near,
        }
    }
}

/// The diff entry of the file currently at `path`: under its old name for a
/// rename. An "added" entry is never the tracked file (it existed already).
fn find_file<'a>(files: &'a [ChangedFile], path: &str) -> Option<&'a ChangedFile> {
    files
        .iter()
        .find(|f| f.status != FileStatus::Added && f.old_path.as_deref().unwrap_or(&f.path) == path)
}

/// [`LineHistorySource`] over a review's platform API: the review's own
/// commit list for the order, each commit's diff from the provider
/// (memoised, so the threads of one review share them), and the compare
/// API for a direct diff.
pub(crate) struct ApiSource<'a> {
    provider: &'a dyn ReviewProvider,
    review: &'a ReviewRef,
    commits: &'a [CommitInfo],
    memo: Mutex<HashMap<String, FileDiff>>,
}

impl<'a> ApiSource<'a> {
    pub(crate) fn new(
        provider: &'a dyn ReviewProvider,
        review: &'a ReviewRef,
        commits: &'a [CommitInfo],
    ) -> Self {
        Self {
            provider,
            review,
            commits,
            memo: Mutex::new(HashMap::new()),
        }
    }
}

impl LineHistorySource for ApiSource<'_> {
    fn commits_between(&self, from: &str, to: &str) -> CoreResult<Option<Vec<String>>> {
        let pos = |sha: &str| self.commits.iter().position(|c| c.sha == sha);
        Ok(match (pos(from), pos(to)) {
            (Some(f), Some(t)) if f < t => Some(
                self.commits[f + 1..=t]
                    .iter()
                    .map(|c| c.sha.clone())
                    .collect(),
            ),
            _ => None,
        })
    }

    fn commit_diff(&self, sha: &str) -> CoreResult<FileDiff> {
        if let Some(d) = self.memo.lock().expect("memo lock poisoned").get(sha) {
            return Ok(d.clone());
        }
        // Not held across the fetch: threads walking other commits go on.
        let diff = self.provider.commit_diff(self.review, sha)?;
        self.memo
            .lock()
            .expect("memo lock poisoned")
            .insert(sha.to_string(), diff.clone());
        Ok(diff)
    }

    fn direct_diff(&self, from: &str, to: &str) -> CoreResult<Option<FileDiff>> {
        let diff = self.provider.fetch_files(
            self.review,
            &DiffRange::Compare {
                from: from.to_string(),
                to: to.to_string(),
            },
        )?;
        // GitHub compares three-dot: a merge base other than `from` means
        // `from` is not an ancestor and the patches are not from it.
        Ok((diff.base_sha == from).then_some(diff))
    }
}
