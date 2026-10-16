//! The then/now code windows and the diff between them.

use similar::{capture_diff_slices, Algorithm, DiffTag};

use super::patch::parse_header;
use super::{CodeSnapshot, DiffCell, DiffKind, DiffRow, SnapshotSource};
use crate::provider::Side;

/// Lines of context on each side of the anchored range.
pub const CONTEXT: u32 = 10;

/// The window of `text` around lines `start..=end` (1-based, clamped to the
/// file); `None` when the range is not in the file (past its end), which
/// means the line is not where it was said to be.
pub fn window(sha: &str, path: &str, text: &str, start: u32, end: u32) -> Option<CodeSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let n = u32::try_from(lines.len()).ok()?;
    let start = start.min(end);
    if start == 0 || end > n {
        return None;
    }
    let first = start.saturating_sub(CONTEXT).max(1);
    let last = (end + CONTEXT).min(n);
    Some(CodeSnapshot {
        sha: sha.to_string(),
        path: path.to_string(),
        first_line: first,
        lines: slice(&lines, first, last),
        anchor_start: Some(start),
        anchor_end: Some(end),
        source: SnapshotSource::File,
    })
}

/// The surroundings of deleted lines: the window around the gap after line
/// `near` (0: the top of the file), with no anchored range.
pub fn around(sha: &str, path: &str, text: &str, near: u32) -> Option<CodeSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let n = u32::try_from(lines.len()).ok()?;
    if n == 0 || near > n {
        return None;
    }
    let first = near.saturating_sub(CONTEXT - 1).max(1);
    let last = (near + CONTEXT).min(n);
    Some(CodeSnapshot {
        sha: sha.to_string(),
        path: path.to_string(),
        first_line: first,
        lines: slice(&lines, first, last),
        anchor_start: None,
        anchor_end: None,
        source: SnapshotSource::File,
    })
}

fn slice(lines: &[&str], first: u32, last: u32) -> Vec<String> {
    lines[(first - 1) as usize..last as usize]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

/// The then window from GitHub's `diff_hunk` (when the blob could not be
/// fetched): the hunk's lines on `side`, numbered from its header, cut to
/// the context around `start..=end`. `None` when the hunk does not hold
/// the commented line.
pub fn from_diff_hunk(
    sha: &str,
    path: &str,
    hunk: &str,
    side: Side,
    start: u32,
    end: u32,
) -> Option<CodeSnapshot> {
    let mut body = hunk.lines();
    let (old_start, _, new_start, _) = parse_header(body.next()?)?;
    let mut old_no = old_start;
    let mut new_no = new_start;
    let mut numbered: Vec<(u32, String)> = Vec::new();
    for line in body {
        let (kind, text) = match line.as_bytes().first() {
            Some(b'\\') => continue,
            Some(b'-') => ('-', &line[1..]),
            Some(b'+') => ('+', &line[1..]),
            Some(b' ') => (' ', &line[1..]),
            // A context line whose leading space a platform trimmed.
            _ => (' ', line),
        };
        match (kind, side) {
            ('-', Side::Old) => {
                numbered.push((old_no, text.to_string()));
                old_no += 1;
            }
            ('-', Side::New) => old_no += 1,
            ('+', Side::New) => {
                numbered.push((new_no, text.to_string()));
                new_no += 1;
            }
            ('+', Side::Old) => new_no += 1,
            _ => {
                let no = if side == Side::New { new_no } else { old_no };
                numbered.push((no, text.to_string()));
                old_no += 1;
                new_no += 1;
            }
        }
    }
    let start = start.min(end);
    if !numbered.iter().any(|(n, _)| *n == end) {
        return None;
    }
    let lo = start.saturating_sub(CONTEXT);
    let hi = end + CONTEXT;
    let kept: Vec<(u32, String)> = numbered
        .into_iter()
        .filter(|(n, _)| *n >= lo && *n <= hi)
        .collect();
    Some(CodeSnapshot {
        sha: sha.to_string(),
        path: path.to_string(),
        first_line: kept.first()?.0,
        lines: kept.into_iter().map(|(_, t)| t).collect(),
        anchor_start: Some(start),
        anchor_end: Some(end),
        source: SnapshotSource::DiffHunk,
    })
}

/// The anchored lines' text, when the snapshot holds all of them.
pub fn anchored_text(s: &CodeSnapshot) -> Option<&[String]> {
    let (a, b) = (s.anchor_start?, s.anchor_end?);
    let from = a.checked_sub(s.first_line)? as usize;
    let to = b.checked_sub(s.first_line)? as usize;
    s.lines.get(from..=to)
}

/// The two windows line by line, aligned for a then | now view: equal
/// lines side by side, a replaced run paired up row by row, the rest on one
/// side only.
pub fn diff_rows(then: &CodeSnapshot, now: &CodeSnapshot) -> Vec<DiffRow> {
    let cell = |s: &CodeSnapshot, i: usize| {
        let line = s.first_line + i as u32;
        DiffCell {
            line,
            text: s.lines[i].clone(),
            anchored: s
                .anchor_start
                .zip(s.anchor_end)
                .is_some_and(|(a, b)| line >= a && line <= b),
        }
    };
    let mut rows = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &then.lines, &now.lines) {
        let (tag, old, new) = op.as_tag_tuple();
        match tag {
            DiffTag::Equal => rows.extend(old.zip(new).map(|(o, n)| DiffRow {
                kind: DiffKind::Same,
                then: Some(cell(then, o)),
                now: Some(cell(now, n)),
            })),
            DiffTag::Delete => rows.extend(old.map(|o| DiffRow {
                kind: DiffKind::Removed,
                then: Some(cell(then, o)),
                now: None,
            })),
            DiffTag::Insert => rows.extend(new.map(|n| DiffRow {
                kind: DiffKind::Added,
                then: None,
                now: Some(cell(now, n)),
            })),
            DiffTag::Replace => {
                let (o_len, n_len) = (old.len(), new.len());
                for i in 0..o_len.max(n_len) {
                    let t = (i < o_len).then(|| cell(then, old.start + i));
                    let w = (i < n_len).then(|| cell(now, new.start + i));
                    let kind = match (&t, &w) {
                        (Some(_), Some(_)) => DiffKind::Changed,
                        (Some(_), None) => DiffKind::Removed,
                        _ => DiffKind::Added,
                    };
                    rows.push(DiffRow {
                        kind,
                        then: t,
                        now: w,
                    });
                }
            }
        }
    }
    rows
}
