//! The code view's file list: a review's changed files with the viewed marks
//! applied.
//!
//! Viewed marks are keyed by path + the file's content at head
//! ([`ChangedFile::content_sha`]) and always taken from the review's own diff,
//! also in "since my last review" mode, so a file marked in one mode shows
//! marked in the other, and a file that changed at head since shows unviewed.

use std::collections::HashMap;

use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::ids::ReviewId;
use crate::provider::{ChangedFile, DiffRange, FileDiff, ReviewProvider};
use crate::store::{viewed_paths, ReviewRecord};

/// One file of [`ReviewFiles`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    #[serde(flatten)]
    pub file: ChangedFile,
    /// Marked viewed, and unchanged at head since.
    pub viewed: bool,
}

/// What `list_files` returns: the files of the diff shown, and the commits
/// its sides are read at (`file_contents` takes these shas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFiles {
    pub review_id: ReviewId,
    /// The left side of every file (see [`ReviewProvider::fetch_files`]).
    pub base_sha: String,
    pub head_sha: String,
    /// The diff is `last_reviewed_sha..head`, not the whole review.
    pub since_last_review: bool,
    /// The platform's file limit was hit.
    pub truncated: bool,
    /// `base_sha` is a stand-in for the left side, not the commit the
    /// patches are relative to (see [`FileDiff::base_approximate`]): the
    /// old side may not line up, so it takes no comments.
    pub base_approximate: bool,
    pub files: Vec<FileEntry>,
}

/// Path → content sha at head, from the review's own diff.
pub(crate) type ContentKeys = HashMap<String, String>;

pub(crate) fn content_keys(diff: &FileDiff) -> ContentKeys {
    diff.files
        .iter()
        .map(|f| (f.path.clone(), f.content_sha.clone()))
        .collect()
}

/// The range a review's own diff is read with.
pub(crate) fn review_range(record: &ReviewRecord) -> DiffRange {
    DiffRange::Review {
        base: record.base_sha.clone(),
        head: record.head_sha.clone(),
    }
}

/// The "since my last review" range, or why there is none: it needs a
/// watermark that is not the current head.
pub(crate) fn since_range(record: &ReviewRecord) -> CoreResult<DiffRange> {
    match record.last_reviewed_sha.as_deref() {
        Some(last) if !last.is_empty() && last != record.head_sha => Ok(DiffRange::Compare {
            from: last.to_string(),
            to: record.head_sha.clone(),
        }),
        _ => Err(CoreError::InvalidInput(
            "nothing changed since your last review".into(),
        )),
    }
}

/// Fetch what `list_files` shows and the review's content keys. In since
/// mode both diffs are fetched at once: the compare for the files, the
/// review's own for the keys.
pub(crate) fn fetch(
    provider: &dyn ReviewProvider,
    record: &ReviewRecord,
    since_last_review: bool,
) -> CoreResult<(FileDiff, ContentKeys)> {
    let own = review_range(record);
    if !since_last_review {
        let diff = provider.fetch_files(&record.review, &own)?;
        let keys = content_keys(&diff);
        return Ok((diff, keys));
    }
    let since = since_range(record)?;
    let (compare, review) = std::thread::scope(|s| {
        let a = s.spawn(|| provider.fetch_files(&record.review, &since));
        let b = s.spawn(|| provider.fetch_files(&record.review, &own));
        (join(a), join(b))
    });
    let compare = compare?;
    let mut keys = content_keys(&review?);
    // A file the compare has but the review's diff does not (it changed and
    // changed back) is keyed by the compare's own sha.
    for f in &compare.files {
        keys.entry(f.path.clone())
            .or_insert_with(|| f.content_sha.clone());
    }
    Ok((compare, keys))
}

fn join<T>(h: std::thread::ScopedJoinHandle<'_, CoreResult<T>>) -> CoreResult<T> {
    h.join()
        .unwrap_or_else(|_| Err(CoreError::Provider("a file fetch panicked".into())))
}

/// Lay the viewed marks over `diff`.
pub(crate) fn assemble(
    id: ReviewId,
    diff: FileDiff,
    keys: &ContentKeys,
    marks: &HashMap<String, String>,
    since_last_review: bool,
) -> ReviewFiles {
    let viewed = viewed_paths(marks, keys);
    ReviewFiles {
        review_id: id,
        base_sha: diff.base_sha,
        head_sha: diff.head_sha,
        since_last_review,
        truncated: diff.truncated,
        base_approximate: diff.base_approximate,
        files: diff
            .files
            .into_iter()
            .map(|file| FileEntry {
                viewed: viewed.binary_search(&file.path).is_ok(),
                file,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
