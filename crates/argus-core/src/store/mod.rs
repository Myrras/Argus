//! Local review state in SQLite (`rusqlite`, bundled).
//!
//! The database path is passed in (the shell picks the app data dir). The
//! schema is versioned with `PRAGMA user_version`: [`MIGRATIONS`] holds one
//! step per version, applied in order inside a transaction, so a database
//! from any earlier release is brought up to date on open.
//!
//! Nothing secret is ever stored here.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::provider::{Anchor, MyReviewState, Note, ProviderId, ReviewRef, ReviewState, Thread};
use crate::review::{ReviewStatus, RoundReason};

/// How long a call waits for a lock another connection holds.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Schema steps; index `i` takes the database from version `i` to `i + 1`.
/// Append only — never edit a released step.
const MIGRATIONS: &[&str] = &[
    // 1: the full schema from the technical plan (§6). `reviews` carries a
    // few columns on top of the plan's for round tracking: the reasons, the
    // classified status, the untouched-thread count, whether the user's
    // review is requested (to see a re-request as a transition), whether
    // the last list still had it, the viewer's username on that platform,
    // and a fingerprint of the last fetch.
    r#"
    CREATE TABLE reviews (
        id                TEXT PRIMARY KEY,
        provider          TEXT NOT NULL,
        host              TEXT NOT NULL,
        repo              TEXT NOT NULL,
        number            INTEGER NOT NULL,
        title             TEXT NOT NULL,
        author            TEXT NOT NULL,
        url               TEXT NOT NULL,
        state             TEXT NOT NULL,
        draft             INTEGER NOT NULL DEFAULT 0,
        head_sha          TEXT NOT NULL,
        base_sha          TEXT NOT NULL,
        last_reviewed_sha TEXT,
        last_reviewed_at  TEXT,
        approved_at       TEXT,
        round             INTEGER NOT NULL DEFAULT 1,
        round_reasons     TEXT NOT NULL DEFAULT '[]',
        status            TEXT NOT NULL,
        untouched_threads INTEGER NOT NULL DEFAULT 0,
        requested         INTEGER NOT NULL DEFAULT 0,
        rerequested_at    TEXT,
        listed            INTEGER NOT NULL DEFAULT 1,
        viewer            TEXT NOT NULL DEFAULT '',
        fingerprint       TEXT NOT NULL DEFAULT '',
        updated_at        TEXT,
        fetched_at        TEXT NOT NULL,
        UNIQUE (provider, host, repo, number)
    );
    CREATE TABLE threads (
        id                 TEXT PRIMARY KEY,
        review_id          TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        provider_thread_id TEXT NOT NULL,
        author             TEXT NOT NULL,
        created_at         TEXT NOT NULL,
        resolved           INTEGER NOT NULL,
        outdated           INTEGER NOT NULL,
        anchor_json        TEXT,
        mine               INTEGER NOT NULL,
        UNIQUE (review_id, provider_thread_id)
    );
    CREATE INDEX threads_by_review ON threads(review_id);
    CREATE TABLE notes (
        id         TEXT NOT NULL,
        thread_id  TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        author     TEXT NOT NULL,
        body       TEXT NOT NULL,
        created_at TEXT NOT NULL,
        system     INTEGER NOT NULL,
        PRIMARY KEY (thread_id, id)
    );
    CREATE TABLE snapshots (
        thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        head_sha  TEXT NOT NULL,
        then_code TEXT,
        then_path TEXT,
        then_line INTEGER,
        now_code  TEXT,
        now_path  TEXT,
        now_line  INTEGER,
        status    TEXT NOT NULL,
        PRIMARY KEY (thread_id, head_sha)
    );
    CREATE TABLE timeline (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        thread_id    TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        head_sha     TEXT NOT NULL,
        kind         TEXT NOT NULL,
        at           TEXT NOT NULL,
        payload_json TEXT NOT NULL
    );
    CREATE INDEX timeline_by_thread ON timeline(thread_id, head_sha);
    CREATE TABLE findings (
        id         TEXT PRIMARY KEY,
        review_id  TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        round      INTEGER NOT NULL,
        path       TEXT,
        line       INTEGER,
        severity   TEXT NOT NULL,
        title      TEXT NOT NULL,
        body       TEXT NOT NULL,
        posted_at  TEXT,
        posted_url TEXT,
        dismissed  INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE verdicts (
        thread_id     TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        round         INTEGER NOT NULL,
        ai_label      TEXT,
        ai_reason     TEXT,
        user_override TEXT,
        PRIMARY KEY (thread_id, round)
    );
    CREATE TABLE checklist (
        id        TEXT PRIMARY KEY,
        review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        round     INTEGER NOT NULL,
        text      TEXT NOT NULL,
        checked   INTEGER NOT NULL DEFAULT 0
    );
    "#,
    // 2: files marked viewed in the code view, per review. `content_sha` is
    // what the file was at head when marked (`ChangedFile::content_sha`): a
    // mark only counts while the file still has it, so a file that changed
    // since shows unviewed again.
    r#"
    CREATE TABLE viewed_files (
        review_id   TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        path        TEXT NOT NULL,
        content_sha TEXT NOT NULL,
        viewed_at   TEXT NOT NULL,
        PRIMARY KEY (review_id, path)
    );
    "#,
    // 3 and 4: intentionally empty. M3 (thread history) and M4 (posting)
    // needed no schema change, but the numbers were reserved for them while
    // the three cores were built in parallel. Never replace these with real
    // steps: later steps exist, so databases already past them would skip
    // the change. New schema goes at the end.
    "-- 3: reserved (M3)",
    "-- 4: reserved (M4)",
    // 5: AI run state and the side of a finding (see `ai::store`).
    crate::ai::store::MIGRATION,
    // 6: the head and left side of the diff a finding was read at.
    crate::ai::store::MIGRATION_6,
    // 7: findings an AI run left out as repeats of the review.
    crate::ai::store::MIGRATION_7,
    // 8: a thread the user replied in is theirs too (`Thread::is_mine`),
    // not only one they started. Fetches only rewrite a review's threads
    // when it changed, so the stored flags are brought up to the rule here:
    // a human (non-system) note by the review's viewer makes it mine.
    // `lower` folds ASCII only, as `eq_ignore_ascii_case` does.
    r#"
    UPDATE threads SET mine = 1
    WHERE mine = 0 AND EXISTS (
        SELECT 1 FROM notes n JOIN reviews r ON r.id = threads.review_id
        WHERE n.thread_id = threads.id AND n.system = 0
          AND r.viewer <> '' AND lower(n.author) = lower(r.viewer)
    );
    "#,
    // 9: findings set aside as repeats of the review, kept with what they
    // repeat; a posted finding's platform ids; checklist repeats counted.
    crate::ai::store::MIGRATION_9,
    // 10: checklist items set aside as repeats of the review, kept; the
    // points a finding relates to.
    crate::ai::store::MIGRATION_10,
    // 11: the points a checklist item relates to.
    crate::ai::store::MIGRATION_11,
    // 12: reserved for the thread resolve work built alongside (see 3 and
    // 4 for why a reserved step stays empty forever).
    "-- 12: reserved (thread resolve)",
    // 13: the user's own review state on the platform, and when (and at
    // which head) they last submitted a review there: the platform's
    // reviewer state decides whether a review waits on the user. Written by
    // fetches; a stored review gets them on its next fetch.
    r#"
    ALTER TABLE reviews ADD COLUMN my_review_state TEXT NOT NULL DEFAULT 'notReviewer';
    ALTER TABLE reviews ADD COLUMN my_reviewed_at TEXT;
    ALTER TABLE reviews ADD COLUMN my_reviewed_sha TEXT;
    "#,
];

/// One review as Argus tracks it: the platform data of the last fetch plus
/// the round bookkeeping. This is what the review list shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecord {
    pub id: ReviewId,
    #[serde(flatten)]
    pub review: ReviewRef,
    pub state: ReviewState,
    pub draft: bool,
    pub head_sha: String,
    pub base_sha: String,
    /// The watermark: the head the user last marked reviewed, and when.
    pub last_reviewed_sha: Option<String>,
    pub last_reviewed_at: Option<DateTime<Utc>>,
    pub approved_at: Option<DateTime<Utc>>,
    /// 1 for the first round, +1 each time it needs re-review.
    pub round: u32,
    pub round_reasons: Vec<RoundReason>,
    pub status: ReviewStatus,
    pub untouched_threads: u32,
    /// The user's review is requested right now.
    pub requested: bool,
    pub rerequested_at: Option<DateTime<Utc>>,
    /// The user's own review state on the platform, as of the last fetch.
    pub my_review_state: MyReviewState,
    /// When the user last submitted a review on the platform, and at which
    /// head (see [`ReviewDetail::my_reviewed_at`]): counts as a watermark
    /// when it is later than Argus' own.
    ///
    /// [`ReviewDetail::my_reviewed_at`]: crate::provider::ReviewDetail::my_reviewed_at
    pub my_reviewed_at: Option<DateTime<Utc>>,
    pub my_reviewed_sha: Option<String>,
    /// The last list call still returned it. An open review that dropped
    /// off (no longer requested, nothing commented) is kept but not active.
    pub listed: bool,
    /// The user's username on this review's platform.
    pub viewer: String,
    /// Hash of the last fetch, to tell whether anything changed.
    #[serde(skip)]
    pub fingerprint: String,
    pub fetched_at: DateTime<Utc>,
}

impl ReviewRecord {
    /// Shown in one of the active tabs (not approved, still listed or
    /// closed without approval).
    pub fn is_active(&self) -> bool {
        match self.status {
            ReviewStatus::Approved => false,
            ReviewStatus::ClosedWithoutApproval => true,
            _ => self.listed,
        }
    }
}

/// A thread as stored: the platform thread plus Argus' own keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredThread {
    /// Argus' id (stable; derived from the review and the platform id).
    pub thread_id: ThreadId,
    pub review_id: ReviewId,
    /// The user's thread: started by them or with a comment of theirs in it
    /// ([`Thread::is_mine`]).
    pub mine: bool,
    /// Started by the user ([`Thread::started_by`]), within [`mine`](Self::mine).
    pub started_by_me: bool,
    #[serde(flatten)]
    pub thread: Thread,
}

/// The database. Cheap to share behind an `Arc`; calls serialise on one
/// connection, which is plenty for one user's reviews.
pub struct Store {
    conn: Mutex<Connection>,
    /// Held for a whole refresh of every provider (see
    /// [`poll::refresh_all`](crate::poll::refresh_all)): refreshes over one
    /// database run one at a time, so a manual refresh never fetches
    /// alongside a poll and races it to the store.
    pub(crate) refreshing: Mutex<()>,
    /// The order fetches of a review started in (see [`Store::begin_fetch`]).
    fetch_seq: AtomicU64,
    /// Per review, the newest fetch stored so far (see
    /// [`Store::claim_fetch`]). In memory: ordering only matters between
    /// fetches of one process, so nothing is migrated or persisted.
    stored_fetch: Mutex<HashMap<ReviewId, u64>>,
}

fn invalid(what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("bad stored {what}").into(),
    )
}

impl Store {
    /// Open (creating if needed) the database at `path` and migrate it.
    pub fn open(path: &Path) -> CoreResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    /// A private in-memory database (tests, or before a data dir exists).
    pub fn in_memory() -> CoreResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> CoreResult<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        // Another connection to the file (a second Argus, a backup tool)
        // holding a lock: wait for it rather than fail the call at once.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // WAL keeps the UI's reads from waiting on a poll's writes; an
        // in-memory database answers "memory" and that is fine.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        migrate(&mut conn)?;
        Ok(Self::over(conn))
    }

    /// A store over `conn` as it is (no pragmas, no migration).
    pub(crate) fn over(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
            refreshing: Mutex::new(()),
            fetch_seq: AtomicU64::new(0),
            stored_fetch: Mutex::new(HashMap::new()),
        }
    }

    /// The ticket of a fetch starting now: later starts get higher ones.
    /// Fetches are ordered by it, not by the wall clock, which can step
    /// back (an NTP correction, a manual change) and make an older fetch
    /// look newer.
    pub(crate) fn begin_fetch(&self) -> u64 {
        self.fetch_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Whether the fetch `seq` of review `id` is the newest one to land so
    /// far; it is then recorded as such. Call it inside the transaction
    /// that stores the fetch, so checking and storing are one step for
    /// concurrent fetches. A transaction that fails afterwards still
    /// leaves `seq` recorded: an older fetch is then dropped too, and the
    /// next poll stores the review.
    pub(crate) fn claim_fetch(&self, id: ReviewId, seq: u64) -> bool {
        let mut stored = self.stored_fetch.lock().expect("fetch order lock poisoned");
        let newest = stored.entry(id).or_insert(0);
        if seq < *newest {
            return false;
        }
        *newest = seq;
        true
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("store lock poisoned")
    }

    /// The schema version the database is at.
    pub fn schema_version(&self) -> CoreResult<u32> {
        Ok(self
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Insert or update a review (keyed by its id), every column. Threads
    /// and notes hang off it and are kept.
    ///
    /// A full-row write of a record read earlier loses whatever changed in
    /// between (a poll's fetch, the user's watermark): code that reads,
    /// changes and writes back does so in one [`transaction`](Self::transaction).
    pub fn upsert_review(&self, r: &ReviewRecord) -> CoreResult<()> {
        write_review(&self.conn(), r, WriteColumns::All)
    }

    /// Run `f` in one transaction, holding the connection throughout (every
    /// other call on this store waits). `IMMEDIATE` takes the write lock up
    /// front, so what `f` reads is still current when it writes.
    ///
    /// `f` works through the `Connection` it is given (the `pub(crate)`
    /// free functions of this module), never through `self`: a `Store`
    /// method called inside would wait on the lock `f` runs under.
    pub(crate) fn transaction<T>(
        &self,
        f: impl FnOnce(&Connection) -> CoreResult<T>,
    ) -> CoreResult<T> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn review(&self, id: ReviewId) -> CoreResult<Option<ReviewRecord>> {
        read_review(&self.conn(), id)
    }

    /// Every stored review, most recently updated on the platform first.
    pub fn reviews(&self) -> CoreResult<Vec<ReviewRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {REVIEW_COLUMNS} FROM reviews
             ORDER BY updated_at IS NULL, updated_at DESC, number DESC"
        ))?;
        let rows = stmt.query_map([], review_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether the last list still had the review.
    pub fn set_listed(&self, id: ReviewId, listed: bool) -> CoreResult<()> {
        self.update(
            "UPDATE reviews SET listed = ?2 WHERE id = ?1",
            params![id.to_string(), listed],
        )
    }

    /// Record that the review was fetched at `at` without anything to store.
    pub fn touch_fetched(&self, id: ReviewId, at: DateTime<Utc>) -> CoreResult<()> {
        touch_fetched(&self.conn(), id, at)
    }

    /// Move the watermark.
    pub fn set_watermark(&self, id: ReviewId, sha: &str, at: DateTime<Utc>) -> CoreResult<()> {
        self.update(
            "UPDATE reviews SET last_reviewed_sha = ?2, last_reviewed_at = ?3 WHERE id = ?1",
            params![id.to_string(), sha, at],
        )
    }

    pub fn set_approved_at(&self, id: ReviewId, at: Option<DateTime<Utc>>) -> CoreResult<()> {
        self.update(
            "UPDATE reviews SET approved_at = ?2 WHERE id = ?1",
            params![id.to_string(), at],
        )
    }

    /// Store a classification: round number, reasons, status, untouched count.
    pub fn set_round(
        &self,
        id: ReviewId,
        round: u32,
        reasons: &[RoundReason],
        status: ReviewStatus,
        untouched_threads: u32,
    ) -> CoreResult<()> {
        let reasons =
            serde_json::to_string(reasons).map_err(|e| CoreError::Store(e.to_string()))?;
        self.update(
            "UPDATE reviews SET round = ?2, round_reasons = ?3, status = ?4,
                untouched_threads = ?5 WHERE id = ?1",
            params![
                id.to_string(),
                round,
                reasons,
                status.as_str(),
                untouched_threads
            ],
        )
    }

    /// Run an UPDATE on one review; no such review is [`CoreError::NotFound`].
    fn update(&self, sql: &str, p: impl rusqlite::Params) -> CoreResult<()> {
        match self.conn().execute(sql, p)? {
            0 => Err(CoreError::NotFound),
            _ => Ok(()),
        }
    }

    /// Make the stored threads of `review` exactly `threads`: upsert each
    /// thread and its notes, drop the ones the platform no longer has. An
    /// upsert (never a delete + insert) keeps what hangs off a thread —
    /// snapshots, timeline, verdicts — across refreshes.
    pub fn upsert_threads(&self, review: ReviewId, threads: &[Thread], me: &str) -> CoreResult<()> {
        self.transaction(|conn| write_threads(conn, review, threads, me))
    }

    /// The stored threads of `review`, oldest first, notes oldest first.
    pub fn threads(&self, review: ReviewId) -> CoreResult<Vec<StoredThread>> {
        read_threads(&self.conn(), review)
    }
}

impl Store {
    /// Mark `path` viewed at `content_sha` (replacing an older mark).
    pub fn set_viewed(
        &self,
        review: ReviewId,
        path: &str,
        content_sha: &str,
        at: DateTime<Utc>,
    ) -> CoreResult<()> {
        self.conn().execute(
            "INSERT INTO viewed_files (review_id, path, content_sha, viewed_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(review_id, path) DO UPDATE SET
                content_sha = excluded.content_sha, viewed_at = excluded.viewed_at",
            params![review.to_string(), path, content_sha, at],
        )?;
        Ok(())
    }

    /// Mark every path of `files` (path → content sha) viewed at that
    /// content, in one transaction.
    pub fn set_viewed_all(
        &self,
        review: ReviewId,
        files: &HashMap<String, String>,
        at: DateTime<Utc>,
    ) -> CoreResult<()> {
        self.transaction(|conn| {
            for (path, content_sha) in files {
                conn.execute(
                    "INSERT INTO viewed_files (review_id, path, content_sha, viewed_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(review_id, path) DO UPDATE SET
                        content_sha = excluded.content_sha, viewed_at = excluded.viewed_at",
                    params![review.to_string(), path, content_sha, at],
                )?;
            }
            Ok(())
        })
    }

    /// Drop the viewed mark of `path` (a no-op when there is none).
    pub fn clear_viewed(&self, review: ReviewId, path: &str) -> CoreResult<()> {
        self.conn().execute(
            "DELETE FROM viewed_files WHERE review_id = ?1 AND path = ?2",
            params![review.to_string(), path],
        )?;
        Ok(())
    }

    /// Every viewed mark of `review`: path → the content sha it was made at.
    pub fn viewed(&self, review: ReviewId) -> CoreResult<HashMap<String, String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT path, content_sha FROM viewed_files WHERE review_id = ?1")?;
        let rows = stmt.query_map([review.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// The paths of `current` (path → content sha at head) that are marked
/// viewed at exactly that content: a mark made before the file changed
/// does not count.
pub fn viewed_paths(
    marks: &HashMap<String, String>,
    current: &HashMap<String, String>,
) -> Vec<String> {
    let mut out: Vec<String> = current
        .iter()
        .filter(|(path, sha)| marks.get(*path) == Some(*sha))
        .map(|(path, _)| path.clone())
        .collect();
    out.sort();
    out
}

/// Which columns [`write_review`] writes over an existing row.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteColumns {
    All,
    /// Everything but the user's own columns (the watermark): what a fetch
    /// may write. A fetch never moves the watermark, so it never writes it
    /// back either, not even unchanged.
    Fetched,
}

/// Insert or update one review row (see [`Store::upsert_review`]).
pub(crate) fn write_review(
    conn: &Connection,
    r: &ReviewRecord,
    cols: WriteColumns,
) -> CoreResult<()> {
    let reasons =
        serde_json::to_string(&r.round_reasons).map_err(|e| CoreError::Store(e.to_string()))?;
    let watermark = match cols {
        WriteColumns::All => {
            "last_reviewed_sha = excluded.last_reviewed_sha,
                last_reviewed_at = excluded.last_reviewed_at,"
        }
        WriteColumns::Fetched => "",
    };
    conn.execute(
        &format!(
            "INSERT INTO reviews (id, provider, host, repo, number, title, author, url, state,
                draft, head_sha, base_sha, last_reviewed_sha, last_reviewed_at, approved_at,
                round, round_reasons, status, untouched_threads, requested, rerequested_at,
                listed, viewer, fingerprint, updated_at, fetched_at, my_review_state,
                my_reviewed_at, my_reviewed_sha)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title, author = excluded.author, url = excluded.url,
                state = excluded.state, draft = excluded.draft, head_sha = excluded.head_sha,
                base_sha = excluded.base_sha, {watermark} approved_at = excluded.approved_at,
                round = excluded.round, round_reasons = excluded.round_reasons,
                status = excluded.status, untouched_threads = excluded.untouched_threads,
                requested = excluded.requested, rerequested_at = excluded.rerequested_at,
                listed = excluded.listed, viewer = excluded.viewer,
                fingerprint = excluded.fingerprint, updated_at = excluded.updated_at,
                fetched_at = excluded.fetched_at, my_review_state = excluded.my_review_state,
                my_reviewed_at = excluded.my_reviewed_at,
                my_reviewed_sha = excluded.my_reviewed_sha"
        ),
        params![
            r.id.to_string(),
            r.review.provider.as_str(),
            r.review.host,
            r.review.repo,
            r.review.number,
            r.review.title,
            r.review.author,
            r.review.url,
            r.state.as_str(),
            r.draft,
            r.head_sha,
            r.base_sha,
            r.last_reviewed_sha,
            r.last_reviewed_at,
            r.approved_at,
            r.round,
            reasons,
            r.status.as_str(),
            r.untouched_threads,
            r.requested,
            r.rerequested_at,
            r.listed,
            r.viewer,
            r.fingerprint,
            r.review.updated_at,
            r.fetched_at,
            r.my_review_state.as_str(),
            r.my_reviewed_at,
            r.my_reviewed_sha,
        ],
    )?;
    Ok(())
}

/// One stored review.
pub(crate) fn read_review(conn: &Connection, id: ReviewId) -> CoreResult<Option<ReviewRecord>> {
    Ok(conn
        .query_row(
            &format!("SELECT {REVIEW_COLUMNS} FROM reviews WHERE id = ?1"),
            [id.to_string()],
            review_row,
        )
        .optional()?)
}

/// [`Store::touch_fetched`] in the caller's transaction.
pub(crate) fn touch_fetched(conn: &Connection, id: ReviewId, at: DateTime<Utc>) -> CoreResult<()> {
    conn.execute(
        "UPDATE reviews SET fetched_at = ?2 WHERE id = ?1",
        params![id.to_string(), at],
    )?;
    Ok(())
}

/// Store what the user marking a review reviewed changes: the watermark and
/// the classification it gives. Nothing a fetch owns.
pub(crate) fn write_mark(conn: &Connection, r: &ReviewRecord) -> CoreResult<()> {
    let reasons =
        serde_json::to_string(&r.round_reasons).map_err(|e| CoreError::Store(e.to_string()))?;
    let n = conn.execute(
        "UPDATE reviews SET last_reviewed_sha = ?2, last_reviewed_at = ?3, status = ?4,
            round_reasons = ?5, untouched_threads = ?6 WHERE id = ?1",
        params![
            r.id.to_string(),
            r.last_reviewed_sha,
            r.last_reviewed_at,
            r.status.as_str(),
            reasons,
            r.untouched_threads
        ],
    )?;
    match n {
        0 => Err(CoreError::NotFound),
        _ => Ok(()),
    }
}

/// Make the stored threads of `review` exactly `threads` (see
/// [`Store::upsert_threads`]), in the caller's transaction.
pub(crate) fn write_threads(
    conn: &Connection,
    review: ReviewId,
    threads: &[Thread],
    me: &str,
) -> CoreResult<()> {
    let mut keep: Vec<String> = Vec::with_capacity(threads.len());
    for t in threads {
        let thread_id = ThreadId::from_key(review, &t.id).to_string();
        let anchor = t
            .anchor
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| CoreError::Store(e.to_string()))?;
        conn.execute(
            "INSERT INTO threads (id, review_id, provider_thread_id, author, created_at,
                resolved, outdated, anchor_json, mine)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                author = excluded.author, created_at = excluded.created_at,
                resolved = excluded.resolved, outdated = excluded.outdated,
                anchor_json = excluded.anchor_json, mine = excluded.mine",
            params![
                thread_id,
                review.to_string(),
                t.id,
                t.author,
                t.created_at,
                t.resolved,
                t.outdated,
                anchor,
                t.is_mine(me),
            ],
        )?;
        let mut note_ids: Vec<&str> = Vec::with_capacity(t.notes.len());
        for n in &t.notes {
            conn.execute(
                "INSERT INTO notes (id, thread_id, author, body, created_at, system)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(thread_id, id) DO UPDATE SET
                    author = excluded.author, body = excluded.body,
                    created_at = excluded.created_at, system = excluded.system",
                params![n.id, thread_id, n.author, n.body, n.created_at, n.system],
            )?;
            note_ids.push(&n.id);
        }
        delete_missing(
            conn,
            "notes",
            "id",
            "thread_id",
            &thread_id,
            &note_ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )?;
        keep.push(thread_id);
    }
    delete_missing(
        conn,
        "threads",
        "id",
        "review_id",
        &review.to_string(),
        &keep,
    )?;
    Ok(())
}

/// The stored threads of `review` (see [`Store::threads`]).
pub(crate) fn read_threads(conn: &Connection, review: ReviewId) -> CoreResult<Vec<StoredThread>> {
    // Who "me" is for `started_by_me`: the user the threads were last
    // written for (`write_threads` gets the same name).
    let viewer: String = conn
        .query_row(
            "SELECT viewer FROM reviews WHERE id = ?1",
            [review.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let mut stmt = conn.prepare(
        "SELECT id, provider_thread_id, author, created_at, resolved, outdated,
            anchor_json, mine
         FROM threads WHERE review_id = ?1 ORDER BY created_at, provider_thread_id",
    )?;
    let mut notes_stmt = conn.prepare(
        "SELECT id, author, body, created_at, system FROM notes
         WHERE thread_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([review.to_string()], |r| {
        let id: String = r.get(0)?;
        let anchor: Option<String> = r.get(6)?;
        let anchor: Option<Anchor> = anchor
            .map(|a| serde_json::from_str(&a))
            .transpose()
            .map_err(|_| invalid("anchor"))?;
        Ok((
            id,
            Thread {
                id: r.get(1)?,
                author: r.get(2)?,
                created_at: r.get(3)?,
                resolved: r.get(4)?,
                outdated: r.get(5)?,
                anchor,
                notes: Vec::new(),
            },
            r.get::<_, bool>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, mut thread, mine) = row?;
        thread.notes = notes_stmt
            .query_map([&id], |r| {
                Ok(Note {
                    id: r.get(0)?,
                    author: r.get(1)?,
                    body: r.get(2)?,
                    created_at: r.get(3)?,
                    system: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        out.push(StoredThread {
            thread_id: id.parse().map_err(|_| invalid("thread id"))?,
            review_id: review,
            mine,
            started_by_me: !viewer.is_empty() && thread.started_by(&viewer),
            thread,
        });
    }
    Ok(out)
}

/// Delete rows of `table` under `parent` whose `key` is not in `keep`.
fn delete_missing(
    tx: &Connection,
    table: &str,
    key: &str,
    parent_col: &str,
    parent: &str,
    keep: &[String],
) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare(&format!(
        "SELECT {key} FROM {table} WHERE {parent_col} = ?1"
    ))?;
    let existing: Vec<String> = stmt
        .query_map([parent], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for k in existing.iter().filter(|k| !keep.contains(k)) {
        tx.execute(
            &format!("DELETE FROM {table} WHERE {parent_col} = ?1 AND {key} = ?2"),
            params![parent, k],
        )?;
    }
    Ok(())
}

fn migrate(conn: &mut Connection) -> CoreResult<()> {
    let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > MIGRATIONS.len() {
        return Err(CoreError::Store(
            "the database is from a newer Argus; update Argus".into(),
        ));
    }
    for (i, step) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction()?;
        tx.execute_batch(step)?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
    }
    Ok(())
}

const REVIEW_COLUMNS: &str = "id, provider, host, repo, number, title, author, url, state, draft,
    head_sha, base_sha, last_reviewed_sha, last_reviewed_at, approved_at, round, round_reasons,
    status, untouched_threads, requested, rerequested_at, listed, viewer, fingerprint,
    updated_at, fetched_at, my_review_state, my_reviewed_at, my_reviewed_sha";

fn review_row(r: &Row<'_>) -> rusqlite::Result<ReviewRecord> {
    let id: String = r.get(0)?;
    let provider: String = r.get(1)?;
    let state: String = r.get(8)?;
    let reasons: String = r.get(16)?;
    let status: String = r.get(17)?;
    Ok(ReviewRecord {
        id: id.parse().map_err(|_| invalid("review id"))?,
        review: ReviewRef {
            provider: ProviderId::parse(&provider).ok_or_else(|| invalid("provider"))?,
            host: r.get(2)?,
            repo: r.get(3)?,
            number: r.get(4)?,
            title: r.get(5)?,
            author: r.get(6)?,
            url: r.get(7)?,
            updated_at: r.get(24)?,
        },
        state: ReviewState::parse(&state).ok_or_else(|| invalid("state"))?,
        draft: r.get(9)?,
        head_sha: r.get(10)?,
        base_sha: r.get(11)?,
        last_reviewed_sha: r.get(12)?,
        last_reviewed_at: r.get(13)?,
        approved_at: r.get(14)?,
        round: r.get(15)?,
        round_reasons: serde_json::from_str(&reasons).map_err(|_| invalid("round reasons"))?,
        status: ReviewStatus::parse(&status).ok_or_else(|| invalid("status"))?,
        untouched_threads: r.get(18)?,
        requested: r.get(19)?,
        rerequested_at: r.get(20)?,
        my_review_state: MyReviewState::parse(&r.get::<_, String>(26)?)
            .ok_or_else(|| invalid("review state"))?,
        my_reviewed_at: r.get(27)?,
        my_reviewed_sha: r.get(28)?,
        listed: r.get(21)?,
        viewer: r.get(22)?,
        fingerprint: r.get(23)?,
        fetched_at: r.get(25)?,
    })
}

#[cfg(test)]
mod tests;
