//! Code view commands: the changed files, a file's contents and viewed
//! marks. All read from the provider, so all run off the IPC thread.

use serde::{Deserialize, Serialize};
use tauri::State;

use argus_core::cache::CacheStats;
use argus_core::{CoreResult, ReviewFiles};

use super::{blocking, parse_review_id};
use crate::error::IpcError;
use crate::state::AppState;

/// The review's changed files with their viewed marks: its own diff, or the
/// diff since the commit last marked reviewed.
#[tauri::command]
pub async fn list_files(
    state: State<'_, AppState>,
    review_id: String,
    since_last_review: bool,
) -> Result<ReviewFiles, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.list_files(id, since_last_review)).await
}

/// The content of `path` at `sha` (one of the shas `list_files` returned).
#[tauri::command]
pub async fn file_contents(
    state: State<'_, AppState>,
    review_id: String,
    sha: String,
    path: String,
) -> Result<String, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.file_at(id, &sha, &path)).await
}

/// One `(sha, path)` of [`file_contents_batch`].
#[derive(Debug, Deserialize)]
pub struct FileRequest {
    pub sha: String,
    pub path: String,
}

/// One entry of a batch answer: its value, or why that one failed (the
/// others still answer).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Batched<T> {
    Ok(T),
    Error(IpcError),
}

impl<T> From<CoreResult<T>> for Batched<T> {
    fn from(r: CoreResult<T>) -> Self {
        match r {
            Ok(v) => Batched::Ok(v),
            Err(e) => Batched::Error(IpcError::from(e)),
        }
    }
}

/// Most files one batch call reads: a larger ask is the webview's mistake,
/// and one call should never hold a blocking thread for long.
const MAX_BATCH: usize = 64;

/// The contents of several `(sha, path)` pairs of one review, in order:
/// what the cards loading together need, in one round trip. Each entry is
/// read as `file_contents` reads it (clone, disk cache, provider).
#[tauri::command]
pub async fn file_contents_batch(
    state: State<'_, AppState>,
    review_id: String,
    requests: Vec<FileRequest>,
) -> Result<Vec<Batched<String>>, IpcError> {
    let id = parse_review_id(&review_id)?;
    if requests.len() > MAX_BATCH {
        return Err(IpcError::invalid_input("too many files in one call"));
    }
    let wanted: Vec<(String, String)> = requests.into_iter().map(|r| (r.sha, r.path)).collect();
    let read = blocking(state, move |core| core.files_at(id, &wanted)).await?;
    Ok(read.into_iter().map(Batched::from).collect())
}

/// Read several files into the disk cache without sending them over:
/// the page's idle prefetch, so a later open (or another session) is
/// instant. Returns how many are now readable.
#[tauri::command]
pub async fn warm_file_cache(
    state: State<'_, AppState>,
    review_id: String,
    requests: Vec<FileRequest>,
) -> Result<usize, IpcError> {
    let id = parse_review_id(&review_id)?;
    if requests.len() > MAX_BATCH {
        return Err(IpcError::invalid_input("too many files in one call"));
    }
    let wanted: Vec<(String, String)> = requests.into_iter().map(|r| (r.sha, r.path)).collect();
    blocking(state, move |core| {
        Ok(core
            .files_at(id, &wanted)?
            .iter()
            .filter(|r| r.is_ok())
            .count())
    })
    .await
}

/// What the disk cache holds.
#[tauri::command]
pub async fn cache_stats(state: State<'_, AppState>) -> Result<CacheStats, IpcError> {
    blocking(state, |core| core.cache_stats()).await
}

/// Empty the disk cache; returns what it holds afterwards.
#[tauri::command]
pub async fn clear_cache(state: State<'_, AppState>) -> Result<CacheStats, IpcError> {
    blocking(state, |core| core.clear_cache()).await
}

/// Mark a changed file viewed or not; returns the review's viewed paths.
#[tauri::command]
pub async fn set_file_viewed(
    state: State<'_, AppState>,
    review_id: String,
    path: String,
    viewed: bool,
) -> Result<Vec<String>, IpcError> {
    let id = parse_review_id(&review_id)?;
    blocking(state, move |core| core.set_file_viewed(id, &path, viewed)).await
}
