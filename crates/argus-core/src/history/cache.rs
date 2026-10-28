//! The history cache in the v1 tables, keyed by (thread, head).
//!
//! - `snapshots`: one row per thread and head. `then_code` / `now_code`
//!   hold the [`CodeSnapshot`] as JSON (`then_path`/`then_line`,
//!   `now_path`/`now_line` repeat where they point, for a reader of the
//!   table); `status` holds the [`CachedMapping`] as JSON.
//! - `timeline`: the commit and resolve entries, one row each in timeline
//!   order (`kind` = `commit` / `resolved` / `unresolved`, `payload_json` =
//!   the [`TimelineEntry`]). Notes are not cached: they come from the
//!   thread as last refreshed, so a new reply shows without recomputing.
//!
//! Writing a head replaces what the thread had for any other head, so the
//! cache holds one entry per thread. A row that does not parse (written by
//! a newer Argus) reads as a miss.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{
    CodeSnapshot, LineStatus, LineTouch, LineTouches, SnapshotText, TimelineEntry, TimelineEvent,
};
use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::store::Store;

/// The walk's outcome as cached (the `status` column).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CachedMapping {
    pub status: LineStatus,
    pub touched_by: Vec<String>,
    /// Followed commit by commit (see [`LineTouch::usable`]).
    pub attributed: bool,
    /// The commit the comment was made on; `None` without one.
    pub then_sha: Option<String>,
    /// The review's commits when computed, oldest first.
    pub commits: Vec<String>,
    /// The thread's resolved flag when computed: a different flag now means
    /// a resolve event the entry does not have yet.
    pub resolved: bool,
}

/// Everything cached for one thread at one head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachedHistory {
    pub mapping: CachedMapping,
    pub then_snapshot: Option<CodeSnapshot>,
    pub now_snapshot: Option<CodeSnapshot>,
    /// Commit and resolve entries (notes are added on read).
    pub entries: Vec<TimelineEntry>,
}

/// What else a cached history was computed from, besides the thread and
/// the head it is keyed by: a different value now means it is stale. Kept
/// in the `status` JSON next to the [`CachedMapping`] (no schema change); a
/// row without one (written before it existed) stands until the head moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CacheKey {
    /// The review's base: a retarget changes what the lines are diffed
    /// against.
    pub base_sha: String,
    /// The platform's outdated flag on the thread (GitHub's current-line
    /// fallback only applies to a thread that is not outdated).
    pub outdated: bool,
    /// The local clone configured when computed: one configured since can
    /// answer exactly what the API could not.
    pub clone: Option<String>,
}

/// The `status` column: the mapping, plus the key it was computed under.
#[derive(Serialize)]
struct StatusOut<'a> {
    #[serde(flatten)]
    mapping: &'a CachedMapping,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<&'a CacheKey>,
}

#[derive(Deserialize)]
struct StatusIn {
    #[serde(flatten)]
    mapping: CachedMapping,
    #[serde(default)]
    key: Option<CacheKey>,
}

fn to_json<T: Serialize>(v: &T) -> CoreResult<String> {
    serde_json::to_string(v).map_err(|e| CoreError::Store(e.to_string()))
}

impl Store {
    /// The review a thread belongs to.
    pub fn thread_review(&self, thread: ThreadId) -> CoreResult<Option<ReviewId>> {
        let id: Option<String> = self
            .conn()
            .query_row(
                "SELECT review_id FROM threads WHERE id = ?1",
                [thread.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.and_then(|s| s.parse().ok()))
    }

    /// The cached history of `thread` at `head`, if any.
    pub(crate) fn cached_history(
        &self,
        thread: ThreadId,
        head: &str,
    ) -> CoreResult<Option<CachedHistory>> {
        Ok(self.cached_history_keyed(thread, head)?.map(|(h, _)| h))
    }

    /// [`cached_history`](Self::cached_history) with the key it was
    /// computed under (`None` for a row written without one).
    pub(crate) fn cached_history_keyed(
        &self,
        thread: ThreadId,
        head: &str,
    ) -> CoreResult<Option<(CachedHistory, Option<CacheKey>)>> {
        let conn = self.conn();
        let row: Option<(Option<String>, Option<String>, String)> = conn
            .query_row(
                "SELECT then_code, now_code, status FROM snapshots
                 WHERE thread_id = ?1 AND head_sha = ?2",
                params![thread.to_string(), head],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((then_code, now_code, status)) = row else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(
            "SELECT payload_json FROM timeline WHERE thread_id = ?1 AND head_sha = ?2
             ORDER BY id",
        )?;
        let payloads: Vec<String> = stmt
            .query_map(params![thread.to_string(), head], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let parsed = (|| {
            let status: StatusIn = serde_json::from_str(&status).ok()?;
            let history = CachedHistory {
                mapping: status.mapping,
                then_snapshot: then_code
                    .map(|c| serde_json::from_str(&c))
                    .transpose()
                    .ok()?,
                now_snapshot: now_code
                    .map(|c| serde_json::from_str(&c))
                    .transpose()
                    .ok()?,
                entries: payloads
                    .iter()
                    .map(|p| serde_json::from_str(p))
                    .collect::<Result<_, _>>()
                    .ok()?,
            };
            Some((history, status.key))
        })();
        Ok(parsed)
    }

    /// Cache `history` for `thread` at `head`, replacing what it had. A
    /// thread no longer stored (a refresh dropped it while the history was
    /// computed) gets nothing cached: there is nothing to hang it off.
    /// Without a [`CacheKey`] (tests); the history computation stores one.
    #[cfg(test)]
    pub(crate) fn put_history(
        &self,
        thread: ThreadId,
        head: &str,
        history: &CachedHistory,
    ) -> CoreResult<()> {
        self.put_history_keyed(thread, head, None, history)
    }

    /// [`put_history`](Self::put_history) under `key` (see [`CacheKey`]).
    pub(crate) fn put_history_keyed(
        &self,
        thread: ThreadId,
        head: &str,
        key: Option<&CacheKey>,
        history: &CachedHistory,
    ) -> CoreResult<()> {
        let id = thread.to_string();
        let then_code = history.then_snapshot.as_ref().map(to_json).transpose()?;
        let now_code = history.now_snapshot.as_ref().map(to_json).transpose()?;
        let status = to_json(&StatusOut {
            mapping: &history.mapping,
            key,
        })?;
        let (now_path, now_line) = match &history.mapping.status {
            LineStatus::Located { path, line, .. } => (Some(path.clone()), Some(*line)),
            _ => (history.now_snapshot.as_ref().map(|s| s.path.clone()), None),
        };
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let stored: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM threads WHERE id = ?1)",
            [&id],
            |r| r.get(0),
        )?;
        if !stored {
            return Ok(());
        }
        tx.execute("DELETE FROM snapshots WHERE thread_id = ?1", [&id])?;
        tx.execute("DELETE FROM timeline WHERE thread_id = ?1", [&id])?;
        tx.execute(
            "INSERT INTO snapshots (thread_id, head_sha, then_code, then_path, then_line,
                now_code, now_path, now_line, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                head,
                then_code,
                history.then_snapshot.as_ref().map(|s| s.path.clone()),
                history.then_snapshot.as_ref().and_then(|s| s.anchor_end),
                now_code,
                now_path,
                now_line,
                status,
            ],
        )?;
        for e in &history.entries {
            let kind = match e.event {
                TimelineEvent::Note { .. } => "note",
                TimelineEvent::Commit { .. } => "commit",
                TimelineEvent::Resolved { .. } => "resolved",
                TimelineEvent::Unresolved { .. } => "unresolved",
            };
            tx.execute(
                "INSERT INTO timeline (thread_id, head_sha, kind, at, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    head,
                    kind,
                    e.at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                    to_json(e)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The cached then/now text of `thread` at `head` (no computing; `None`
    /// on a miss).
    pub fn snapshot_text(&self, thread: ThreadId, head: &str) -> CoreResult<Option<SnapshotText>> {
        let Some(h) = self.cached_history(thread, head)? else {
            return Ok(None);
        };
        let join = |s: &Option<CodeSnapshot>| s.as_ref().map(|s| s.lines.join("\n"));
        let (now_path, now_line) = match &h.mapping.status {
            LineStatus::Located { path, line, .. } => (Some(path.clone()), Some(*line)),
            LineStatus::Deleted { path, .. } => (Some(path.clone()), None),
            _ => (None, None),
        };
        Ok(Some(SnapshotText {
            then_code: join(&h.then_snapshot),
            now_code: join(&h.now_snapshot),
            now_path,
            now_line,
            status: h.mapping.status,
        }))
    }

    /// What the cached walks of `review` at `head` say about each thread's
    /// lines, by platform thread id. Threads without a cached walk at this
    /// head are absent.
    pub fn line_touches(&self, review: ReviewId, head: &str) -> CoreResult<LineTouches> {
        read_line_touches(&self.conn(), review, head)
    }
}

/// [`Store::line_touches`] on a connection the caller holds (inside its
/// transaction).
pub(crate) fn read_line_touches(
    conn: &Connection,
    review: ReviewId,
    head: &str,
) -> CoreResult<LineTouches> {
    let mut stmt = conn.prepare(
        "SELECT t.provider_thread_id, s.status FROM snapshots s
         JOIN threads t ON t.id = s.thread_id
         WHERE t.review_id = ?1 AND s.head_sha = ?2",
    )?;
    let rows = stmt.query_map(params![review.to_string(), head], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = LineTouches::new();
    for row in rows {
        let (thread, status) = row?;
        let Ok(m) = serde_json::from_str::<CachedMapping>(&status) else {
            continue;
        };
        let Some(then_sha) = m.then_sha else { continue };
        let usable = m.attributed
            && matches!(
                m.status,
                LineStatus::Located { .. }
                    | LineStatus::Deleted { .. }
                    | LineStatus::FileGone { .. }
            );
        out.insert(
            thread,
            LineTouch {
                then_sha,
                commits: m.commits,
                touched_by: m.touched_by,
                usable,
            },
        );
    }
    Ok(out)
}
