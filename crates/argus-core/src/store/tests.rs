use super::*;
use crate::review::advance;
use crate::testutil::{at, detail, note, thread, AUTHOR, ME};

fn record() -> ReviewRecord {
    advance(None, &detail("h1"), &[], ME, true, at(10, 10))
}

fn table_names(store: &Store) -> Vec<String> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn migrates_an_empty_database_to_the_full_schema() {
    let store = Store::in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    let tables = table_names(&store);
    for t in [
        "checklist",
        "content_cache",
        "findings",
        "notes",
        "reviews",
        "snapshots",
        "threads",
        "timeline",
        "verdicts",
        "viewed_files",
    ] {
        assert!(tables.iter().any(|n| n == t), "missing table {t}");
    }
}

#[test]
fn reopening_keeps_data_and_does_not_rerun_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data/argus.db");
    let r = record();
    {
        let store = Store::open(&path).unwrap();
        store.upsert_review(&r).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    assert_eq!(store.review(r.id).unwrap(), Some(r));
}

#[test]
fn refuses_a_database_from_a_newer_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argus.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
    }
    assert!(matches!(Store::open(&path), Err(CoreError::Store(_))));
}

#[test]
fn upsert_review_is_idempotent_and_round_trips() {
    let store = Store::in_memory().unwrap();
    let mut r = record();
    r.fingerprint = "abc".into();
    store.upsert_review(&r).unwrap();
    store.upsert_review(&r).unwrap();
    assert_eq!(store.reviews().unwrap(), vec![r.clone()]);

    r.review.title = "Renamed".into();
    r.head_sha = "h2".into();
    r.round_reasons = vec![RoundReason::NewCommits, RoundReason::ReRequested];
    r.status = ReviewStatus::NeedsReReview;
    store.upsert_review(&r).unwrap();
    let all = store.reviews().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0], r);
}

#[test]
fn watermark_approval_and_round_updates() {
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();

    store.set_watermark(r.id, "h1", at(11, 8)).unwrap();
    store.set_approved_at(r.id, Some(at(11, 9))).unwrap();
    store
        .set_round(
            r.id,
            3,
            &[RoundReason::AuthorReplied],
            ReviewStatus::NeedsReReview,
            2,
        )
        .unwrap();
    let got = store.review(r.id).unwrap().unwrap();
    assert_eq!(got.last_reviewed_sha.as_deref(), Some("h1"));
    assert_eq!(got.last_reviewed_at, Some(at(11, 8)));
    assert_eq!(got.approved_at, Some(at(11, 9)));
    assert_eq!(got.round, 3);
    assert_eq!(got.round_reasons, [RoundReason::AuthorReplied]);
    assert_eq!(got.status, ReviewStatus::NeedsReReview);
    assert_eq!(got.untouched_threads, 2);

    store.set_approved_at(r.id, None).unwrap();
    assert_eq!(store.review(r.id).unwrap().unwrap().approved_at, None);

    let missing = crate::ids::ReviewId::from_key("nope");
    assert!(matches!(
        store.set_watermark(missing, "x", at(1, 1)),
        Err(CoreError::NotFound)
    ));
    assert_eq!(store.review(missing).unwrap(), None);
}

#[test]
fn upsert_threads_is_idempotent_and_drops_what_is_gone() {
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();
    let t1 = thread(
        "501",
        ME,
        "h1",
        vec![note("501", ME, at(10, 12)), note("502", AUTHOR, at(10, 13))],
    );
    let t2 = thread("510", AUTHOR, "h1", vec![note("510", AUTHOR, at(10, 14))]);
    store
        .upsert_threads(r.id, &[t1.clone(), t2.clone()], ME)
        .unwrap();
    store
        .upsert_threads(r.id, &[t1.clone(), t2.clone()], ME)
        .unwrap();

    let got = store.threads(r.id).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].thread, t1);
    assert!(got[0].mine && !got[1].mine);
    assert!(got[0].started_by_me && !got[1].started_by_me);
    assert_eq!(got[0].thread_id, ThreadId::from_key(r.id, "501"));

    // Something hanging off a thread survives an upsert of that thread.
    store
        .conn()
        .execute(
            "INSERT INTO snapshots (thread_id, head_sha, status) VALUES (?1, 'h1', 'live')",
            [got[0].thread_id.to_string()],
        )
        .unwrap();

    // A reply was deleted and the second thread is gone.
    let mut t1b = t1.clone();
    t1b.notes.truncate(1);
    t1b.resolved = true;
    store.upsert_threads(r.id, &[t1b.clone()], ME).unwrap();
    let got = store.threads(r.id).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].thread, t1b);
    let snapshots: u32 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(snapshots, 1);
}

#[test]
fn migrates_a_version_1_database_to_viewed_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argus.db");
    let r = record();
    {
        // A database as the first release left it.
        let mut conn = Connection::open(&path).unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute_batch(MIGRATIONS[0]).unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.commit().unwrap();
    }
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
        assert!(table_names(&store).iter().any(|t| t == "viewed_files"));
        store.upsert_review(&r).unwrap();
        store
            .set_viewed(r.id, "src/cache.rs", "blob1", at(11, 8))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.viewed(r.id).unwrap(),
        HashMap::from([("src/cache.rs".to_string(), "blob1".to_string())])
    );
}

#[test]
fn viewed_marks_reset_when_the_file_changed() {
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();
    store
        .set_viewed(r.id, "a.rs", "blob-a1", at(11, 8))
        .unwrap();
    store
        .set_viewed(r.id, "b.rs", "blob-b1", at(11, 8))
        .unwrap();
    let current = |a: &str, b: &str| {
        HashMap::from([
            ("a.rs".to_string(), a.to_string()),
            ("b.rs".to_string(), b.to_string()),
            ("c.rs".to_string(), "blob-c1".to_string()),
        ])
    };
    let marks = store.viewed(r.id).unwrap();
    assert_eq!(
        viewed_paths(&marks, &current("blob-a1", "blob-b1")),
        ["a.rs", "b.rs"]
    );
    // b.rs changed at head since: its mark no longer counts.
    assert_eq!(
        viewed_paths(&marks, &current("blob-a1", "blob-b2")),
        ["a.rs"]
    );

    // Re-marking takes the new content; clearing drops it (twice is fine).
    store
        .set_viewed(r.id, "b.rs", "blob-b2", at(12, 8))
        .unwrap();
    store.clear_viewed(r.id, "a.rs").unwrap();
    store.clear_viewed(r.id, "a.rs").unwrap();
    let marks = store.viewed(r.id).unwrap();
    assert_eq!(
        viewed_paths(&marks, &current("blob-a1", "blob-b2")),
        ["b.rs"]
    );

    // Marks go with their review.
    store
        .conn()
        .execute("DELETE FROM reviews WHERE id = ?1", [r.id.to_string()])
        .unwrap();
    assert!(store.viewed(r.id).unwrap().is_empty());
}

/// Insert `r` with the columns every schema since v1 has (the store's own
/// write also sets later ones, which an old schema lacks).
fn insert_old_review(store: &Store, r: &ReviewRecord) {
    store
        .conn()
        .execute(
            "INSERT INTO reviews (id, provider, host, repo, number, title, author, url, state,
                draft, head_sha, base_sha, last_reviewed_sha, last_reviewed_at, approved_at,
                round, round_reasons, status, untouched_threads, requested, rerequested_at,
                listed, viewer, fingerprint, updated_at, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
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
                serde_json::to_string(&r.round_reasons).unwrap(),
                r.status.as_str(),
                r.untouched_threads,
                r.requested,
                r.rerequested_at,
                r.listed,
                r.viewer,
                r.fingerprint,
                r.review.updated_at,
                r.fetched_at,
            ],
        )
        .unwrap();
}

/// A database at schema `version` as that release left it, holding a review
/// with a thread, a viewed mark and v1 AI rows (findings without a side).
/// Written through the store's own code on the unmigrated connection, so
/// the formats are the real ones.
fn old_database(path: &Path, version: usize) -> (ReviewRecord, ThreadId) {
    let mut conn = Connection::open(path).unwrap();
    let tx = conn.transaction().unwrap();
    for step in &MIGRATIONS[..version] {
        tx.execute_batch(step).unwrap();
    }
    tx.pragma_update(None, "user_version", version).unwrap();
    tx.commit().unwrap();
    let store = Store::over(conn);
    let mut r = record();
    r.last_reviewed_sha = Some("h0".into());
    r.last_reviewed_at = Some(at(9, 8));
    insert_old_review(&store, &r);
    let t = thread(
        "t1",
        ME,
        "h1",
        vec![note("n1", ME, at(10, 1)), note("n2", AUTHOR, at(10, 2))],
    );
    let t1 = ThreadId::from_key(r.id, "t1");
    if version >= 12 {
        store.upsert_threads(r.id, &[t], ME).unwrap();
    } else {
        upsert_threads_before_v12(&store, r.id, &[t], ME);
    }
    if version >= 2 {
        store
            .set_viewed(r.id, "src/cache.rs", "blob1", at(10, 3))
            .unwrap();
    }
    let conn = store.conn();
    conn.execute(
        "INSERT INTO findings (id, review_id, round, path, line, severity, title, body)
         VALUES (?1, ?2, 1, 'src/cache.rs', 10, 'high', 'Old finding', 'From v1')",
        params![crate::ai::FindingId::new().to_string(), r.id.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO checklist (id, review_id, round, text, checked)
         VALUES (?1, ?2, 1, 'Look at the TTL', 1)",
        params![crate::ai::ChecklistId::new().to_string(), r.id.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO verdicts (thread_id, round, ai_label, ai_reason)
         VALUES (?1, 1, 'addressed', 'Fixed in h1')",
        params![t1.to_string()],
    )
    .unwrap();
    (r, t1)
}

/// `Store::upsert_threads` as a database before migration 12 takes it: the
/// columns without `resolvable`.
fn upsert_threads_before_v12(store: &Store, review: ReviewId, threads: &[Thread], me: &str) {
    let conn = store.conn();
    for t in threads {
        let id = ThreadId::from_key(review, &t.id).to_string();
        conn.execute(
            "INSERT INTO threads (id, review_id, provider_thread_id, author, created_at,
                resolved, outdated, anchor_json, mine)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET resolved = excluded.resolved,
                outdated = excluded.outdated, mine = excluded.mine",
            params![
                id,
                review.to_string(),
                t.id,
                t.author,
                t.created_at,
                t.resolved,
                t.outdated,
                t.anchor.as_ref().map(|a| serde_json::to_string(a).unwrap()),
                t.is_mine(me),
            ],
        )
        .unwrap();
        for n in &t.notes {
            conn.execute(
                "INSERT OR IGNORE INTO notes (id, thread_id, author, body, created_at, system)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![n.id, id, n.author, n.body, n.created_at, n.system],
            )
            .unwrap();
        }
    }
}

#[test]
fn upgrades_every_older_database_to_the_latest_keeping_its_data() {
    use crate::ai::store as ai_store;
    use crate::ai::{Severity, VerdictLabel};
    use crate::provider::Side;

    for version in 1..MIGRATIONS.len() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argus.db");
        let (r, t1) = old_database(&path, version);

        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.schema_version().unwrap() as usize,
            MIGRATIONS.len(),
            "from v{version}"
        );
        // The review, its watermark, thread and notes as they were.
        assert_eq!(store.review(r.id).unwrap(), Some(r.clone()), "v{version}");
        let threads = store.threads(r.id).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, t1);
        assert_eq!(threads[0].thread.notes.len(), 2);
        assert!(threads[0].mine);
        // A line thread is resolvable on both platforms (migration 12).
        assert!(threads[0].thread.resolvable, "v{version}");
        if version >= 2 {
            assert_eq!(
                store.viewed(r.id).unwrap(),
                HashMap::from([("src/cache.rs".to_string(), "blob1".to_string())])
            );
        }
        // v1 AI rows read back; a finding gets the new side's default.
        let findings = ai_store::findings(&store, r.id).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            (findings[0].side, findings[0].severity, findings[0].line),
            (Side::New, Severity::High, 10)
        );
        assert_eq!(findings[0].title, "Old finding");
        // No repeats before migration 9: nothing set aside.
        assert_eq!(findings[0].repeat_of, None);
        assert!(ai_store::skipped_findings(&store, r.id).unwrap().is_empty());
        assert_eq!(ai_store::run(&store, r.id).unwrap(), None);
        let checklist = ai_store::checklist(&store, r.id).unwrap();
        assert_eq!(checklist.len(), 1);
        assert!(checklist[0].checked);
        let verdicts = ai_store::verdicts(&store, r.id).unwrap();
        assert_eq!(verdicts[0].ai_label, Some(VerdictLabel::Addressed));
        // No run yet: the AI pass starts from scratch for this review.
        assert!(ai_store::run(&store, r.id).unwrap().is_none());
        // The disk cache (migration 15) starts empty and takes entries.
        assert_eq!(store.cache_stats().unwrap().entries, 0, "v{version}");
        store.cache_put("k", "file", b"text", 1 << 20).unwrap();
        assert_eq!(store.cache_get("k").unwrap().as_deref(), Some(&b"text"[..]));
        // New writes land in the upgraded schema.
        store
            .set_viewed(r.id, "src/other.rs", "blob2", at(11, 1))
            .unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        let expected = if version >= 2 { 2 } else { 1 };
        assert_eq!(reopened.viewed(r.id).unwrap().len(), expected);
    }
}

#[test]
fn a_failed_migration_step_leaves_the_database_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argus.db");
    let (r, _) = old_database(&path, 4);
    // Something else already made the table step 5 creates: that step
    // fails, and its transaction leaves version 4 and the data intact.
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE ai_runs (x INTEGER)")
        .unwrap();
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
    let side_columns: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('findings') WHERE name = 'side'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(side_columns, 0);
    let reviews: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM reviews WHERE id = ?1",
            [r.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reviews, 1);
}

#[test]
fn a_transaction_reads_and_writes_in_one_go() {
    // Read-modify-write goes through the transaction's connection, never
    // the store's own methods (which would wait on the held lock).
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();
    store.set_watermark(r.id, "h1", at(11, 8)).unwrap();
    let next = store
        .transaction(|conn| {
            let mut cur = read_review(conn, r.id)?.ok_or(CoreError::NotFound)?;
            // Sees what is stored now, not `r`.
            assert_eq!(cur.last_reviewed_sha.as_deref(), Some("h1"));
            cur.approved_at = Some(at(11, 9));
            write_review(conn, &cur, WriteColumns::All)?;
            Ok(cur)
        })
        .unwrap();
    assert_eq!(store.review(r.id).unwrap(), Some(next.clone()));
    // A failure rolls the whole transaction back.
    let failed: CoreResult<()> = store.transaction(|conn| {
        let mut cur = read_review(conn, r.id)?.ok_or(CoreError::NotFound)?;
        cur.approved_at = None;
        write_review(conn, &cur, WriteColumns::All)?;
        Err(CoreError::NotFound)
    });
    assert!(matches!(failed, Err(CoreError::NotFound)));
    assert_eq!(store.review(r.id).unwrap(), Some(next));
    store.set_listed(r.id, false).unwrap();
    assert!(!store.review(r.id).unwrap().unwrap().listed);
}

#[test]
fn a_locked_database_is_waited_on_not_failed() {
    // Another connection (a second Argus, a backup tool) holding a lock
    // makes a write wait up to the busy timeout instead of failing at once.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argus.db")).unwrap();
    let ms: i64 = store
        .conn()
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ms, 5000);
}

#[test]
fn a_thread_the_user_replied_in_is_stored_as_theirs() {
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();
    let replied = thread(
        "601",
        AUTHOR,
        "h1",
        vec![note("601", AUTHOR, at(10, 12)), note("602", ME, at(10, 13))],
    );
    let mut resolve = note("612", ME, at(10, 15));
    resolve.system = true;
    let resolved = thread(
        "611",
        AUTHOR,
        "h1",
        vec![note("611", AUTHOR, at(10, 14)), resolve],
    );
    store
        .upsert_threads(r.id, &[replied, resolved], ME)
        .unwrap();
    let got = store.threads(r.id).unwrap();
    assert!(got[0].mine && !got[0].started_by_me);
    // Only a system note of the user's: not theirs.
    assert!(!got[1].mine && !got[1].started_by_me);
}

#[test]
fn upgrading_to_v8_makes_threads_the_user_replied_in_theirs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argus.db");
    let (r, _) = old_database(&path, 7);
    {
        let store = Store::over(Connection::open(&path).unwrap());
        let replied = thread(
            "t2",
            AUTHOR,
            "h1",
            vec![
                note("n3", AUTHOR, at(10, 3)),
                note("n4", "Rev-Fixture", at(10, 4)),
            ],
        );
        let mut resolve = note("n6", ME, at(10, 6));
        resolve.system = true;
        let resolved = thread(
            "t3",
            AUTHOR,
            "h1",
            vec![note("n5", AUTHOR, at(10, 5)), resolve],
        );
        let mine = thread(
            "t1",
            ME,
            "h1",
            vec![note("n1", ME, at(10, 1)), note("n2", AUTHOR, at(10, 2))],
        );
        upsert_threads_before_v12(&store, r.id, &[mine, replied, resolved], ME);
        // As v7 stored them: only a thread the user started was theirs.
        store
            .conn()
            .execute(
                "UPDATE threads SET mine = (lower(author) = lower(?1))",
                [ME],
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    let mine: HashMap<String, (bool, bool)> = store
        .threads(r.id)
        .unwrap()
        .into_iter()
        .map(|t| (t.thread.id, (t.mine, t.started_by_me)))
        .collect();
    assert_eq!(mine["t1"], (true, true));
    assert_eq!(mine["t2"], (true, false));
    assert_eq!(mine["t3"], (false, false));
}

#[test]
fn upgrading_to_v12_marks_line_threads_resolvable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argus.db");
    let (r, t1) = old_database(&path, 11);
    {
        let store = Store::over(Connection::open(&path).unwrap());
        let mut general = thread("t2", AUTHOR, "h1", vec![note("n3", AUTHOR, at(10, 3))]);
        general.anchor = None;
        upsert_threads_before_v12(&store, r.id, &[general], ME);
    }
    let store = Store::open(&path).unwrap();
    let threads = store.threads(r.id).unwrap();
    let resolvable: Vec<(ThreadId, bool)> = threads
        .iter()
        .map(|t| (t.thread_id, t.thread.resolvable))
        .collect();
    // The line thread is; one without an anchor waits for its next fetch.
    assert_eq!(
        resolvable,
        [(t1, true), (ThreadId::from_key(r.id, "t2"), false)]
    );
    // A fetch writes the platform's answer.
    let mut general = threads[1].thread.clone();
    general.resolvable = true;
    store
        .upsert_threads(r.id, &[threads[0].thread.clone(), general], ME)
        .unwrap();
    assert!(store.threads(r.id).unwrap()[1].thread.resolvable);
}

#[test]
fn keeps_the_users_platform_review_state() {
    let store = Store::in_memory().unwrap();
    let mut r = record();
    assert_eq!(r.my_review_state, MyReviewState::NotReviewer);
    r.my_review_state = MyReviewState::RequestedChanges;
    r.my_reviewed_at = Some(at(10, 12));
    r.my_reviewed_sha = Some("h1".into());
    store.upsert_review(&r).unwrap();
    assert_eq!(store.review(r.id).unwrap(), Some(r.clone()));
    // A database from before step 13 reads as "not a reviewer" until the
    // next fetch says otherwise (see the upgrade test for every version).
    r.my_review_state = MyReviewState::AwaitingReview;
    store.upsert_review(&r).unwrap();
    assert_eq!(
        store.review(r.id).unwrap().unwrap().my_review_state,
        MyReviewState::AwaitingReview
    );
}

#[test]
fn marks_a_set_of_files_viewed_at_once() {
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();
    store.set_viewed(r.id, "a.rs", "old", at(10, 1)).unwrap();
    let files = HashMap::from([
        ("a.rs".to_string(), "new".to_string()),
        ("b.rs".to_string(), "b1".to_string()),
    ]);
    store.set_viewed_all(r.id, &files, at(10, 2)).unwrap();
    assert_eq!(store.viewed(r.id).unwrap(), files);
}
