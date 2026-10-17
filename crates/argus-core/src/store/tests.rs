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
    let store = Store {
        conn: Mutex::new(conn),
        refreshing: Mutex::new(()),
    };
    let mut r = record();
    r.last_reviewed_sha = Some("h0".into());
    r.last_reviewed_at = Some(at(9, 8));
    store.upsert_review(&r).unwrap();
    let t = thread(
        "t1",
        ME,
        "h1",
        vec![note("n1", ME, at(10, 1)), note("n2", AUTHOR, at(10, 2))],
    );
    store.upsert_threads(r.id, &[t], ME).unwrap();
    let t1 = ThreadId::from_key(r.id, "t1");
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
        let checklist = ai_store::checklist(&store, r.id).unwrap();
        assert_eq!(checklist.len(), 1);
        assert!(checklist[0].checked);
        let verdicts = ai_store::verdicts(&store, r.id).unwrap();
        assert_eq!(verdicts[0].ai_label, Some(VerdictLabel::Addressed));
        // No run yet: the AI pass starts from scratch for this review.
        assert!(ai_store::run(&store, r.id).unwrap().is_none());
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
fn update_review_reads_and_writes_in_one_go() {
    let store = Store::in_memory().unwrap();
    let r = record();
    store.upsert_review(&r).unwrap();
    store.set_watermark(r.id, "h1", at(11, 8)).unwrap();
    let next = store
        .update_review(r.id, |mut cur| {
            // Sees what is stored now, not `r`.
            assert_eq!(cur.last_reviewed_sha.as_deref(), Some("h1"));
            cur.approved_at = Some(at(11, 9));
            Ok(cur)
        })
        .unwrap();
    assert_eq!(store.review(r.id).unwrap(), Some(next));
    assert!(matches!(
        store.update_review(crate::ids::ReviewId::new(), Ok),
        Err(CoreError::NotFound)
    ));
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
