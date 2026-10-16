//! `git diff-tree` output as the platforms' file lists.
//!
//! Two runs of the same diff: `--raw -z` for each file's status and paths
//! (NUL-separated, so names with spaces, quotes or non-ASCII need no
//! unquoting), and `-p` for the patches. Both come out of one diff queue in
//! the same order, so the n-th patch section is the n-th raw entry; a count
//! that does not match is refused rather than guessed.

use crate::provider::{ChangedFile, FileDiff, FileStatus};

/// Why a diff could not be read into a [`FileDiff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffError {
    /// A file changed type (file ↔ symlink/submodule): git prints it as a
    /// delete plus an add, the platforms as one entry; the API decides.
    TypeChange,
    /// The output did not have the expected shape.
    Malformed,
}

/// One entry of `--raw -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawEntry {
    pub status: FileStatus,
    pub path: String,
    pub old_path: Option<String>,
    /// The blob at the new side; empty for a removed file.
    pub new_oid: String,
}

/// Parse `git diff-tree -r -M -z --raw <a> <b>`.
pub(crate) fn parse_raw(out: &[u8]) -> Result<Vec<RawEntry>, DiffError> {
    let mut fields = out.split(|b| *b == 0).filter(|f| !f.is_empty());
    let mut entries = Vec::new();
    while let Some(meta) = fields.next() {
        let meta = std::str::from_utf8(meta).map_err(|_| DiffError::Malformed)?;
        // `:<old mode> <new mode> <old oid> <new oid> <status>[score]`
        let parts: Vec<&str> = meta
            .strip_prefix(':')
            .ok_or(DiffError::Malformed)?
            .split(' ')
            .collect();
        let [_, _, _, new_oid, status] = parts[..] else {
            return Err(DiffError::Malformed);
        };
        let mut path = || {
            fields
                .next()
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .ok_or(DiffError::Malformed)
        };
        let removed = new_oid.bytes().all(|b| b == b'0');
        let new_oid = if removed { "" } else { new_oid }.to_string();
        let entry = match status.as_bytes().first() {
            Some(b'A') => RawEntry {
                status: FileStatus::Added,
                path: path()?,
                old_path: None,
                new_oid,
            },
            Some(b'M') => RawEntry {
                status: FileStatus::Modified,
                path: path()?,
                old_path: None,
                new_oid,
            },
            Some(b'D') => RawEntry {
                status: FileStatus::Removed,
                path: path()?,
                old_path: None,
                new_oid,
            },
            Some(b'R') => {
                let old = path()?;
                RawEntry {
                    status: FileStatus::Renamed,
                    path: path()?,
                    old_path: Some(old),
                    new_oid,
                }
            }
            // A copy leaves its source in place: for the walk it is a new
            // file (copy detection is off, so this is defensive).
            Some(b'C') => {
                let _source = path()?;
                RawEntry {
                    status: FileStatus::Added,
                    path: path()?,
                    old_path: None,
                    new_oid,
                }
            }
            Some(b'T') => return Err(DiffError::TypeChange),
            _ => return Err(DiffError::Malformed),
        };
        entries.push(entry);
    }
    Ok(entries)
}

/// The sections of `git diff-tree -p`, one per file, each starting at its
/// `diff --git` line. Inside a hunk every line has a ` `/`+`/`-`/`\`
/// prefix, so a line starting `diff --git ` is always a header.
pub(crate) fn split_patch(out: &str) -> Vec<Vec<&str>> {
    let mut sections: Vec<Vec<&str>> = Vec::new();
    // `split('\n')`, not `lines()`: a CRLF file keeps its `\r`.
    for line in out.split('\n') {
        if line.starts_with("diff --git ") {
            sections.push(Vec::new());
        }
        if let Some(s) = sections.last_mut() {
            s.push(line);
        }
    }
    // The output ends with a newline: drop the empty piece after it.
    if let Some(last) = sections.last_mut() {
        if last.last() == Some(&"") {
            last.pop();
        }
    }
    sections
}

/// The raw entries with their patch sections, as a [`FileDiff`] from
/// `base` to `head`.
pub(crate) fn assemble(
    base: &str,
    head: &str,
    raw: Vec<RawEntry>,
    patch: &str,
) -> Result<FileDiff, DiffError> {
    let sections = split_patch(patch);
    if sections.len() != raw.len() {
        return Err(DiffError::Malformed);
    }
    let files = raw
        .into_iter()
        .zip(sections)
        .map(|(entry, section)| {
            let hunk_start = section.iter().position(|l| l.starts_with("@@"));
            let meta = &section[..hunk_start.unwrap_or(section.len())];
            let binary =
                hunk_start.is_none() && meta.iter().any(|l| l.starts_with("Binary files "));
            let hunks = hunk_start.map(|i| &section[i..]);
            let count =
                |c: char| hunks.map_or(0, |h| h.iter().filter(|l| l.starts_with(c)).count() as u32);
            ChangedFile {
                path: entry.path,
                old_path: entry.old_path,
                status: entry.status,
                additions: count('+'),
                deletions: count('-'),
                patch: hunks.map(|h| h.join("\n")),
                content_sha: entry.new_oid,
                binary,
                too_large: false,
            }
        })
        .collect();
    Ok(FileDiff {
        base_sha: base.to_string(),
        head_sha: head.to_string(),
        files,
        truncated: false,
    })
}
