//! AI state in SQLite: the v1 `findings`, `checklist` and `verdicts`
//! tables, plus `ai_runs` (migration 5, [`MIGRATION`]), the diff a
//! finding was read at (migration 6, [`MIGRATION_6`]), the count of
//! findings left out as repeats of the review (migration 7,
//! [`MIGRATION_7`]) and, since migration 9 ([`MIGRATION_9`]), those
//! findings themselves with what they repeat, plus the platform ids of a
//! posted finding; since migration 10 ([`MIGRATION_10`]) the checklist
//! items that repeat the review too, and what a finding relates to; since
//! migration 11 ([`MIGRATION_11`]) what a checklist item relates to.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::dedupe::RaisedRef;
use super::model::{
    AiRunState, AiRunStatus, ChecklistId, ChecklistItem, Finding, FindingId, RelatedRef, RepeatOf,
    RepeatReason, Severity, Verdict, VerdictLabel,
};
use super::output::RunOutput;
use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::provider::Side;
use crate::store::Store;

/// Migration 5: one row per review with its latest AI run, and the side a
/// finding is anchored on (v1 `findings` has path + line only).
pub(crate) const MIGRATION: &str = r#"
    CREATE TABLE ai_runs (
        review_id        TEXT PRIMARY KEY REFERENCES reviews(id) ON DELETE CASCADE,
        status           TEXT NOT NULL,
        round            INTEGER NOT NULL,
        head_sha         TEXT NOT NULL DEFAULT '',
        adapter          TEXT NOT NULL DEFAULT '',
        error            TEXT,
        interrupted      INTEGER NOT NULL DEFAULT 0,
        started_at       TEXT,
        finished_at      TEXT,
        last_done_round  INTEGER,
        dropped_findings INTEGER NOT NULL DEFAULT 0,
        dropped_verdicts INTEGER NOT NULL DEFAULT 0
    );
    ALTER TABLE findings ADD COLUMN side TEXT NOT NULL DEFAULT 'new';
    CREATE INDEX findings_by_review ON findings(review_id, round);
    CREATE INDEX checklist_by_review ON checklist(review_id, round);
"#;

/// Migration 6: the diff a finding was read at. `head_sha` is the right side
/// its new-side lines are numbered at (the post API moves them from there);
/// `base_sha` the left side its old-side lines are numbered at, which for a
/// run over the commits since the last review is the watermark, not the
/// review's base. Rows from before stay NULL.
pub(crate) const MIGRATION_6: &str = r#"
    ALTER TABLE findings ADD COLUMN head_sha TEXT;
    ALTER TABLE findings ADD COLUMN base_sha TEXT;
"#;

/// Migration 7: findings a run left out because the review already makes
/// their point (`super::dedupe`). Rows from before count none.
pub(crate) const MIGRATION_7: &str = r#"
    ALTER TABLE ai_runs ADD COLUMN dropped_duplicates INTEGER NOT NULL DEFAULT 0;
"#;

/// Migration 9: a finding that repeats the review is kept, set aside:
/// `repeat_reason` (`agentMarked` / `similar`) with the thread or posted
/// finding it repeats; `shown_anyway` once the user shows it (local only).
/// A posted finding keeps the platform's thread and comment ids, which
/// find its thread again whatever its text. `ai_runs.skipped_checklist`
/// counts the checklist items a run left out as repeats. Rows from before
/// are no repeats and have no ids (their URL still names the comment).
pub(crate) const MIGRATION_9: &str = r#"
    ALTER TABLE findings ADD COLUMN repeat_reason TEXT;
    ALTER TABLE findings ADD COLUMN repeat_of_thread TEXT;
    ALTER TABLE findings ADD COLUMN repeat_of_finding TEXT;
    ALTER TABLE findings ADD COLUMN shown_anyway INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE findings ADD COLUMN posted_thread_id TEXT;
    ALTER TABLE findings ADD COLUMN posted_note_id TEXT;
    ALTER TABLE ai_runs ADD COLUMN skipped_checklist INTEGER NOT NULL DEFAULT 0;
"#;

/// Migration 10: a checklist item that repeats the review is kept, set
/// aside, like a finding (migration 9 only counted it); and a finding keeps
/// the points the agent said it relates to (`related_to`, a JSON array of
/// [`RelatedRef`]; NULL: none). Rows from before are no repeats and
/// relate to nothing.
pub(crate) const MIGRATION_10: &str = r#"
    ALTER TABLE checklist ADD COLUMN repeat_reason TEXT;
    ALTER TABLE checklist ADD COLUMN repeat_of_thread TEXT;
    ALTER TABLE checklist ADD COLUMN repeat_of_finding TEXT;
    ALTER TABLE checklist ADD COLUMN shown_anyway INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE findings ADD COLUMN related_to TEXT;
"#;

/// Migration 11: a checklist item keeps the points it relates to, as a
/// finding does (`related_to`, migration 10). Rows from before relate to
/// nothing.
pub(crate) const MIGRATION_11: &str = r#"
    ALTER TABLE checklist ADD COLUMN related_to TEXT;
"#;

/// Error text of a run that was running when Argus stopped.
pub(crate) const INTERRUPTED: &str = "interrupted";

/// The persisted part of an [`AiRunState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunRow {
    pub review_id: ReviewId,
    pub status: AiRunStatus,
    pub round: u32,
    pub adapter: String,
    pub error: Option<String>,
    /// Failed because Argus stopped mid-run (retried automatically).
    pub interrupted: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub last_done_round: Option<u32>,
    pub dropped_findings: u32,
    pub dropped_verdicts: u32,
    pub dropped_duplicates: u32,
    pub skipped_checklist: u32,
}

impl RunRow {
    pub fn state(&self, enabled: bool, queued: bool) -> AiRunState {
        AiRunState {
            review_id: self.review_id,
            enabled,
            status: self.status,
            queued,
            round: Some(self.round),
            adapter: Some(self.adapter.clone()).filter(|a| !a.is_empty()),
            error: self.error.clone(),
            started_at: self.started_at,
            finished_at: self.finished_at,
            last_done_round: self.last_done_round,
            dropped_findings: self.dropped_findings,
            dropped_verdicts: self.dropped_verdicts,
            dropped_duplicates: self.dropped_duplicates,
            skipped_checklist: self.skipped_checklist,
            read_only_guaranteed: true,
        }
    }
}

fn bad(what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("bad stored {what}").into(),
    )
}

const RUN_COLUMNS: &str = "review_id, status, round, adapter, error, interrupted, started_at,
    finished_at, last_done_round, dropped_findings, dropped_verdicts, dropped_duplicates,
    skipped_checklist";

fn run_row(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    let id: String = r.get(0)?;
    let status: String = r.get(1)?;
    Ok(RunRow {
        review_id: id.parse().map_err(|_| bad("review id"))?,
        status: AiRunStatus::parse(&status).ok_or_else(|| bad("run status"))?,
        round: r.get(2)?,
        adapter: r.get(3)?,
        error: r.get(4)?,
        interrupted: r.get(5)?,
        started_at: r.get(6)?,
        finished_at: r.get(7)?,
        last_done_round: r.get(8)?,
        dropped_findings: r.get(9)?,
        dropped_verdicts: r.get(10)?,
        dropped_duplicates: r.get(11)?,
        skipped_checklist: r.get(12)?,
    })
}

pub(crate) fn run(store: &Store, review: ReviewId) -> CoreResult<Option<RunRow>> {
    Ok(store
        .conn()
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM ai_runs WHERE review_id = ?1"),
            [review.to_string()],
            run_row,
        )
        .optional()?)
}

pub(crate) fn runs(store: &Store) -> CoreResult<HashMap<ReviewId, RunRow>> {
    let conn = store.conn();
    let mut stmt = conn.prepare(&format!("SELECT {RUN_COLUMNS} FROM ai_runs"))?;
    let rows = stmt.query_map([], run_row)?;
    rows.map(|r| r.map(|r| (r.review_id, r)).map_err(CoreError::from))
        .collect()
}

/// A run started: it is the review's latest now (the last successful round
/// is kept).
pub(crate) fn mark_running(
    store: &Store,
    review: ReviewId,
    round: u32,
    head_sha: &str,
    adapter: &str,
    now: DateTime<Utc>,
) -> CoreResult<()> {
    store.conn().execute(
        "INSERT INTO ai_runs (review_id, status, round, head_sha, adapter, started_at)
         VALUES (?1, 'running', ?2, ?3, ?4, ?5)
         ON CONFLICT(review_id) DO UPDATE SET
            status = 'running', round = excluded.round, head_sha = excluded.head_sha,
            adapter = excluded.adapter, error = NULL, interrupted = 0,
            started_at = excluded.started_at, finished_at = NULL",
        params![review.to_string(), round, head_sha, adapter, now],
    )?;
    Ok(())
}

pub(crate) fn mark_failed(
    store: &Store,
    review: ReviewId,
    error: &str,
    now: DateTime<Utc>,
) -> CoreResult<()> {
    store.conn().execute(
        "UPDATE ai_runs SET status = 'failed', error = ?2, interrupted = 0, finished_at = ?3
         WHERE review_id = ?1",
        params![review.to_string(), error, now],
    )?;
    Ok(())
}

/// On start-up: a run still `running` was cut off by the app stopping.
pub(crate) fn interrupt_running(store: &Store, now: DateTime<Utc>) -> CoreResult<usize> {
    Ok(store.conn().execute(
        "UPDATE ai_runs SET status = 'failed', error = ?1, interrupted = 1, finished_at = ?2
         WHERE status = 'running'",
        params![INTERRUPTED, now],
    )?)
}

/// Store a successful run's output for `round` and mark the run done, in
/// one transaction. A rerun of the same round replaces that round's
/// unposted findings (repeats of the review included) and its checklist
/// (repeats included; keeping dismissed and shown-anyway marks on a finding
/// with the same path, line, side and title, and checked and shown-anyway
/// marks on an item with the same text)
/// and the AI half of its verdicts (keeping overrides; a thread the rerun
/// says nothing about loses its stale AI half). A finding identical to one
/// of the round already posted is not stored again, so it cannot be posted
/// twice. Other rounds' findings stay. A verdict in a new round takes the
/// user's override along from the thread's previous one.
pub(crate) fn save_done(
    store: &Store,
    review: ReviewId,
    round: u32,
    out: &RunOutput,
    now: DateTime<Utc>,
) -> CoreResult<()> {
    let mut conn = store.conn();
    let tx = conn.transaction()?;
    let rid = review.to_string();

    // A finding of this round with `mark` set, by path, line, side and
    // title (one on the other side of the same line is another finding).
    let marked = |mark: &str| -> CoreResult<HashSet<(String, u32, String, String)>> {
        let mut stmt = tx.prepare(&format!(
            "SELECT path, line, side, title FROM findings
             WHERE review_id = ?1 AND round = ?2 AND {mark}"
        ))?;
        let rows = stmt.query_map(params![rid, round], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    let dismissed = marked("dismissed = 1")?;
    let shown_anyway = marked("shown_anyway = 1")?;
    let posted = marked("posted_at IS NOT NULL")?;
    tx.execute(
        "DELETE FROM findings WHERE review_id = ?1 AND round = ?2 AND posted_at IS NULL",
        params![rid, round],
    )?;
    for f in &out.findings {
        let side = side_str(f.side);
        let key = (f.path.clone(), f.line, side.to_string(), f.title.clone());
        if posted.contains(&key) {
            continue;
        }
        let was_dismissed = dismissed.contains(&key);
        let repeat = f.repeat.map(|r| r.repeat_of());
        let related = related_json(&f.related_to)?;
        tx.execute(
            "INSERT INTO findings (id, review_id, round, path, line, side, severity, title, body,
                dismissed, head_sha, base_sha, repeat_reason, repeat_of_thread,
                repeat_of_finding, shown_anyway, related_to)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                FindingId::new().to_string(),
                rid,
                round,
                f.path,
                f.line,
                side,
                f.severity.as_str(),
                f.title,
                f.body,
                was_dismissed,
                out.head_sha,
                out.base_sha,
                repeat.as_ref().map(|r| r.reason.as_str()),
                repeat
                    .as_ref()
                    .and_then(|r| r.thread_id)
                    .map(|t| t.to_string()),
                repeat
                    .as_ref()
                    .and_then(|r| r.finding_id)
                    .map(|f| f.to_string()),
                repeat.is_some() && shown_anyway.contains(&key),
                related,
            ],
        )?;
    }

    // An item of this round with `mark` set, by text.
    let items = |mark: &str| -> CoreResult<HashSet<String>> {
        let mut stmt = tx.prepare(&format!(
            "SELECT text FROM checklist WHERE review_id = ?1 AND round = ?2 AND {mark}"
        ))?;
        let rows = stmt.query_map(params![rid, round], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    let checked = items("checked = 1")?;
    let shown_items = items("shown_anyway = 1")?;
    tx.execute(
        "DELETE FROM checklist WHERE review_id = ?1 AND round = ?2",
        params![rid, round],
    )?;
    for c in &out.checklist {
        let repeat = c.repeat.map(|r| r.repeat_of());
        tx.execute(
            "INSERT INTO checklist (id, review_id, round, text, checked, repeat_reason,
                repeat_of_thread, repeat_of_finding, shown_anyway, related_to)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                ChecklistId::new().to_string(),
                rid,
                round,
                c.text,
                checked.contains(&c.text),
                repeat.as_ref().map(|r| r.reason.as_str()),
                repeat
                    .as_ref()
                    .and_then(|r| r.thread_id)
                    .map(|t| t.to_string()),
                repeat
                    .as_ref()
                    .and_then(|r| r.finding_id)
                    .map(|f| f.to_string()),
                repeat.is_some() && shown_items.contains(&c.text),
                related_json(&c.related_to)?,
            ],
        )?;
    }

    // This round's AI half is replaced as a whole: a thread the rerun left
    // out must not keep the verdict of the earlier run.
    tx.execute(
        "UPDATE verdicts SET ai_label = NULL, ai_reason = NULL
         WHERE round = ?2 AND thread_id IN (SELECT id FROM threads WHERE review_id = ?1)",
        params![rid, round],
    )?;
    for (thread, label, reason) in &out.verdicts {
        // A new row carries the user's override of the thread's latest
        // earlier verdict: the newest round is the one shown, and the
        // user's own call must not vanish behind it.
        tx.execute(
            "INSERT INTO verdicts (thread_id, round, ai_label, ai_reason, user_override)
             VALUES (?1, ?2, ?3, ?4,
                (SELECT user_override FROM verdicts
                 WHERE thread_id = ?1 AND round < ?2 ORDER BY round DESC LIMIT 1))
             ON CONFLICT(thread_id, round) DO UPDATE SET
                ai_label = excluded.ai_label, ai_reason = excluded.ai_reason",
            params![thread.to_string(), round, label.as_str(), reason],
        )?;
    }
    tx.execute(
        "DELETE FROM verdicts
         WHERE round = ?2 AND ai_label IS NULL AND user_override IS NULL
            AND thread_id IN (SELECT id FROM threads WHERE review_id = ?1)",
        params![rid, round],
    )?;

    let n = tx.execute(
        "UPDATE ai_runs SET status = 'done', error = NULL, interrupted = 0, round = ?2,
            finished_at = ?3, last_done_round = ?2, dropped_findings = ?4, dropped_verdicts = ?5,
            dropped_duplicates = ?6, skipped_checklist = ?7
         WHERE review_id = ?1",
        params![
            rid,
            round,
            now,
            out.dropped_findings,
            out.dropped_verdicts,
            out.dropped_duplicates,
            out.skipped_checklist
        ],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    tx.commit()?;
    Ok(())
}

/// Related points as stored: a JSON array of [`RelatedRef`], NULL for none.
fn related_json(related: &[RaisedRef]) -> CoreResult<Option<String>> {
    if related.is_empty() {
        return Ok(None);
    }
    let refs: Vec<RelatedRef> = related.iter().map(|r| r.related_ref()).collect();
    serde_json::to_string(&refs)
        .map(Some)
        .map_err(|e| CoreError::Store(e.to_string()))
}

/// Related points as read back (column `at`).
fn related_at(r: &Row<'_>, at: usize) -> rusqlite::Result<Vec<RelatedRef>> {
    match r.get::<_, Option<String>>(at)? {
        None => Ok(Vec::new()),
        Some(json) => serde_json::from_str(&json).map_err(|_| bad("related points")),
    }
}

fn side_str(side: Side) -> &'static str {
    match side {
        Side::New => "new",
        Side::Old => "old",
    }
}

const FINDING_COLUMNS: &str = "id, review_id, round, path, line, side, severity, title, body,
    dismissed, posted_at, posted_url, repeat_reason, repeat_of_thread, repeat_of_finding,
    related_to";

/// A finding or checklist item set aside as a repeat of the review, not
/// shown anyway.
const SKIPPED: &str = "repeat_reason IS NOT NULL AND shown_anyway = 0";

/// The repeat mark in columns `at` (reason), `at + 1` (thread) and `at + 2`
/// (posted finding).
fn repeat_at(r: &Row<'_>, at: usize) -> rusqlite::Result<Option<RepeatOf>> {
    let Some(reason) = r.get::<_, Option<String>>(at)? else {
        return Ok(None);
    };
    Ok(Some(RepeatOf {
        reason: RepeatReason::parse(&reason).ok_or_else(|| bad("repeat reason"))?,
        thread_id: r
            .get::<_, Option<String>>(at + 1)?
            .map(|t| t.parse().map_err(|_| bad("thread id")))
            .transpose()?,
        finding_id: r
            .get::<_, Option<String>>(at + 2)?
            .map(|f| f.parse().map_err(|_| bad("finding id")))
            .transpose()?,
    }))
}

fn finding_row(r: &Row<'_>) -> rusqlite::Result<Finding> {
    let id: String = r.get(0)?;
    let review: String = r.get(1)?;
    let side: String = r.get(5)?;
    let severity: String = r.get(6)?;
    Ok(Finding {
        id: id.parse().map_err(|_| bad("finding id"))?,
        review_id: review.parse().map_err(|_| bad("review id"))?,
        round: r.get(2)?,
        path: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        line: r.get::<_, Option<u32>>(4)?.unwrap_or_default(),
        side: match side.as_str() {
            "old" => Side::Old,
            _ => Side::New,
        },
        severity: Severity::parse(&severity).ok_or_else(|| bad("severity"))?,
        title: r.get(7)?,
        body: r.get(8)?,
        dismissed: r.get(9)?,
        posted_at: r.get(10)?,
        posted_url: r.get(11)?,
        repeat_of: repeat_at(r, 12)?,
        related_to: related_at(r, 15)?,
    })
}

/// Every finding of `review` shown as one, all rounds: severity (high
/// first), then path and line, newest round first among equals. Repeats of
/// the review are left out unless shown anyway ([`skipped_findings`]).
pub(crate) fn findings(store: &Store, review: ReviewId) -> CoreResult<Vec<Finding>> {
    findings_where(store, review, &format!("NOT ({SKIPPED})"))
}

/// The findings of `review` set aside as repeats of the review, in the
/// same order.
pub(crate) fn skipped_findings(store: &Store, review: ReviewId) -> CoreResult<Vec<Finding>> {
    findings_where(store, review, SKIPPED)
}

fn findings_where(store: &Store, review: ReviewId, filter: &str) -> CoreResult<Vec<Finding>> {
    let conn = store.conn();
    let mut stmt = conn.prepare(&format!(
        "SELECT {FINDING_COLUMNS} FROM findings WHERE review_id = ?1 AND {filter}
         ORDER BY CASE severity WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
            path, line, round DESC, rowid"
    ))?;
    let rows = stmt.query_map([review.to_string()], finding_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Show a finding set aside as a repeat anyway: it becomes a normal
/// finding (keeping what it repeats). Local only.
pub(crate) fn show_anyway(store: &Store, id: FindingId) -> CoreResult<Finding> {
    let conn = store.conn();
    let n = conn.execute(
        "UPDATE findings SET shown_anyway = 1 WHERE id = ?1",
        params![id.to_string()],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(conn.query_row(
        &format!("SELECT {FINDING_COLUMNS} FROM findings WHERE id = ?1"),
        [id.to_string()],
        finding_row,
    )?)
}

/// Whether `id` is set aside as a repeat of the review (and not shown
/// anyway): such a finding is not posted.
pub(crate) fn is_skipped(store: &Store, id: FindingId) -> CoreResult<bool> {
    Ok(store
        .conn()
        .query_row(
            &format!("SELECT {SKIPPED} FROM findings WHERE id = ?1"),
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// The platform ids a posted finding's comment got (`None` before
/// migration 9).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PostedIds {
    pub thread: Option<String>,
    pub note: Option<String>,
}

/// [`PostedIds`] of every posted finding of `review`.
pub(crate) fn posted_ids(
    store: &Store,
    review: ReviewId,
) -> CoreResult<HashMap<FindingId, PostedIds>> {
    let conn = store.conn();
    let mut stmt = conn.prepare(
        "SELECT id, posted_thread_id, posted_note_id FROM findings
         WHERE review_id = ?1 AND posted_at IS NOT NULL",
    )?;
    let rows = stmt.query_map([review.to_string()], |r| {
        let id: String = r.get(0)?;
        Ok((
            id.parse().map_err(|_| bad("finding id"))?,
            PostedIds {
                thread: r.get(1)?,
                note: r.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) fn set_dismissed(store: &Store, id: FindingId, dismissed: bool) -> CoreResult<Finding> {
    let conn = store.conn();
    let n = conn.execute(
        "UPDATE findings SET dismissed = ?2 WHERE id = ?1",
        params![id.to_string(), dismissed],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(conn.query_row(
        &format!("SELECT {FINDING_COLUMNS} FROM findings WHERE id = ?1"),
        [id.to_string()],
        finding_row,
    )?)
}

pub(crate) fn finding(store: &Store, id: FindingId) -> CoreResult<Option<Finding>> {
    let conn = store.conn();
    Ok(conn
        .query_row(
            &format!("SELECT {FINDING_COLUMNS} FROM findings WHERE id = ?1"),
            [id.to_string()],
            finding_row,
        )
        .optional()?)
}

/// The head and left side of the diff `id` was read at (`None` for a
/// finding stored before migration 6).
pub(crate) fn finding_shas(
    store: &Store,
    id: FindingId,
) -> CoreResult<(Option<String>, Option<String>)> {
    Ok(store.conn().query_row(
        "SELECT head_sha, base_sha FROM findings WHERE id = ?1",
        [id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Record that the user posted the finding as a comment at `url`, with the
/// platform's ids of it.
pub(crate) fn mark_posted(
    store: &Store,
    id: FindingId,
    url: &str,
    ids: &PostedIds,
    now: DateTime<Utc>,
) -> CoreResult<()> {
    let n = store.conn().execute(
        "UPDATE findings SET posted_at = ?2, posted_url = ?3, posted_thread_id = ?4,
            posted_note_id = ?5
         WHERE id = ?1",
        params![id.to_string(), now, url, ids.thread, ids.note],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

const CHECKLIST_COLUMNS: &str = "id, review_id, round, text, checked, repeat_reason,
    repeat_of_thread, repeat_of_finding, related_to";

fn checklist_row(r: &Row<'_>) -> rusqlite::Result<ChecklistItem> {
    let id: String = r.get(0)?;
    let review: String = r.get(1)?;
    Ok(ChecklistItem {
        id: id.parse().map_err(|_| bad("checklist id"))?,
        review_id: review.parse().map_err(|_| bad("review id"))?,
        round: r.get(2)?,
        text: r.get(3)?,
        checked: r.get(4)?,
        repeat_of: repeat_at(r, 5)?,
        related_to: related_at(r, 8)?,
    })
}

/// Every checklist item of `review` shown as one: newest round first, in
/// the order the agent gave them. Repeats of the review are left out unless
/// shown anyway ([`skipped_checklist`]).
pub(crate) fn checklist(store: &Store, review: ReviewId) -> CoreResult<Vec<ChecklistItem>> {
    checklist_where(store, review, &format!("NOT ({SKIPPED})"))
}

/// The checklist items of `review` set aside as repeats of the review, in
/// the same order.
pub(crate) fn skipped_checklist(store: &Store, review: ReviewId) -> CoreResult<Vec<ChecklistItem>> {
    checklist_where(store, review, SKIPPED)
}

fn checklist_where(
    store: &Store,
    review: ReviewId,
    filter: &str,
) -> CoreResult<Vec<ChecklistItem>> {
    let conn = store.conn();
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHECKLIST_COLUMNS} FROM checklist WHERE review_id = ?1 AND {filter}
         ORDER BY round DESC, rowid"
    ))?;
    let rows = stmt.query_map([review.to_string()], checklist_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn checklist_item(conn: &rusqlite::Connection, id: ChecklistId) -> CoreResult<ChecklistItem> {
    Ok(conn.query_row(
        &format!("SELECT {CHECKLIST_COLUMNS} FROM checklist WHERE id = ?1"),
        [id.to_string()],
        checklist_row,
    )?)
}

/// Show a checklist item set aside as a repeat anyway: it becomes a normal
/// item (keeping what it repeats). Local only.
pub(crate) fn show_item_anyway(store: &Store, id: ChecklistId) -> CoreResult<ChecklistItem> {
    let conn = store.conn();
    let n = conn.execute(
        "UPDATE checklist SET shown_anyway = 1 WHERE id = ?1",
        params![id.to_string()],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    checklist_item(&conn, id)
}

pub(crate) fn set_checked(
    store: &Store,
    id: ChecklistId,
    checked: bool,
) -> CoreResult<ChecklistItem> {
    let conn = store.conn();
    let n = conn.execute(
        "UPDATE checklist SET checked = ?2 WHERE id = ?1",
        params![id.to_string(), checked],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound);
    }
    checklist_item(&conn, id)
}

fn label(s: Option<String>) -> rusqlite::Result<Option<VerdictLabel>> {
    s.map(|s| VerdictLabel::parse(&s).ok_or_else(|| bad("verdict label")))
        .transpose()
}

fn verdict_row(r: &Row<'_>) -> rusqlite::Result<Verdict> {
    let thread: String = r.get(0)?;
    let review: String = r.get(1)?;
    let ai_label = label(r.get(3)?)?;
    let user_override = label(r.get(5)?)?;
    Ok(Verdict {
        thread_id: thread.parse().map_err(|_| bad("thread id"))?,
        review_id: review.parse().map_err(|_| bad("review id"))?,
        round: r.get(2)?,
        ai_label,
        ai_reason: r.get(4)?,
        user_override,
        label: user_override.or(ai_label),
    })
}

const VERDICT_SELECT: &str = "SELECT v.thread_id, t.review_id, v.round, v.ai_label, v.ai_reason,
    v.user_override FROM verdicts v JOIN threads t ON t.id = v.thread_id";

/// The current verdict per thread of `review`: its latest round's.
pub(crate) fn verdicts(store: &Store, review: ReviewId) -> CoreResult<Vec<Verdict>> {
    let conn = store.conn();
    let mut stmt = conn.prepare(&format!(
        "{VERDICT_SELECT} WHERE t.review_id = ?1
            AND v.round = (SELECT MAX(round) FROM verdicts WHERE thread_id = v.thread_id)
         ORDER BY t.created_at, t.provider_thread_id"
    ))?;
    let rows = stmt.query_map([review.to_string()], verdict_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Set (or clear) the user's own label on a thread's current verdict. A
/// thread without one gets a row at its review's current round.
pub(crate) fn set_override(
    store: &Store,
    thread: ThreadId,
    value: Option<VerdictLabel>,
) -> CoreResult<Verdict> {
    let mut conn = store.conn();
    let tx = conn.transaction()?;
    let tid = thread.to_string();
    let review_round: Option<u32> = tx
        .query_row(
            "SELECT r.round FROM threads t JOIN reviews r ON r.id = t.review_id WHERE t.id = ?1",
            [&tid],
            |r| r.get(0),
        )
        .optional()?;
    let review_round = review_round.ok_or(CoreError::NotFound)?;
    let latest: Option<u32> = tx.query_row(
        "SELECT MAX(round) FROM verdicts WHERE thread_id = ?1",
        [&tid],
        |r| r.get(0),
    )?;
    let round = latest.unwrap_or(review_round);
    tx.execute(
        "INSERT INTO verdicts (thread_id, round, user_override) VALUES (?1, ?2, ?3)
         ON CONFLICT(thread_id, round) DO UPDATE SET user_override = excluded.user_override",
        params![tid, round, value.map(VerdictLabel::as_str)],
    )?;
    let v = tx.query_row(
        &format!("{VERDICT_SELECT} WHERE v.thread_id = ?1 AND v.round = ?2"),
        params![tid, round],
        verdict_row,
    )?;
    tx.commit()?;
    Ok(v)
}
