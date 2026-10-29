//! Placing a comment on the current diff: pure functions over unified diff
//! hunks, so every rule is testable without a platform.
//!
//! A comment targets `(path, line, side)` at some head. Before it is posted
//! it is moved to the current head (when the head moved since) and placed on
//! the current diff: on a row of a hunk, or, where the provider allows it
//! ([`OutsideDiff`]), on an unchanged line outside every hunk. GitLab takes
//! such a line natively (both `old_line` and `new_line`; it unfolds the
//! diff around the position); GitHub refuses a line comment outside the
//! diff, so there Argus posts a file comment that quotes the lines.

use serde::Serialize;

use crate::provider::{ChangedFile, FileDiff, FileStatus, Side};

use super::{NewComment, OutsideDiff, PlacedComment, PostError, PostErrorKind};

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
    pub new_start: u32,
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
        new_start,
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
    /// On rows of a hunk; `false` for unchanged lines outside the diff
    /// (their rows are [`LineKind::Context`] with both real line numbers).
    pub in_diff: bool,
}

/// Where a line of one side sits relative to a patch's hunks. A comment's
/// range stays inside one region: one hunk (GitHub's rule; GitLab's
/// `line_range` needs both ends in the same diff), or one stretch of
/// unchanged code between two hunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// On a row of hunk `n` (0-based).
    Hunk(usize),
    /// Outside the hunks, after `n` of them (0: above the first).
    Outside(usize),
}

impl Region {
    pub fn is_outside(self) -> bool {
        matches!(self, Region::Outside(_))
    }
}

/// The region of line `n` on `side`. The gap index counts the hunks whose
/// lines on that side all lie above `n`, as the code view counts them
/// (`lib/commentRanges`), so both sides agree on what one range may span.
pub(crate) fn region_of(hunks: &[Hunk], side: Side, n: u32) -> Region {
    if let Some(i) = hunks
        .iter()
        .position(|h| h.rows.iter().any(|r| r.line_on(side) == Some(n)))
    {
        return Region::Hunk(i);
    }
    let above = hunks
        .iter()
        .filter(|h| {
            h.rows
                .iter()
                .filter_map(|r| r.line_on(side))
                .max()
                .is_some_and(|m| m < n)
        })
        .count();
    Region::Outside(above)
}

/// The row of line `n` of `side` when it is unchanged code outside every
/// hunk: its number on the other side is shifted by what the hunks above it
/// added or removed. `None` when `n` is in a hunk, or would map below 1.
pub(crate) fn outside_row(hunks: &[Hunk], side: Side, n: u32) -> Option<DiffLine> {
    if n == 0 {
        return None;
    }
    // new - old, over the hunks that end above `n`.
    let mut delta: i64 = 0;
    for h in hunks {
        let (start, count) = match side {
            Side::Old => (h.old_start, h.old_count),
            Side::New => (h.new_start, h.new_count),
        };
        if count > 0 && n >= start && n < start + count {
            return None;
        }
        // A hunk with no lines on this side (`-a,0`: a pure insertion after
        // line `a`) is above `n` once `n` is past `a`.
        if n < start + count.max(1) {
            break;
        }
        delta += i64::from(h.new_count) - i64::from(h.old_count);
    }
    let other = match side {
        Side::Old => i64::from(n) + delta,
        Side::New => i64::from(n) - delta,
    };
    let other = u32::try_from(other).ok().filter(|o| *o >= 1)?;
    let (old_pos, new_pos) = match side {
        Side::Old => (n, other),
        Side::New => (other, n),
    };
    Some(DiffLine {
        kind: LineKind::Context,
        old_pos,
        new_pos,
    })
}

/// Why a line or range could not be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceError {
    /// Not on a hunk row of that side (and outside lines are not allowed,
    /// or the line maps nowhere).
    NotOnSide,
    /// The range starts after it ends.
    Backwards,
    /// The range's ends lie in different regions.
    Crosses { start: Region, end: Region },
}

/// Place `line` (and `start..=line` for a range) on `side` of `patch`.
/// `None` when the line is not on a hunk row of that side, or a range does
/// not lie inside one hunk.
pub fn place(patch: &str, side: Side, line: u32, start: Option<u32>) -> Option<Placement> {
    place_in(patch, side, line, start, false).ok()
}

/// [`place`], and with `outside` also on unchanged lines outside every
/// hunk; a range must stay inside one [`Region`].
pub fn place_in(
    patch: &str,
    side: Side,
    line: u32,
    start: Option<u32>,
    outside: bool,
) -> Result<Placement, PlaceError> {
    let hunks = parse_hunks(patch);
    let row = |n: u32| -> Result<(Region, DiffLine), PlaceError> {
        match region_of(&hunks, side, n) {
            Region::Hunk(i) => hunks[i]
                .rows
                .iter()
                .find(|r| r.line_on(side) == Some(n))
                .map(|r| (Region::Hunk(i), *r))
                .ok_or(PlaceError::NotOnSide),
            r @ Region::Outside(_) if outside => outside_row(&hunks, side, n)
                .map(|row| (r, row))
                .ok_or(PlaceError::NotOnSide),
            Region::Outside(_) => Err(PlaceError::NotOnSide),
        }
    };
    let (region, end) = row(line)?;
    let start = match start.filter(|s| *s != line) {
        None => None,
        Some(s) if s > line => return Err(PlaceError::Backwards),
        Some(s) => {
            // The region first: a start in another hunk or outside the
            // diff is a crossing, not a missing line.
            let first = region_of(&hunks, side, s);
            if first != region {
                return Err(PlaceError::Crosses {
                    start: first,
                    end: region,
                });
            }
            Some(row(s)?.1)
        }
    };
    Ok(Placement {
        line: end,
        start,
        in_diff: !region.is_outside(),
    })
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
///   left side is no longer it (a rebase), or the left side is only a
///   stand-in ([`FileDiff::base_approximate`]), the comment fails instead
///   of landing on another line.
/// - Either way the result must be on a hunk row of that side in `current`,
///   a range inside one hunk, or the comment fails with
///   [`PostErrorKind::LineNotInDiff`].
pub fn resolve(
    c: &NewComment,
    written_at: &str,
    current: &FileDiff,
    since: Option<&FileDiff>,
) -> Result<PlacedComment, PostError> {
    resolve_in(c, written_at, current, since, OutsideDiff::None)
}

/// [`resolve`], where `outside` says whether the provider takes comments on
/// unchanged lines outside the hunks ([`OutsideDiff::Native`] or
/// [`OutsideDiff::Fallback`]): then such a line (of a modified or renamed
/// file with a text patch; an added or removed file is all hunk) is placed
/// as a [`LineKind::Context`] row with both line numbers and
/// [`PlacedComment::in_diff`] `false`. A range must stay in one
/// [`Region`]: a range across a hunk's edge or two hunks fails with
/// [`PostErrorKind::InvalidInput`] and a message that says so.
pub fn resolve_in(
    c: &NewComment,
    written_at: &str,
    current: &FileDiff,
    since: Option<&FileDiff>,
    outside: OutsideDiff,
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
                    let changed = || {
                        not_in_diff(format!(
                            "{label} is no longer in the diff: the line changed since"
                        ))
                    };
                    // Every line of a range must survive, and stay next to
                    // its neighbour: checking only the ends would carry a
                    // comment over a changed middle or an inserted line.
                    let first = start.unwrap_or(line);
                    let mut mapped = Vec::new();
                    for n in first..=line {
                        let m = map_forward(patch, n).ok_or_else(changed)?;
                        if mapped.last().is_some_and(|prev: &u32| m != prev + 1) {
                            return Err(changed());
                        }
                        mapped.push(m);
                    }
                    line = *mapped.last().expect("the range holds `line`");
                    start = start.map(|_| mapped[0]);
                    path = f.path.clone();
                }
            }
        }
        (Side::Old, _) => {
            if current.base_approximate {
                // A stand-in left side (no merge base from the platform or
                // the clone): the old side the user saw may not line up
                // with the patch, so a removed line could land elsewhere.
                // The code view offers none; this holds for every caller.
                return Err(not_in_diff(
                    "Removed lines can't take comments for this file until Argus knows its merge base. Fetch your local clone and try again.".into(),
                ));
            }
            if c.base_sha.as_deref().is_some_and(|b| b != current.base_sha) {
                // The left side differs either because the lines were read
                // in a diff since the last review (an AI run's scope), whose
                // left side is a commit of the branch, or because the
                // branch was rebased. Neither numbering fits the review's
                // diff, and the two cannot be told apart here.
                return Err(not_in_diff(format!(
                    "{label} ({}) was read against another left side than the review's diff has now. Comments on removed lines from a since-last-review AI run can't be posted; re-run the AI pass on the full diff (after a rebase, comment again on the current diff).",
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
            "{path} has no text diff to comment on: the platform sent no patch for it (it is binary, too large, or only renamed). Leave the remark on the review itself instead."
        ))
    })?;
    let allow_outside = outside != OutsideDiff::None
        && matches!(file.status, FileStatus::Modified | FileStatus::Renamed);
    let placement = match place_in(patch, c.side, line, start, allow_outside) {
        Ok(p) => p,
        Err(PlaceError::Crosses { start: s, end: e }) if allow_outside => {
            return Err(crossing(&lines_label(&path, line, start), s, e));
        }
        Err(_) => return Err(gone()),
    };
    Ok(PlacedComment {
        path: file.path.clone(),
        old_path: file.old_path.clone().unwrap_or_else(|| file.path.clone()),
        side: c.side,
        line: placement.line,
        start: placement.start,
        in_diff: placement.in_diff,
        excerpt: None,
        head_sha: current.head_sha.clone(),
        base_sha: current.base_sha.clone(),
        body: c.body.clone(),
    })
}

/// A range whose ends lie in different regions: why it can't be one
/// comment, in words.
fn crossing(label: &str, start: Region, end: Region) -> PostError {
    let why = if start.is_outside() == end.is_outside() {
        if start.is_outside() {
            "it spans a changed part of the file"
        } else {
            "it spans two parts of the diff"
        }
    } else {
        "it mixes changed lines with unchanged code outside the diff"
    };
    PostError::new(
        PostErrorKind::InvalidInput,
        format!(
            "{label} can't be one comment: {why}. A comment's lines stay inside one diff hunk, or all in one stretch of unchanged code."
        ),
    )
}
