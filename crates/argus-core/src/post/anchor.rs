//! Placing a comment on the current diff: pure functions over unified diff
//! hunks, so every rule is testable without a platform.
//!
//! A comment targets `(path, line, side)` at some head. Before it is posted
//! it is moved to the current head (when the head moved since) and checked
//! against the current diff: a platform only accepts a line comment on a row
//! of a hunk (GitHub refuses anything else; GitLab would accept expanded
//! context, but Argus keeps to what both show).

use serde::Serialize;

use crate::provider::{ChangedFile, FileDiff, FileStatus, Side};

use super::{NewComment, PlacedComment, PostError, PostErrorKind};

/// How a diff row changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LineKind {
    Added,
    Removed,
    /// Unchanged: on both sides.
    Context,
}

/// One row of a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: LineKind,
    /// The old/new line counters at this row, as GitLab counts them (its
    /// line codes use both): for an added row `old_pos` is the old line it
    /// sits before, for a removed row `new_pos` likewise. Use
    /// [`old_line`](Self::old_line) / [`new_line`](Self::new_line) for the
    /// real line numbers.
    pub old_pos: u32,
    pub new_pos: u32,
}

impl DiffLine {
    /// The line on the old side; `None` for an added row.
    pub fn old_line(&self) -> Option<u32> {
        (self.kind != LineKind::Added).then_some(self.old_pos)
    }

    /// The line on the new side; `None` for a removed row.
    pub fn new_line(&self) -> Option<u32> {
        (self.kind != LineKind::Removed).then_some(self.new_pos)
    }

    pub fn line_on(&self, side: Side) -> Option<u32> {
        match side {
            Side::Old => self.old_line(),
            Side::New => self.new_line(),
        }
    }
}

/// One `@@ -a,b +c,d @@` hunk and its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_count: u32,
    pub rows: Vec<DiffLine>,
}

/// `a,b` or `a` (count 1) after the `-`/`+`.
fn range(s: &str) -> Option<(u32, u32)> {
    let (start, count) = s.split_once(',').unwrap_or((s, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// A hunk header: the empty hunk and the new side's first line.
fn header(line: &str) -> Option<(Hunk, u32)> {
    let mut parts = line.split_whitespace().skip(1);
    let (old_start, old_count) = range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_count) = range(parts.next()?.strip_prefix('+')?)?;
    let hunk = Hunk {
        old_start,
        old_count,
        new_count,
        rows: Vec::new(),
    };
    Some((hunk, new_start))
}

/// The hunks of a unified patch. Rows are read while the header's counts
/// last, so `---`/`+++` file headers, a trailing blank line or text after
/// the hunk are never taken for rows; `\ No newline at end of file` is
/// skipped.
pub(crate) fn parse_hunks(patch: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    // (old counter, new counter, old rows left, new rows left)
    let mut cur: Option<(u32, u32, u32, u32)> = None;
    for line in patch.lines() {
        if line.starts_with("@@") {
            cur = header(line).map(|(h, new_start)| {
                let c = (h.old_start, new_start, h.old_count, h.new_count);
                hunks.push(h);
                c
            });
            continue;
        }
        let Some((old, new, old_left, new_left)) = cur.as_mut() else {
            continue;
        };
        if *old_left == 0 && *new_left == 0 {
            continue;
        }
        let kind = match line.as_bytes().first() {
            Some(b'+') => LineKind::Added,
            Some(b'-') => LineKind::Removed,
            Some(b'\\') => continue,
            // A context row; some tools strip its leading space.
            _ => LineKind::Context,
        };
        let row = DiffLine {
            kind,
            old_pos: *old,
            new_pos: *new,
        };
        match kind {
            LineKind::Added => {
                *new += 1;
                *new_left = new_left.saturating_sub(1);
            }
            LineKind::Removed => {
                *old += 1;
                *old_left = old_left.saturating_sub(1);
            }
            LineKind::Context => {
                *old += 1;
                *new += 1;
                *old_left = old_left.saturating_sub(1);
                *new_left = new_left.saturating_sub(1);
            }
        }
        if let Some(h) = hunks.last_mut() {
            h.rows.push(row);
        }
    }
    hunks
}

/// The lines of one hunk a comment can go on. A range comment must stay
/// inside one hunk (GitHub's rule), hence per hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentableHunk {
    /// Old-side lines (removed and context rows), ascending.
    pub old: Vec<u32>,
    /// New-side lines (added and context rows), ascending.
    pub new: Vec<u32>,
}

/// Every hunk of `patch` with its commentable lines.
pub fn commentable_hunks(patch: &str) -> Vec<CommentableHunk> {
    parse_hunks(patch)
        .into_iter()
        .map(|h| CommentableHunk {
            old: h.rows.iter().filter_map(DiffLine::old_line).collect(),
            new: h.rows.iter().filter_map(DiffLine::new_line).collect(),
        })
        .collect()
}

/// Where a comment lands: its (last) row, and the first row of a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub line: DiffLine,
    pub start: Option<DiffLine>,
}

/// Place `line` (and `start..=line` for a range) on `side` of `patch`.
/// `None` when the line is not on a hunk row of that side, or a range does
/// not lie inside one hunk.
pub fn place(patch: &str, side: Side, line: u32, start: Option<u32>) -> Option<Placement> {
    let hunks = parse_hunks(patch);
    let find = |n: u32| {
        hunks.iter().enumerate().find_map(|(i, h)| {
            h.rows
                .iter()
                .find(|r| r.line_on(side) == Some(n))
                .map(|r| (i, *r))
        })
    };
    let (hunk, end) = find(line)?;
    let start = match start.filter(|s| *s != line) {
        None => None,
        Some(s) if s > line => return None,
        Some(s) => {
            let (h, row) = find(s)?;
            if h != hunk {
                return None;
            }
            Some(row)
        }
    };
    Some(Placement { line: end, start })
}

/// Map a new-side `line` of the file before `patch` (a diff from an older
/// head to the current one) to its line after. `None` when the patch
/// removed or changed that line.
pub fn map_forward(patch: &str, line: u32) -> Option<u32> {
    let mut offset: i64 = 0;
    for h in parse_hunks(patch) {
        if h.old_count > 0 && line >= h.old_start && line < h.old_start + h.old_count {
            let row = h.rows.iter().find(|r| r.old_line() == Some(line))?;
            return (row.kind == LineKind::Context).then_some(row.new_pos);
        }
        // A pure insertion (`-a,0`) goes after old line `a`.
        let end = h.old_start + h.old_count.max(1);
        if line < end {
            break;
        }
        offset += i64::from(h.new_count) - i64::from(h.old_count);
    }
    u32::try_from(i64::from(line) + offset).ok()
}

fn not_in_diff(message: String) -> PostError {
    PostError::new(PostErrorKind::LineNotInDiff, message)
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "old side",
        Side::New => "new side",
    }
}

fn lines_label(path: &str, line: u32, start: Option<u32>) -> String {
    match start {
        Some(s) if s != line => format!("{path}:{s}-{line}"),
        _ => format!("{path}:{line}"),
    }
}

/// The file of `diff` a comment on `path` belongs to: by its path at head,
/// or by its path before a rename.
fn file_for<'a>(diff: &'a FileDiff, path: &str) -> Option<&'a ChangedFile> {
    diff.files.iter().find(|f| f.path == path).or_else(|| {
        diff.files
            .iter()
            .find(|f| f.old_path.as_deref() == Some(path))
    })
}

/// Check what can be checked without a diff: a body, a line, a sane range.
pub(crate) fn validate(c: &NewComment) -> Result<(), PostError> {
    let invalid = |m: &str| PostError::new(PostErrorKind::InvalidInput, m);
    if c.body.trim().is_empty() {
        return Err(invalid("the comment is empty"));
    }
    if c.path.trim().is_empty() {
        return Err(invalid("the comment has no file"));
    }
    if c.line == 0 || c.start_line == Some(0) {
        return Err(invalid("line numbers start at 1"));
    }
    if c.start_line.is_some_and(|s| s > c.line) {
        return Err(invalid("a range must start before it ends"));
    }
    Ok(())
}

/// Move `c` to the current head and place it on `current` (the review's
/// diff at that head).
///
/// `since` is the diff from the head `c` was written at to the current head
/// (`None` when it was written at the current head). Rules:
///
/// - New side, head unchanged: the line as written.
/// - New side, head moved: the line is mapped through `since` (a rename
///   carries the path along). A line that `since` removed or changed, a
///   file it deleted or has no patch for, or a `since` whose left side is
///   not the head written at (the branch was rewritten) fails the comment.
/// - Old side: the old side is the review's base, which new commits do not
///   move, so the line is kept; when `c.base_sha` is given and the diff's
///   left side is no longer it (a rebase), the comment fails instead of
///   landing on another line.
/// - Either way the result must be on a hunk row of that side in `current`,
///   a range inside one hunk, or the comment fails with
///   [`PostErrorKind::LineNotInDiff`].
pub fn resolve(
    c: &NewComment,
    written_at: &str,
    current: &FileDiff,
    since: Option<&FileDiff>,
) -> Result<PlacedComment, PostError> {
    validate(c)?;
    let label = lines_label(&c.path, c.line, c.start_line);
    let mut path = c.path.clone();
    let (mut line, mut start) = (c.line, c.start_line.filter(|s| *s != c.line));
    match (c.side, since) {
        (Side::New, Some(since)) => {
            if since.base_sha != written_at {
                return Err(not_in_diff(format!(
                    "{label} is no longer in the diff: the branch was rewritten since"
                )));
            }
            match file_for(since, &c.path) {
                None if since.truncated => {
                    return Err(not_in_diff(format!(
                        "{label} could not be followed to the current head: too many files changed"
                    )))
                }
                // Untouched since: same path, same lines.
                None => {}
                Some(f) if f.status == FileStatus::Removed => {
                    return Err(not_in_diff(format!(
                        "{label} is no longer in the diff: the file was deleted"
                    )))
                }
                Some(f) => {
                    let Some(patch) = f.patch.as_deref() else {
                        return Err(not_in_diff(format!(
                            "{label} could not be followed to the current head"
                        )));
                    };
                    let moved = |n| {
                        map_forward(patch, n).ok_or_else(|| {
                            not_in_diff(format!(
                                "{label} is no longer in the diff: the line changed since"
                            ))
                        })
                    };
                    line = moved(line)?;
                    start = start.map(moved).transpose()?;
                    path = f.path.clone();
                }
            }
        }
        (Side::Old, _) => {
            if c.base_sha.as_deref().is_some_and(|b| b != current.base_sha) {
                return Err(not_in_diff(format!(
                    "{label} ({}) is no longer in the diff: the base changed since",
                    side_name(Side::Old)
                )));
            }
        }
        (Side::New, None) => {}
    }
    let gone = || {
        not_in_diff(format!(
            "{} ({}) is no longer in the diff",
            lines_label(&path, line, start),
            side_name(c.side)
        ))
    };
    let file = file_for(current, &path).ok_or_else(gone)?;
    let patch = file.patch.as_deref().ok_or_else(|| {
        not_in_diff(format!(
            "{path} has no text diff to comment on (binary or too large)"
        ))
    })?;
    let placement = place(patch, c.side, line, start).ok_or_else(gone)?;
    Ok(PlacedComment {
        path: file.path.clone(),
        old_path: file.old_path.clone().unwrap_or_else(|| file.path.clone()),
        side: c.side,
        line: placement.line,
        start: placement.start,
        head_sha: current.head_sha.clone(),
        base_sha: current.base_sha.clone(),
        body: c.body.clone(),
    })
}
