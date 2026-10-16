//! How a line moves through one unified diff.
//!
//! A patch is read as a list of change [`Block`]s (a run of removed and/or
//! added lines between context lines). A line above a block keeps its
//! number, a line below shifts by what the block added minus removed, and a
//! line inside a block's removed span was changed (the block added lines
//! too) or deleted (it added none).

/// One change: old lines `old..old + removed` became new lines
/// `new..new + added` (1-based). A pure insertion (`removed == 0`) goes
/// before old line `old`; after a pure deletion (`added == 0`) the next line
/// is new line `new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub old: u32,
    pub removed: u32,
    pub new: u32,
    pub added: u32,
}

/// What a diff did to one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// Unchanged, now at this line.
    Kept(u32),
    /// Rewritten; its replacement is at this line. Within a block the i-th
    /// removed line pairs with the i-th added one (the last added line when
    /// fewer were added), which is how a reader sees an edit.
    Changed(u32),
    /// Removed with nothing in its place; `near` is the new line just above
    /// where it was (0 at the top of the file).
    Removed { near: u32 },
}

/// The change blocks of `patch`, in file order; `None` when it is not a
/// well-formed unified diff. Headers (`diff --git`, `---`/`+++`, `index`)
/// before a hunk and `\ No newline at end of file` markers are skipped.
pub fn parse_blocks(patch: &str) -> Option<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut lines = patch.lines();
    while let Some(line) = lines.next() {
        if !line.starts_with("@@") {
            continue;
        }
        let (old_start, old_len, new_start, new_len) = parse_header(line)?;
        // With a zero length the start names the line *before* the hunk.
        let mut old_no = if old_len == 0 {
            old_start + 1
        } else {
            old_start
        };
        let mut new_no = if new_len == 0 {
            new_start + 1
        } else {
            new_start
        };
        let (mut old_left, mut new_left) = (old_len, new_len);
        let mut current: Option<Block> = None;
        while old_left > 0 || new_left > 0 {
            let body = lines.next()?;
            let start = Block {
                old: old_no,
                removed: 0,
                new: new_no,
                added: 0,
            };
            match body.as_bytes().first() {
                Some(b'\\') => {}
                Some(b'-') => {
                    old_left = old_left.checked_sub(1)?;
                    current.get_or_insert(start).removed += 1;
                    old_no += 1;
                }
                Some(b'+') => {
                    new_left = new_left.checked_sub(1)?;
                    current.get_or_insert(start).added += 1;
                    new_no += 1;
                }
                // Context; an empty line is context whose space a platform
                // trimmed.
                _ => {
                    old_left = old_left.checked_sub(1)?;
                    new_left = new_left.checked_sub(1)?;
                    blocks.extend(current.take());
                    old_no += 1;
                    new_no += 1;
                }
            }
        }
        blocks.extend(current.take());
    }
    Some(blocks)
}

/// `@@ -a[,b] +c[,d] @@ ...` → (a, b, c, d); a missing length is 1.
pub(crate) fn parse_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let inner = line.strip_prefix("@@ ")?;
    let (ranges, _) = inner.split_once(" @@")?;
    let (old, new) = ranges.split_once(' ')?;
    let range = |s: &str, sign: char| -> Option<(u32, u32)> {
        let s = s.strip_prefix(sign)?;
        match s.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (a, b) = range(old, '-')?;
    let (c, d) = range(new, '+')?;
    Some((a, b, c, d))
}

/// Where old line `line` ends up after `blocks`.
pub fn map_line(blocks: &[Block], line: u32) -> Fate {
    let mut delta: i64 = 0;
    for b in blocks {
        if line < b.old {
            break;
        }
        if line >= b.old + b.removed {
            delta += i64::from(b.added) - i64::from(b.removed);
            continue;
        }
        let i = line - b.old;
        return if b.added > 0 {
            Fate::Changed(b.new + i.min(b.added - 1))
        } else {
            Fate::Removed {
                near: b.new.saturating_sub(1),
            }
        };
    }
    Fate::Kept(u32::try_from(i64::from(line) + delta).unwrap_or(0))
}

/// Whether `blocks` change the region `lo..=hi`: a block removes a line in
/// it, or inserts lines between two of its lines. Lines added right above or
/// below the region do not touch it.
pub fn touches(blocks: &[Block], lo: u32, hi: u32) -> bool {
    blocks.iter().any(|b| {
        if b.removed > 0 {
            b.old <= hi && b.old + b.removed > lo
        } else {
            lo < b.old && b.old <= hi
        }
    })
}
