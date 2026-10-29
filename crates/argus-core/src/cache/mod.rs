//! The disk cache of what never changes once read: a file's content at a
//! commit, and a review's file list (with its patches) at a pair of
//! commits. Reopening a review, switching between reviews or restarting
//! Argus then reads them from SQLite instead of the platform, also without
//! network for a head read before.
//!
//! - **Keys** name the platform, host, repository and the commits: a file
//!   is `(provider, host, repo, sha, path)`; a file list is `(provider,
//!   host, repo, number, range)` with the range's two shas (the review's
//!   stored base and head, or a compare's from and to). A new head is a
//!   new key, so nothing is ever invalidated: old entries age out.
//! - **What is stored** is the content or the normalised [`FileDiff`] only,
//!   never a platform answer with its headers, never an error. A read the
//!   local clone answered is not cached (the clone is faster; see
//!   [`Core::file_at`](crate::Core::file_at)).
//! - **Size**: every entry counts its bytes; after a write that takes the
//!   total over the limit ([`Settings::cache_limit_mb`](crate::Settings),
//!   default [`DEFAULT_LIMIT_MB`]), the least recently used entries go
//!   until it is under [`EVICT_TO`] of it. A read marks the entry used
//!   (`used_at`, a counter, not the clock: clocks step back).
//!
//! Migration 15 creates the table ([`MIGRATION_15`]).

use rusqlite::{params, OptionalExtension};

use crate::error::CoreResult;
use crate::provider::{DiffRange, FileDiff, ReviewRef};
use crate::store::Store;

/// The default limit of the cache, in megabytes.
pub const DEFAULT_LIMIT_MB: u64 = 512;
/// The smallest limit a setting can ask for: below it most reviews would
/// not fit and every open would read the platform again.
pub const MIN_LIMIT_MB: u64 = 32;
/// The largest limit a setting can ask for.
pub const MAX_LIMIT_MB: u64 = 16 * 1024;
/// Eviction frees down to this share of the limit, so the next few writes
/// do not each evict again.
pub const EVICT_TO: f64 = 0.9;

/// Migration 15: the cache table. `kind` is `file` or `diff`; `used_at`
/// orders the entries for eviction.
pub(crate) const MIGRATION_15: &str = r#"
    CREATE TABLE content_cache (
        key     TEXT PRIMARY KEY NOT NULL,
        kind    TEXT NOT NULL,
        body    BLOB NOT NULL,
        size    INTEGER NOT NULL,
        used_at INTEGER NOT NULL
    );
    CREATE INDEX content_cache_used ON content_cache (used_at);
"#;

/// What the cache holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub entries: u64,
    pub bytes: u64,
}

/// The key parts every entry starts with. A line break separates them: no
/// host, repository or sha has one, and a path comes last in its key, so
/// a path with one cannot pass for another key.
fn prefix(kind: &str, r: &ReviewRef) -> String {
    format!("{kind}\n{}\n{}\n{}", r.provider.as_str(), r.host, r.repo)
}

/// The key of `path` at `sha` in `r`'s repository.
pub(crate) fn file_key(r: &ReviewRef, sha: &str, path: &str) -> String {
    format!("{}\n{sha}\n{path}", prefix("file", r))
}

/// The key of review `r`'s file list over `range`.
pub(crate) fn diff_key(r: &ReviewRef, range: &DiffRange) -> String {
    let (what, a, b) = match range {
        DiffRange::Review { base, head } => ("review", base, head),
        DiffRange::Compare { from, to } => ("compare", from, to),
    };
    format!("{}\n{}\n{what}\n{a}\n{b}", prefix("diff", r), r.number)
}

/// A full commit sha (SHA-1 or SHA-256 hex): what names content that never
/// changes. A branch name or a short sha could name another commit later,
/// so a read at one is never cached.
fn is_commit_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `range` names commits only.
fn is_fixed(range: &DiffRange) -> bool {
    match range {
        DiffRange::Review { base, head } => is_commit_sha(base) && is_commit_sha(head),
        DiffRange::Compare { from, to } => is_commit_sha(from) && is_commit_sha(to),
    }
}

/// A limit in megabytes as bytes, clamped to the allowed range.
pub fn limit_bytes(mb: u64) -> u64 {
    mb.clamp(MIN_LIMIT_MB, MAX_LIMIT_MB) * 1024 * 1024
}

impl Store {
    /// The cached body under `key`, marking it used.
    pub(crate) fn cache_get(&self, key: &str) -> CoreResult<Option<Vec<u8>>> {
        let conn = self.conn();
        let body: Option<Vec<u8>> = conn
            .query_row(
                "SELECT body FROM content_cache WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        if body.is_some() {
            conn.execute(
                "UPDATE content_cache SET used_at =
                    (SELECT COALESCE(MAX(used_at), 0) + 1 FROM content_cache)
                 WHERE key = ?1",
                params![key],
            )?;
        }
        Ok(body)
    }

    /// Store `body` under `key` (replacing what was there), then evict the
    /// least recently used entries while the total is over `limit` bytes.
    /// A body larger than the limit is not stored.
    pub(crate) fn cache_put(
        &self,
        key: &str,
        kind: &str,
        body: &[u8],
        limit: u64,
    ) -> CoreResult<()> {
        if body.len() as u64 > limit {
            return Ok(());
        }
        self.transaction(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO content_cache (key, kind, body, size, used_at)
                 VALUES (?1, ?2, ?3, ?4,
                    (SELECT COALESCE(MAX(used_at), 0) + 1 FROM content_cache))",
                params![key, kind, body, body.len() as i64],
            )?;
            evict(conn, limit)
        })
    }

    /// Evict down to `limit` bytes now (a lower limit was saved).
    pub(crate) fn cache_trim(&self, limit: u64) -> CoreResult<()> {
        self.transaction(|conn| evict(conn, limit))
    }

    pub fn cache_stats(&self) -> CoreResult<CacheStats> {
        let (entries, bytes): (i64, i64) = self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM content_cache",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(CacheStats {
            entries: entries as u64,
            bytes: bytes as u64,
        })
    }

    /// Forget review `r`'s cached file lists (every range): the review's
    /// own refresh reads them from the platform again.
    pub(crate) fn cache_forget_diffs(&self, r: &ReviewRef) -> CoreResult<()> {
        // Every key with this prefix sorts between it and the prefix with
        // its last separator (`\n`) one higher (keys compare bytewise).
        let from = format!("{}\n{}\n", prefix("diff", r), r.number);
        let to = format!("{}\u{b}", &from[..from.len() - 1]);
        self.conn().execute(
            "DELETE FROM content_cache WHERE key >= ?1 AND key < ?2",
            params![from, to],
        )?;
        Ok(())
    }

    /// Empty the cache.
    pub fn cache_clear(&self) -> CoreResult<()> {
        self.conn().execute("DELETE FROM content_cache", [])?;
        Ok(())
    }
}

/// Drop the least recently used entries while the total is over `limit`,
/// down to [`EVICT_TO`] of it.
fn evict(conn: &rusqlite::Connection, limit: u64) -> CoreResult<()> {
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(size), 0) FROM content_cache",
        [],
        |r| r.get(0),
    )?;
    let total = total as u64;
    if total <= limit {
        return Ok(());
    }
    let target = (limit as f64 * EVICT_TO) as u64;
    let mut freed = 0u64;
    let mut last_used: Option<i64> = None;
    {
        let mut stmt = conn.prepare("SELECT size, used_at FROM content_cache ORDER BY used_at")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if total - freed <= target {
                break;
            }
            let size: i64 = row.get(0)?;
            freed += size as u64;
            last_used = Some(row.get(1)?);
        }
    }
    if let Some(upto) = last_used {
        conn.execute(
            "DELETE FROM content_cache WHERE used_at <= ?1",
            params![upto],
        )?;
    }
    Ok(())
}

/// The cache over one store, with the limit the settings give.
pub(crate) struct DiskCache<'a> {
    pub store: &'a Store,
    pub limit: u64,
}

impl DiskCache<'_> {
    /// `path` at `sha`, when cached. A failed read is a miss: the cache
    /// never makes a read fail.
    pub fn file(&self, r: &ReviewRef, sha: &str, path: &str) -> Option<String> {
        if !is_commit_sha(sha) {
            return None;
        }
        let body = self.store.cache_get(&file_key(r, sha, path)).ok()??;
        String::from_utf8(body).ok()
    }

    pub fn put_file(&self, r: &ReviewRef, sha: &str, path: &str, content: &str) {
        if !is_commit_sha(sha) {
            return;
        }
        if let Err(e) = self.store.cache_put(
            &file_key(r, sha, path),
            "file",
            content.as_bytes(),
            self.limit,
        ) {
            tracing::debug!("cache: could not store a file ({e})");
        }
    }

    /// Review `r`'s file list over `range`, when cached.
    pub fn diff(&self, r: &ReviewRef, range: &DiffRange) -> Option<FileDiff> {
        if !is_fixed(range) {
            return None;
        }
        let body = self.store.cache_get(&diff_key(r, range)).ok()??;
        serde_json::from_slice(&body).ok()
    }

    pub fn put_diff(&self, r: &ReviewRef, range: &DiffRange, diff: &FileDiff) {
        if !is_fixed(range) || !cacheable(range, diff) {
            return;
        }
        let Ok(body) = serde_json::to_vec(diff) else {
            return;
        };
        if let Err(e) = self
            .store
            .cache_put(&diff_key(r, range), "diff", &body, self.limit)
        {
            tracing::debug!("cache: could not store a file list ({e})");
        }
    }
}

/// Whether a diff read now may be cached under `range`: it is of the
/// commits the key names. A platform diffs a review as it is now, so a
/// review that moved since the last poll answers with a newer head than
/// the stored one its key would name.
pub(crate) fn cacheable(range: &DiffRange, diff: &FileDiff) -> bool {
    match range {
        DiffRange::Review { head, .. } => diff.head_sha == *head,
        DiffRange::Compare { to, .. } => diff.head_sha == *to,
    }
}

#[cfg(test)]
mod tests;
