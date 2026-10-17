//! Tests over the AI pass' moving parts: the trigger rule, the store, the
//! scheduler, the Core API, and real spawns of fake agent CLIs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::adapter::{self, AgentError, Invocation, OutputKind};
use super::job::{self, JobContext};
use super::model::{AiRunStatus, Severity, VerdictLabel};
use super::output::{RawFinding, RunOutput};
use super::runtime::{AiRuntime, Executor};
use super::store::{self as ai_store, RunRow};
use super::*;
use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::post::{PostError, PostErrorKind};
use crate::provider::{
    ChangedFile, DiffRange, FileDiff, FileStatus, ProviderId, ReviewDetail, ReviewProvider,
    ReviewRef, Side, Thread,
};
use crate::review::{advance, ReviewStatus};
use crate::settings::Settings;
use crate::store::{ReviewRecord, Store};
use crate::testutil::{at, detail, note, review_ref, thread, AUTHOR, ME};
use crate::{Core, RefreshReport};

// ---------------------------------------------------------------- fixtures

fn record(number: u64) -> ReviewRecord {
    let mut d = detail("h1");
    d.review = review_ref(number);
    advance(None, &d, &[], ME, true, at(10, 10))
}

fn changed(path: &str) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        old_path: None,
        status: FileStatus::Modified,
        additions: 1,
        deletions: 1,
        patch: Some("@@ -1,2 +1,2 @@\n-let ttl = 10;\n+let ttl = 0;\n ctx".into()),
        content_sha: "c1".into(),
        binary: false,
        too_large: false,
    }
}

/// Serves a fixed diff for any range, and records the ranges asked for.
struct FakeProvider {
    files: Vec<ChangedFile>,
    asked: Mutex<Vec<DiffRange>>,
}

impl FakeProvider {
    fn new(paths: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            files: paths.iter().map(|p| changed(p)).collect(),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl ReviewProvider for FakeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        Ok(ME.into())
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        Ok(Vec::new())
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        Err(CoreError::NotFound)
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(Vec::new())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, _: &str) -> CoreResult<String> {
        Err(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        self.asked.lock().unwrap().push(range.clone());
        Ok(FileDiff {
            base_sha: "base".into(),
            head_sha: "h1".into(),
            files: self.files.clone(),
            truncated: false,
        })
    }
}

fn ctx(settings: Settings, provider: Arc<dyn ReviewProvider>) -> JobContext {
    JobContext {
        settings,
        provider,
        timeout: Duration::from_secs(20),
        child: Default::default(),
    }
}

fn finding(path: &str, line: u32, severity: Severity, title: &str) -> RawFinding {
    RawFinding {
        path: path.into(),
        line,
        side: Side::New,
        severity,
        title: title.into(),
        body: "why".into(),
    }
}

fn run_row(status: AiRunStatus, round: u32, last_done: Option<u32>, interrupted: bool) -> RunRow {
    RunRow {
        review_id: ReviewId::from_key("x"),
        status,
        round,
        adapter: "claude".into(),
        error: None,
        interrupted,
        started_at: None,
        finished_at: None,
        last_done_round: last_done,
        dropped_findings: 0,
        dropped_verdicts: 0,
    }
}

// ---------------------------------------------------------------- trigger

#[test]
fn trigger_rule() {
    let r1 = record(1);
    // New item: never ran.
    assert!(needs_run(&r1, None));
    // Same round, done: no rerun.
    assert!(!needs_run(
        &r1,
        Some(&run_row(AiRunStatus::Done, 1, Some(1), false))
    ));
    // New round past the last successful run.
    let mut r2 = r1.clone();
    r2.round = 2;
    r2.status = ReviewStatus::NeedsReReview;
    assert!(needs_run(
        &r2,
        Some(&run_row(AiRunStatus::Done, 1, Some(1), false))
    ));
    // A failure is not retried in its own round, but a new round runs.
    assert!(!needs_run(
        &r2,
        Some(&run_row(AiRunStatus::Failed, 2, Some(1), false))
    ));
    assert!(needs_run(
        &r2,
        Some(&run_row(AiRunStatus::Failed, 1, Some(1), false))
    ));
    // Interrupted by a restart: retried.
    assert!(needs_run(
        &r2,
        Some(&run_row(AiRunStatus::Failed, 2, Some(1), true))
    ));
    // Running: never doubled.
    assert!(!needs_run(
        &r1,
        Some(&run_row(AiRunStatus::Running, 1, None, false))
    ));
    // Closed, merged, approved: nothing.
    let mut closed = r1.clone();
    closed.state = crate::provider::ReviewState::Merged;
    assert!(!needs_run(&closed, None));
    let mut approved = r1.clone();
    approved.status = ReviewStatus::Approved;
    assert!(!needs_run(&approved, None));
}

// ---------------------------------------------------------------- store

#[test]
fn migration_5_adds_ai_runs_and_finding_side() {
    let store = Store::in_memory().unwrap();
    assert!(store.schema_version().unwrap() >= 5);
    let conn = store.conn();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'ai_runs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let side: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('findings') WHERE name = 'side'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(side, 1);
}

#[test]
fn store_round_trips_findings_checklist_verdicts_and_overrides() {
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let mine = thread("t1", ME, "h1", vec![note("n1", ME, at(1, 1))]);
    let theirs = thread("t2", AUTHOR, "h1", vec![note("n2", AUTHOR, at(1, 2))]);
    store.upsert_threads(r.id, &[mine, theirs], ME).unwrap();
    let t1 = ThreadId::from_key(r.id, "t1");
    let t2 = ThreadId::from_key(r.id, "t2");

    // Round 1.
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 11)).unwrap();
    let out1 = RunOutput {
        findings: vec![
            finding("src/b.rs", 9, Severity::Low, "Typo"),
            finding("src/a.rs", 3, Severity::High, "Off by one"),
            finding("src/a.rs", 1, Severity::Medium, "Leak"),
        ],
        checklist: vec!["Is a TTL of 0 intended?".into(), "Who calls this?".into()],
        verdicts: vec![(t1, VerdictLabel::Untouched, "nothing changed".into())],
        dropped_findings: 2,
        dropped_verdicts: 1,
        ..RunOutput::default()
    };
    ai_store::save_done(&store, r.id, 1, &out1, at(10, 12)).unwrap();
    let run = ai_store::run(&store, r.id).unwrap().unwrap();
    assert_eq!(run.status, AiRunStatus::Done);
    assert_eq!(run.last_done_round, Some(1));
    assert_eq!((run.dropped_findings, run.dropped_verdicts), (2, 1));

    let fs = ai_store::findings(&store, r.id).unwrap();
    let order: Vec<_> = fs
        .iter()
        .map(|f| (f.severity, f.path.as_str(), f.line))
        .collect();
    assert_eq!(
        order,
        [
            (Severity::High, "src/a.rs", 3),
            (Severity::Medium, "src/a.rs", 1),
            (Severity::Low, "src/b.rs", 9)
        ]
    );
    assert_eq!(fs[0].side, Side::New);
    let dismissed = ai_store::set_dismissed(&store, fs[0].id, true).unwrap();
    assert!(dismissed.dismissed);

    let items = ai_store::checklist(&store, r.id).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].text, "Is a TTL of 0 intended?");
    assert!(
        ai_store::set_checked(&store, items[0].id, true)
            .unwrap()
            .checked
    );

    // Rerun of round 1: same items keep their marks.
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 13)).unwrap();
    ai_store::save_done(&store, r.id, 1, &out1, at(10, 14)).unwrap();
    let fs = ai_store::findings(&store, r.id).unwrap();
    assert_eq!(fs.len(), 3);
    assert!(fs[0].dismissed && !fs[1].dismissed);
    assert!(ai_store::checklist(&store, r.id).unwrap()[0].checked);

    // Round 2: round-1 findings stay, with their round.
    let out2 = RunOutput {
        findings: vec![finding("src/a.rs", 3, Severity::High, "Still off by one")],
        checklist: vec!["Check the migration".into()],
        verdicts: vec![(t1, VerdictLabel::Addressed, "fixed".into())],
        ..RunOutput::default()
    };
    ai_store::mark_running(&store, r.id, 2, "h2", "claude", at(11, 1)).unwrap();
    ai_store::save_done(&store, r.id, 2, &out2, at(11, 2)).unwrap();
    let fs = ai_store::findings(&store, r.id).unwrap();
    assert_eq!(fs.len(), 4);
    assert_eq!((fs[0].round, fs[0].title.as_str()), (2, "Still off by one"));
    assert_eq!(fs[1].round, 1);
    let items = ai_store::checklist(&store, r.id).unwrap();
    assert_eq!(items[0].round, 2);
    assert_eq!(items.len(), 3);

    // Verdicts: the latest round's per thread; the override is kept apart.
    let vs = ai_store::verdicts(&store, r.id).unwrap();
    assert_eq!(vs.len(), 1);
    assert_eq!(
        (vs[0].round, vs[0].ai_label),
        (2, Some(VerdictLabel::Addressed))
    );
    let v = ai_store::set_override(&store, t1, Some(VerdictLabel::Partly)).unwrap();
    assert_eq!(v.ai_label, Some(VerdictLabel::Addressed));
    assert_eq!(v.user_override, Some(VerdictLabel::Partly));
    assert_eq!(v.label, Some(VerdictLabel::Partly));
    // A rerun of the round keeps the override.
    ai_store::save_done(&store, r.id, 2, &out2, at(11, 3)).unwrap();
    let vs = ai_store::verdicts(&store, r.id).unwrap();
    assert_eq!(vs[0].user_override, Some(VerdictLabel::Partly));
    let cleared = ai_store::set_override(&store, t1, None).unwrap();
    assert_eq!(cleared.label, Some(VerdictLabel::Addressed));
    // A thread with no AI verdict gets one at the review's round.
    let v2 = ai_store::set_override(&store, t2, Some(VerdictLabel::Untouched)).unwrap();
    assert_eq!((v2.round, v2.ai_label), (r.round, None));
    assert!(matches!(
        ai_store::set_override(&store, ThreadId::from_key(r.id, "nope"), None),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        ai_store::set_dismissed(&store, FindingId::new(), true),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn migration_6_stores_the_diff_a_finding_was_read_at() {
    let store = Store::in_memory().unwrap();
    assert!(store.schema_version().unwrap() >= 6);
    let r = record(12);
    store.upsert_review(&r).unwrap();
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 11)).unwrap();
    let out = RunOutput {
        findings: vec![finding("src/a.rs", 3, Severity::High, "Off by one")],
        head_sha: Some("h1".into()),
        base_sha: Some("b0".into()),
        ..RunOutput::default()
    };
    ai_store::save_done(&store, r.id, 1, &out, at(10, 12)).unwrap();
    let f = &ai_store::findings(&store, r.id).unwrap()[0];
    assert_eq!(
        ai_store::finding_shas(&store, f.id).unwrap(),
        (Some("h1".to_string()), Some("b0".to_string()))
    );
    // A row from before the migration has neither.
    store
        .conn()
        .execute(
            "INSERT INTO findings (id, review_id, round, path, line, severity, title, body)
             VALUES (?1, ?2, 1, 'src/a.rs', 1, 'low', 'Old', '')",
            rusqlite::params![FindingId::new().to_string(), r.id.to_string()],
        )
        .unwrap();
    let old = ai_store::findings(&store, r.id)
        .unwrap()
        .into_iter()
        .find(|f| f.title == "Old")
        .unwrap();
    assert_eq!(
        ai_store::finding_shas(&store, old.id).unwrap(),
        (None, None)
    );
}

#[test]
fn an_override_survives_a_newer_round() {
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    store
        .upsert_threads(
            r.id,
            &[thread("t1", ME, "h1", vec![note("n1", ME, at(1, 1))])],
            ME,
        )
        .unwrap();
    let t1 = ThreadId::from_key(r.id, "t1");
    let verdict = |label| RunOutput {
        verdicts: vec![(t1, label, "seen".into())],
        ..RunOutput::default()
    };
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 1)).unwrap();
    ai_store::save_done(
        &store,
        r.id,
        1,
        &verdict(VerdictLabel::Untouched),
        at(10, 2),
    )
    .unwrap();
    ai_store::set_override(&store, t1, Some(VerdictLabel::Addressed)).unwrap();

    ai_store::mark_running(&store, r.id, 2, "h2", "claude", at(11, 1)).unwrap();
    ai_store::save_done(&store, r.id, 2, &verdict(VerdictLabel::Partly), at(11, 2)).unwrap();
    let vs = ai_store::verdicts(&store, r.id).unwrap();
    assert_eq!(vs.len(), 1);
    assert_eq!(vs[0].round, 2);
    assert_eq!(vs[0].ai_label, Some(VerdictLabel::Partly));
    assert_eq!(vs[0].user_override, Some(VerdictLabel::Addressed));
    assert_eq!(vs[0].label, Some(VerdictLabel::Addressed));
}

#[test]
fn a_rerun_drops_stale_verdicts_and_never_repeats_a_posted_finding() {
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let t = |id: &str| thread(id, ME, "h1", vec![note(id, ME, at(1, 1))]);
    store.upsert_threads(r.id, &[t("t1"), t("t2")], ME).unwrap();
    let (t1, t2) = (
        ThreadId::from_key(r.id, "t1"),
        ThreadId::from_key(r.id, "t2"),
    );
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 1)).unwrap();
    let first = RunOutput {
        findings: vec![finding("src/a.rs", 3, Severity::High, "Off by one")],
        verdicts: vec![
            (t1, VerdictLabel::Untouched, "a".into()),
            (t2, VerdictLabel::Partly, "b".into()),
        ],
        ..RunOutput::default()
    };
    ai_store::save_done(&store, r.id, 1, &first, at(10, 2)).unwrap();
    let posted = ai_store::findings(&store, r.id).unwrap()[0].id;
    ai_store::mark_posted(&store, posted, "u#1", at(10, 3)).unwrap();

    // The same round again: the posted finding comes back, t2 is left out.
    let rerun = RunOutput {
        findings: vec![
            finding("src/a.rs", 3, Severity::High, "Off by one"),
            finding("src/a.rs", 1, Severity::Low, "New one"),
        ],
        verdicts: vec![(t1, VerdictLabel::Addressed, "c".into())],
        ..RunOutput::default()
    };
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 4)).unwrap();
    ai_store::save_done(&store, r.id, 1, &rerun, at(10, 5)).unwrap();
    let fs = ai_store::findings(&store, r.id).unwrap();
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert_eq!(fs.iter().filter(|f| f.title == "Off by one").count(), 1);
    assert!(fs
        .iter()
        .find(|f| f.title == "Off by one")
        .unwrap()
        .posted_at
        .is_some());
    let vs = ai_store::verdicts(&store, r.id).unwrap();
    assert_eq!(vs.len(), 1, "{vs:?}");
    assert_eq!(
        (vs[0].thread_id, vs[0].ai_label),
        (t1, Some(VerdictLabel::Addressed))
    );

    // A stale AI half goes, a user's override on it stays.
    ai_store::set_override(&store, t1, Some(VerdictLabel::Partly)).unwrap();
    ai_store::save_done(&store, r.id, 1, &RunOutput::default(), at(10, 6)).unwrap();
    let vs = ai_store::verdicts(&store, r.id).unwrap();
    assert_eq!(vs.len(), 1);
    assert_eq!(
        (vs[0].ai_label, vs[0].user_override),
        (None, Some(VerdictLabel::Partly))
    );
}

#[test]
fn restart_turns_running_into_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argus.db");
    let r = record(12);
    {
        let store = Store::open(&db).unwrap();
        store.upsert_review(&r).unwrap();
        ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 11)).unwrap();
    }
    let core = Core::open(&db, &dir.path().join("settings.json")).unwrap();
    let state = core.ai_state(r.id).unwrap();
    assert_eq!(state.status, AiRunStatus::Failed);
    assert_eq!(state.error.as_deref(), Some(ai_store::INTERRUPTED));
    assert!(state.finished_at.is_some());
    // And it is retried on the next refresh.
    let run = ai_store::run(&core.store, r.id).unwrap().unwrap();
    assert!(run.interrupted);
    assert!(needs_run(&r, Some(&run)));
}

// ---------------------------------------------------------------- runtime

/// An executor that blocks until the gate opens (or it is cancelled) and
/// records how many runs overlap, overall and per review.
#[derive(Default)]
struct Probe {
    current: AtomicUsize,
    max: AtomicUsize,
    starts: AtomicUsize,
    per_review: Mutex<HashMap<ReviewId, usize>>,
    overlap_same_review: AtomicBool,
    gate: Mutex<bool>,
    opened: Condvar,
}

impl Probe {
    fn executor(self: &Arc<Self>) -> Executor {
        let p = Arc::clone(self);
        Arc::new(
            move |_store: &Store, rec: &ReviewRecord, _ctx: &JobContext, cancel: &AtomicBool| {
                p.starts.fetch_add(1, Ordering::SeqCst);
                {
                    let mut m = p.per_review.lock().unwrap();
                    let n = m.entry(rec.id).or_default();
                    *n += 1;
                    if *n > 1 {
                        p.overlap_same_review.store(true, Ordering::SeqCst);
                    }
                }
                let now = p.current.fetch_add(1, Ordering::SeqCst) + 1;
                p.max.fetch_max(now, Ordering::SeqCst);
                let mut open = p.gate.lock().unwrap();
                while !*open && !cancel.load(Ordering::Relaxed) {
                    open = p
                        .opened
                        .wait_timeout(open, Duration::from_millis(20))
                        .unwrap()
                        .0;
                }
                drop(open);
                p.current.fetch_sub(1, Ordering::SeqCst);
                *p.per_review.lock().unwrap().get_mut(&rec.id).unwrap() -= 1;
                Ok(RunOutput {
                    checklist: vec!["probe".into()],
                    ..RunOutput::default()
                })
            },
        )
    }

    fn into_executor(self) -> Executor {
        Arc::new(self).executor()
    }

    fn open(&self) {
        *self.gate.lock().unwrap() = true;
        self.opened.notify_all();
    }

    fn wait_starts(&self, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.starts.load(Ordering::SeqCst) < n {
            assert!(Instant::now() < deadline, "runs did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn store_with(numbers: &[u64]) -> (Arc<Store>, Vec<ReviewId>) {
    let store = Arc::new(Store::in_memory().unwrap());
    let ids = numbers
        .iter()
        .map(|n| {
            let r = record(*n);
            store.upsert_review(&r).unwrap();
            r.id
        })
        .collect();
    (store, ids)
}

#[test]
fn at_most_two_run_at_once_and_one_per_review() {
    let (store, ids) = store_with(&[1, 2, 3, 4]);
    let probe = Arc::new(Probe::default());
    let rt = AiRuntime::new(store.clone(), true, probe.executor(), MAX_CONCURRENT);
    let states = Arc::new(Mutex::new(Vec::new()));
    let s2 = states.clone();
    rt.set_callback(Arc::new(move |s| s2.lock().unwrap().push(s.clone())));
    let provider: Arc<dyn ReviewProvider> = FakeProvider::new(&["src/a.rs"]);
    let c = ctx(Settings::default(), provider);
    for id in &ids {
        assert!(rt.enqueue(*id, c.clone()));
    }
    probe.wait_starts(2);
    // The first review again while it runs: queued behind itself.
    assert!(rt.enqueue(ids[0], c.clone()));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(probe.current.load(Ordering::SeqCst), 2);
    assert_eq!(rt.state(ids[0]).unwrap().status, AiRunStatus::Running);
    assert!(rt.state(ids[0]).unwrap().queued);
    assert!(rt.state(ids[3]).unwrap().queued);
    assert!(rt.busy(ids[3]));
    probe.open();
    assert!(rt.wait_idle(Duration::from_secs(5)));
    assert_eq!(probe.max.load(Ordering::SeqCst), 2);
    assert_eq!(probe.starts.load(Ordering::SeqCst), 5);
    assert!(!probe.overlap_same_review.load(Ordering::SeqCst));
    for id in &ids {
        let st = rt.state(*id).unwrap();
        assert_eq!(st.status, AiRunStatus::Done);
        assert!(!st.queued);
        assert_eq!(ai_store::checklist(&store, *id).unwrap()[0].text, "probe");
    }
    let seen = states.lock().unwrap();
    assert!(seen.iter().any(|s| s.queued));
    assert!(seen.iter().any(|s| s.status == AiRunStatus::Running));
    assert!(seen.iter().any(|s| s.status == AiRunStatus::Done));
}

#[test]
fn cancel_stops_running_and_drops_queued_runs() {
    let (store, ids) = store_with(&[1, 2, 3]);
    let probe = Arc::new(Probe::default());
    let rt = AiRuntime::new(store.clone(), true, probe.executor(), MAX_CONCURRENT);
    let c = ctx(Settings::default(), FakeProvider::new(&["src/a.rs"]));
    for id in &ids {
        rt.enqueue(*id, c.clone());
    }
    probe.wait_starts(2);
    rt.cancel(ids[2]); // queued
    assert!(!rt.state(ids[2]).unwrap().queued);
    rt.cancel(ids[0]); // running
    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.state(ids[0]).unwrap().status == AiRunStatus::Running {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let st = rt.state(ids[0]).unwrap();
    assert_eq!(st.status, AiRunStatus::Failed);
    assert_eq!(st.error.as_deref(), Some(CANCELLED));
    // Cancelled output never reaches the store.
    assert!(ai_store::checklist(&store, ids[0]).unwrap().is_empty());
    probe.open();
    assert!(rt.wait_idle(Duration::from_secs(5)));
    assert_eq!(rt.state(ids[1]).unwrap().status, AiRunStatus::Done);
    assert_eq!(rt.state(ids[2]).unwrap().status, AiRunStatus::NotStarted);
}

#[test]
fn nothing_starts_while_the_ai_pass_is_off() {
    let (store, ids) = store_with(&[1]);
    let probe = Arc::new(Probe::default());
    let rt = AiRuntime::new(store, false, probe.executor(), MAX_CONCURRENT);
    let c = ctx(Settings::default(), FakeProvider::new(&["src/a.rs"]));
    assert!(!rt.enqueue(ids[0], c.clone()));
    // A run that reached the queue before the pass was turned off (the
    // race of an enqueue with `set_enabled(false)`) still never starts.
    rt.queue_unchecked(ids[0], c);
    rt.pump();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(probe.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn shutdown_cancels_everything_and_starts_nothing_after() {
    let (store, ids) = store_with(&[1, 2, 3]);
    let probe = Arc::new(Probe::default());
    let rt = AiRuntime::new(store.clone(), true, probe.executor(), MAX_CONCURRENT);
    let c = ctx(Settings::default(), FakeProvider::new(&["src/a.rs"]));
    for id in &ids {
        rt.enqueue(*id, c.clone());
    }
    probe.wait_starts(2);
    let start = Instant::now();
    assert!(rt.shutdown(Duration::from_secs(5)));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!rt.busy(ids[0]) && !rt.busy(ids[2]));
    for id in &ids[..2] {
        let st = rt.state(*id).unwrap();
        assert_eq!(
            (st.status, st.error.as_deref()),
            (AiRunStatus::Failed, Some(CANCELLED))
        );
    }
    assert!(!rt.enqueue(ids[2], c));
    assert_eq!(probe.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn a_secret_in_the_answer_never_reaches_the_store() {
    let (store, ids) = store_with(&[1]);
    let token = format!("ghp_{}", "F1xture".repeat(5));
    let t2 = token.clone();
    let ex: Executor = Arc::new(
        move |_: &Store, _: &ReviewRecord, _: &JobContext, _: &AtomicBool| {
            Ok(RunOutput {
                findings: vec![RawFinding {
                    body: format!("~/.config/gh/hosts.yml holds {t2}"),
                    ..finding("src/a.rs", 1, Severity::High, "Token in a file")
                }],
                checklist: vec![format!("Rotate {t2}")],
                ..RunOutput::default()
            })
        },
    );
    let rt = AiRuntime::new(store.clone(), true, ex, MAX_CONCURRENT);
    rt.enqueue(ids[0], ctx(Settings::default(), FakeProvider::new(&[])));
    assert!(rt.wait_idle(Duration::from_secs(5)));
    let f = &ai_store::findings(&store, ids[0]).unwrap()[0];
    assert_eq!(f.body, "~/.config/gh/hosts.yml holds [redacted]");
    let items = ai_store::checklist(&store, ids[0]).unwrap();
    assert_eq!(items[0].text, "Rotate [redacted]");
    assert!(!format!("{f:?}{items:?}").contains(&token));
}

#[test]
fn the_state_says_whether_the_agent_is_read_only_for_certain() {
    let (store, ids) = store_with(&[1]);
    let rt = AiRuntime::new(
        store,
        true,
        Probe::default().into_executor(),
        MAX_CONCURRENT,
    );
    rt.set_adapter("claude");
    assert!(rt.state(ids[0]).unwrap().read_only_guaranteed);
    rt.set_adapter("auggie");
    assert!(!rt.state(ids[0]).unwrap().read_only_guaranteed);
    rt.set_enabled(false);
    assert!(!rt.state(ids[0]).unwrap().read_only_guaranteed);
}

#[test]
fn a_failed_or_crashed_run_stores_nothing() {
    let (store, ids) = store_with(&[1, 2]);
    let ex: Executor = Arc::new(
        |_: &Store, rec: &ReviewRecord, _: &JobContext, _: &AtomicBool| {
            if rec.review.number == 1 {
                Err("the agent gave no answer".into())
            } else {
                panic!("boom")
            }
        },
    );
    let rt = AiRuntime::new(store.clone(), true, ex, MAX_CONCURRENT);
    let c = ctx(Settings::default(), FakeProvider::new(&[]));
    rt.enqueue(ids[0], c.clone());
    rt.enqueue(ids[1], c);
    assert!(rt.wait_idle(Duration::from_secs(5)));
    let a = rt.state(ids[0]).unwrap();
    assert_eq!(a.status, AiRunStatus::Failed);
    assert_eq!(a.error.as_deref(), Some("the agent gave no answer"));
    assert_eq!(a.round, Some(1));
    let b = rt.state(ids[1]).unwrap();
    assert_eq!(b.error.as_deref(), Some("the AI run crashed"));
    assert!(ai_store::findings(&store, ids[0]).unwrap().is_empty());
}

// ---------------------------------------------------------------- Core API

fn core_with_review() -> (Core, Arc<Probe>, ReviewRecord) {
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let provider: Arc<dyn ReviewProvider> = FakeProvider::new(&["src/a.rs"]);
    let core = Core::with_providers(store, vec![provider]);
    let probe = Arc::new(Probe::default());
    probe.open();
    core.ai.set_executor(probe.executor());
    (core, probe, r)
}

fn report() -> RefreshReport {
    RefreshReport {
        changed: Vec::new(),
        errors: Vec::new(),
        refreshed_at: at(10, 12),
    }
}

#[test]
fn refresh_triggers_new_items_and_new_rounds_only() {
    let (core, probe, r) = core_with_review();
    let events = Arc::new(Mutex::new(Vec::new()));
    let e2 = events.clone();
    core.on_ai_state(move |s| e2.lock().unwrap().push(s.status));

    assert_eq!(core.on_refresh_report(&report()), vec![r.id]);
    assert!(core.ai.wait_idle(Duration::from_secs(5)));
    assert_eq!(core.ai_state(r.id).unwrap().status, AiRunStatus::Done);
    assert_eq!(core.checklist(r.id).unwrap().len(), 1);
    assert!(events.lock().unwrap().contains(&AiRunStatus::Done));

    // Same round: no rerun.
    assert!(core.on_refresh_report(&report()).is_empty());
    assert_eq!(probe.starts.load(Ordering::SeqCst), 1);

    // New round: runs again.
    let mut next = r.clone();
    next.round = 2;
    next.status = ReviewStatus::NeedsReReview;
    core.store.upsert_review(&next).unwrap();
    assert_eq!(core.on_refresh_report(&report()), vec![r.id]);
    assert!(core.ai.wait_idle(Duration::from_secs(5)));
    assert_eq!(probe.starts.load(Ordering::SeqCst), 2);
    assert_eq!(core.ai_state(r.id).unwrap().last_done_round, Some(2));

    // Manual run of the same round still runs.
    let st = core.run_ai(r.id).unwrap();
    assert!(st.queued || st.status == AiRunStatus::Running || st.status == AiRunStatus::Done);
    assert!(core.ai.wait_idle(Duration::from_secs(5)));
    assert_eq!(probe.starts.load(Ordering::SeqCst), 3);
    assert!(matches!(
        core.run_ai(ReviewId::from_key("missing")),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn ai_disabled_runs_and_queues_nothing() {
    let (core, probe, r) = core_with_review();
    assert_eq!(core.on_refresh_report(&report()), vec![r.id]);
    assert!(core.ai.wait_idle(Duration::from_secs(5)));

    let mut s = core.settings();
    s.ai.enabled = false;
    core.update_settings(s).unwrap();
    let mut next = r.clone();
    next.round = 2;
    core.store.upsert_review(&next).unwrap();
    assert!(core.on_refresh_report(&report()).is_empty());
    assert!(matches!(core.run_ai(r.id), Err(CoreError::InvalidInput(_))));
    assert!(core.findings(r.id).unwrap().is_empty());
    assert!(core.checklist(r.id).unwrap().is_empty());
    assert!(core.verdicts(r.id).unwrap().is_empty());
    let st = core.ai_state(r.id).unwrap();
    assert!(!st.enabled);
    assert_eq!(st.status, AiRunStatus::NotStarted);
    assert_eq!(probe.starts.load(Ordering::SeqCst), 1);
    // The rest of the app still works.
    assert_eq!(core.reviews().unwrap().len(), 1);
}

#[test]
fn core_api_mutations_round_trip() {
    let (core, _probe, r) = core_with_review();
    core.store
        .upsert_threads(
            r.id,
            &[thread("t1", ME, "h1", vec![note("n1", ME, at(1, 1))])],
            ME,
        )
        .unwrap();
    let t1 = ThreadId::from_key(r.id, "t1");
    ai_store::mark_running(&core.store, r.id, 1, "h1", "claude", at(10, 11)).unwrap();
    ai_store::save_done(
        &core.store,
        r.id,
        1,
        &RunOutput {
            findings: vec![finding("src/a.rs", 2, Severity::Medium, "Leak")],
            checklist: vec!["Intended?".into()],
            verdicts: vec![(t1, VerdictLabel::Partly, "half".into())],
            ..RunOutput::default()
        },
        at(10, 12),
    )
    .unwrap();
    let f = &core.findings(r.id).unwrap()[0];
    assert!(core.dismiss_finding(f.id, true).unwrap().dismissed);
    assert!(!core.dismiss_finding(f.id, false).unwrap().dismissed);
    let c = &core.checklist(r.id).unwrap()[0];
    assert!(core.set_checklist_checked(c.id, true).unwrap().checked);
    let v = core
        .set_verdict_override(t1, Some(VerdictLabel::Addressed))
        .unwrap();
    assert_eq!(v.ai_label, Some(VerdictLabel::Partly));
    assert_eq!(v.label, Some(VerdictLabel::Addressed));
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["userOverride"], "addressed");
    assert_eq!(json["aiLabel"], "partly");
    let json = serde_json::to_value(f).unwrap();
    assert_eq!(json["side"], "new");
    assert_eq!(json["severity"], "medium");
}

// ---------------------------------------------------------------- posting

/// A provider that accepts line comments on `src/a.rs` (the `changed`
/// patch) and records them; a body containing "reject" is refused.
#[derive(Default)]
struct Poster {
    posted: Mutex<Vec<crate::post::PlacedComment>>,
}

impl ReviewProvider for Poster {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        Ok(ME.into())
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        Ok(Vec::new())
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        let mut d = detail("h1");
        d.review = review_ref(12);
        Ok(d)
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(Vec::new())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, _: &str) -> CoreResult<String> {
        Err(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, _: &DiffRange) -> CoreResult<FileDiff> {
        Ok(FileDiff {
            base_sha: "base".into(),
            head_sha: "h1".into(),
            files: vec![changed("src/a.rs")],
            truncated: false,
        })
    }
    fn post_comment(
        &self,
        r: &ReviewRef,
        c: &crate::post::PlacedComment,
    ) -> Result<crate::post::PostedComment, PostError> {
        if c.body.contains("reject") {
            return Err(PostError::new(
                PostErrorKind::PostRejected,
                "Poster: refused",
            ));
        }
        let mut posted = self.posted.lock().unwrap();
        posted.push(c.clone());
        Ok(crate::post::PostedComment {
            url: format!("{}#c{}", r.url, posted.len()),
            provider_thread_id: format!("{}", 700 + posted.len()),
            provider_note_id: format!("{}", 700 + posted.len()),
            path: c.path.clone(),
            side: c.side,
            line: c.line_number(),
            start_line: c.start_number(),
            head_sha: c.head_sha.clone(),
        })
    }
}

fn raw(path: &str, line: u32, side: Side, title: &str) -> RawFinding {
    RawFinding {
        side,
        ..finding(path, line, Severity::High, title)
    }
}

/// A core with a stored review, a `Poster` and these findings of round 1.
fn core_with_findings(findings: Vec<RawFinding>) -> (Core, Arc<Poster>, ReviewId) {
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    ai_store::mark_running(&store, r.id, 1, "h1", "claude", at(10, 11)).unwrap();
    ai_store::save_done(
        &store,
        r.id,
        1,
        &RunOutput {
            findings,
            head_sha: Some("h1".into()),
            base_sha: Some("base".into()),
            ..RunOutput::default()
        },
        at(10, 12),
    )
    .unwrap();
    let poster = Arc::new(Poster::default());
    let core = Core::with_providers(store, vec![poster.clone()]);
    core.ai.set_enabled(true);
    (core, poster, r.id)
}

#[test]
fn posting_findings_marks_the_posted_ones_and_reports_the_rest() {
    let (core, poster, id) = core_with_findings(vec![
        raw("src/a.rs", 1, Side::New, "Ttl of zero"),
        raw("src/a.rs", 99, Side::New, "Gone line"),
        raw("src/a.rs", 1, Side::Old, "Old side"),
        raw("src/a.rs", 2, Side::New, "Refused one"),
    ]);
    let by_title = |t: &str| {
        core.findings(id)
            .unwrap()
            .into_iter()
            .find(|f| f.title == t)
            .unwrap()
    };
    let (a, gone, old, refused) = (
        by_title("Ttl of zero"),
        by_title("Gone line"),
        by_title("Old side"),
        by_title("Refused one"),
    );
    let results = core
        .post_findings(
            id,
            vec![
                FindingPost {
                    finding_id: a.id,
                    body: None,
                },
                FindingPost {
                    finding_id: gone.id,
                    body: None,
                },
                FindingPost {
                    finding_id: old.id,
                    body: Some("Edited text".into()),
                },
                FindingPost {
                    finding_id: refused.id,
                    body: Some("please reject".into()),
                },
                FindingPost {
                    finding_id: FindingId::new(),
                    body: None,
                },
            ],
        )
        .unwrap();
    let kinds: Vec<Option<PostErrorKind>> = results
        .iter()
        .map(|r| match &r.outcome {
            FindingPostOutcome::Posted { .. } => None,
            FindingPostOutcome::Failed(e) => Some(e.kind),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            None,
            Some(PostErrorKind::LineNotInDiff),
            None,
            Some(PostErrorKind::PostRejected),
            Some(PostErrorKind::NotFound),
        ]
    );
    assert_eq!(
        results
            .iter()
            .map(|r| r.finding_id)
            .take(4)
            .collect::<Vec<_>>(),
        [a.id, gone.id, old.id, refused.id]
    );

    // What went out: the default text, then the edit; both on their side.
    let posted = poster.posted.lock().unwrap();
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[0].body, "**Ttl of zero**\n\nwhy");
    assert_eq!(
        (posted[1].body.as_str(), posted[1].side),
        ("Edited text", Side::Old)
    );
    drop(posted);

    // Posted ones carry their time and url; the others stay selectable.
    let stored = core.findings(id).unwrap();
    let state = |t: &str| stored.iter().find(|f| f.title == t).unwrap();
    assert!(state("Ttl of zero").posted_at.is_some());
    assert!(state("Ttl of zero")
        .posted_url
        .as_deref()
        .unwrap()
        .ends_with("#c1"));
    assert!(state("Old side").posted_url.is_some());
    for t in ["Gone line", "Refused one"] {
        assert!(state(t).posted_at.is_none() && state(t).posted_url.is_none());
    }

    // A second go: the posted one is refused, the failed one can retry
    // (still refused here), and nothing is posted twice.
    let again = core
        .post_findings(
            id,
            vec![
                FindingPost {
                    finding_id: a.id,
                    body: None,
                },
                FindingPost {
                    finding_id: a.id,
                    body: None,
                },
            ],
        )
        .unwrap();
    assert!(again.iter().all(|r| matches!(
        &r.outcome,
        FindingPostOutcome::Failed(e) if e.kind == PostErrorKind::InvalidInput
    )));
    assert_eq!(poster.posted.lock().unwrap().len(), 2);

    // The wire shape.
    let json = serde_json::to_value(&results[0]).unwrap();
    assert_eq!(json["status"], "posted");
    assert!(json["postedUrl"].is_string() && json["postedAt"].is_string());
    assert_eq!(json["findingId"], a.id.to_string());
    let json = serde_json::to_value(&results[1]).unwrap();
    assert_eq!(
        (json["status"].as_str(), json["kind"].as_str()),
        (Some("failed"), Some("lineNotInDiff"))
    );
    let edit: FindingPost =
        serde_json::from_str(&format!(r#"{{"findingId":"{}"}}"#, a.id)).unwrap();
    assert_eq!(edit.body, None);
}

#[test]
fn a_finding_is_posted_from_the_diff_it_was_read_at() {
    let (core, poster, id) = core_with_findings(vec![
        raw("src/a.rs", 1, Side::New, "At h1"),
        raw("src/a.rs", 1, Side::Old, "Old side"),
    ]);
    let find = |t: &str| {
        core.findings(id)
            .unwrap()
            .into_iter()
            .find(|f| f.title == t)
            .unwrap()
            .id
    };
    let post = |fid| {
        core.post_findings(
            id,
            vec![FindingPost {
                finding_id: fid,
                body: None,
            }],
        )
        .unwrap()
        .remove(0)
        .outcome
    };
    let failed = |o: FindingPostOutcome| match o {
        FindingPostOutcome::Failed(e) => e,
        o => panic!("{o:?}"),
    };
    let set = |fid: FindingId, head: Option<&str>, base: Option<&str>| {
        core.store
            .conn()
            .execute(
                "UPDATE findings SET head_sha = ?2, base_sha = ?3 WHERE id = ?1",
                rusqlite::params![fid.to_string(), head, base],
            )
            .unwrap();
    };

    // Read at an older head that the platform's history no longer leads
    // from: the stored head is what the post API maps from, so it fails
    // instead of landing on today's line 1.
    let at_h1 = find("At h1");
    set(at_h1, Some("h0"), Some("base"));
    let e = failed(post(at_h1));
    assert_eq!(e.kind, PostErrorKind::LineNotInDiff);

    // An old-side finding read against another left side (the diff since
    // the last review) is refused, never placed on the base's old side.
    let old = find("Old side");
    set(old, Some("h1"), Some("h0"));
    assert_eq!(failed(post(old)).kind, PostErrorKind::LineNotInDiff);
    // One stored before the shas were (migration 6): run the pass again.
    set(old, None, None);
    let e = failed(post(old));
    assert_eq!(e.kind, PostErrorKind::LineNotInDiff);
    assert!(e.message.contains("re-run the AI pass"), "{}", e.message);
    assert!(poster.posted.lock().unwrap().is_empty());

    // With the left side it was read at, it goes out.
    set(old, Some("h1"), Some("base"));
    assert!(matches!(post(old), FindingPostOutcome::Posted { .. }));
    // A new-side finding without shas keeps the stored head.
    set(at_h1, None, None);
    assert!(matches!(post(at_h1), FindingPostOutcome::Posted { .. }));
}

#[test]
fn posting_findings_of_another_review_or_nothing_is_safe() {
    let (core, poster, id) = core_with_findings(vec![raw("src/a.rs", 1, Side::New, "x")]);
    assert!(core.post_findings(id, Vec::new()).unwrap().is_empty());
    let err = core.post_findings(ReviewId::new(), Vec::new()).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::NotFound);
    assert!(poster.posted.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------- fake CLIs

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// A fake `claude`: records argv, cwd, stdin and the workspace listing per
/// call, then prints `out<n>.jsonl` (or `out.jsonl`) as its stream.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
n=$(cat "$dir/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/count"
printf '%s\n' "$@" > "$dir/argv$n.txt"
pwd -P > "$dir/cwd$n.txt"
prev=""
for a in "$@"; do
  if [ "$prev" = "--settings" ]; then cp "$a" "$dir/settings$n.json"; fi
  prev="$a"
done
cat > "$dir/stdin$n.txt"
find . -type f > "$dir/ls$n.txt" 2>/dev/null
echo '{"type":"system","subtype":"init","session_id":"fake"}'
echo 'profile noise that is not json'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}'
if [ -f "$dir/out$n.jsonl" ]; then cat "$dir/out$n.jsonl"; else cat "$dir/out.jsonl"; fi
"#;

fn result_line(answer: &str) -> String {
    serde_json::json!({
        "type": "result", "subtype": "success", "is_error": false, "result": answer
    })
    .to_string()
        + "\n"
}

const GOOD_ANSWER: &str = r#"Here you go:
```json
{"findings":[
  {"path":"src/a.rs","line":2,"side":"new","severity":"high","title":"TTL of 0 disables the cache","body":"Every lookup misses."},
  {"path":"src/not-in-diff.rs","line":1,"side":"new","severity":"low","title":"x","body":"y"}
 ],
 "checklist":[{"text":"Is turning the cache off intended?"}],
 "verdicts":[{"threadId":"00000000-0000-0000-0000-000000000000","label":"addressed","reason":"?"}]}
```"#;

fn fake_settings(binary: &Path, adapter_id: &str) -> Settings {
    let mut s = Settings::default();
    s.ai.adapter = adapter_id.into();
    s.ai.binary = Some(binary.to_string_lossy().into_owned());
    s
}

#[test]
#[cfg_attr(windows, ignore)]
fn claude_run_in_a_snapshot_dir() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", FAKE_CLAUDE);
    std::fs::write(bin.path().join("out.jsonl"), result_line(GOOD_ANSWER)).unwrap();
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    store
        .upsert_threads(
            r.id,
            &[thread("t1", ME, "h1", vec![note("n1", ME, at(1, 1))])],
            ME,
        )
        .unwrap();
    let provider = FakeProvider::new(&["src/a.rs"]);
    let c = ctx(fake_settings(&script, "claude"), provider.clone());

    let out = job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    assert_eq!(out.findings.len(), 1);
    assert_eq!(out.findings[0].title, "TTL of 0 disables the cache");
    assert_eq!(out.dropped_findings, 1);
    // Round 1 asks about no thread, so any verdict is dropped.
    assert_eq!(out.dropped_verdicts, 1);
    assert_eq!(out.checklist, vec!["Is turning the cache off intended?"]);
    assert!(matches!(
        provider.asked.lock().unwrap()[0],
        DiffRange::Review { .. }
    ));

    let read = |name: &str| std::fs::read_to_string(bin.path().join(name)).unwrap();
    let argv = read("argv1.txt");
    for flag in [
        "--print",
        "stream-json",
        "--verbose",
        "Read,Grep,Glob",
        "--strict-mcp-config",
    ] {
        assert!(argv.contains(flag), "missing {flag} in {argv}");
    }
    let stdin = read("stdin1.txt");
    assert!(stdin.contains(prompt::FIRST_ROUND_TASK));
    assert!(stdin.contains("+let ttl = 0;"));
    assert!(!stdin.contains("Repository conventions"));
    assert!(!argv.contains("--add-dir"));
    assert!(
        argv.contains("--setting-sources\nuser\n--settings\n"),
        "{argv}"
    );
    let ls = read("ls1.txt");
    assert!(ls.contains("./threads.json"), "{ls}");
    assert!(ls.contains("./diff/src/a.rs.diff"), "{ls}");
    // The temp dir is gone after the run.
    let cwd = read("cwd1.txt");
    assert!(cwd.contains("argus-ai-"));
    assert!(!Path::new(cwd.trim()).exists());
    assert_eq!(read("count").trim(), "1");
}

#[test]
#[cfg_attr(windows, ignore)]
fn claude_runs_in_the_clone_with_its_conventions_in_a_later_round() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", FAKE_CLAUDE);
    let clone = tempfile::tempdir().unwrap();
    std::fs::write(clone.path().join("CLAUDE.md"), "Fixture rule: no unwrap.").unwrap();
    let store = Store::in_memory().unwrap();
    let mut r = record(12);
    r.round = 2;
    r.last_reviewed_sha = Some("h0".into());
    r.last_reviewed_at = Some(at(9, 9));
    r.status = ReviewStatus::NeedsReReview;
    store.upsert_review(&r).unwrap();
    store
        .upsert_threads(
            r.id,
            &[thread("t1", ME, "h0", vec![note("n1", ME, at(1, 1))])],
            ME,
        )
        .unwrap();
    let t1 = ThreadId::from_key(r.id, "t1");
    let answer = format!(
        r#"{{"findings":[],"checklist":[],"verdicts":[{{"threadId":"{t1}","label":"partly","reason":"Only one call site changed."}}]}}"#
    );
    std::fs::write(bin.path().join("out.jsonl"), result_line(&answer)).unwrap();
    let mut settings = fake_settings(&script, "claude");
    settings.clone_paths.insert(
        "github.com/acme-fixture/widgets".into(),
        clone.path().into(),
    );
    let provider = FakeProvider::new(&["src/a.rs"]);
    let c = ctx(settings, provider.clone());

    let out = job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        out.verdicts,
        vec![(
            t1,
            VerdictLabel::Partly,
            "Only one call site changed.".into()
        )]
    );
    assert_eq!(out.dropped_verdicts, 0);
    assert!(matches!(
        &provider.asked.lock().unwrap()[0],
        DiffRange::Compare { from, .. } if from == "h0"
    ));
    let read = |name: &str| std::fs::read_to_string(bin.path().join(name)).unwrap();
    // Run in its own temp dir, the clone given as an extra directory: the
    // checkout's own agent settings never load.
    let cwd = PathBuf::from(read("cwd1.txt").trim());
    assert!(cwd.to_string_lossy().contains("argus-ai-"), "{cwd:?}");
    let canonical_clone = clone.path().canonicalize().unwrap();
    let argv: Vec<String> = read("argv1.txt").lines().map(str::to_string).collect();
    let after = |flag: &str| {
        argv.iter()
            .position(|a| a == flag)
            .map(|i| argv[i + 1].clone())
            .unwrap_or_else(|| panic!("no {flag} in {argv:?}"))
    };
    assert_eq!(PathBuf::from(after("--add-dir")), canonical_clone);
    assert_eq!(after("--setting-sources"), "user");
    let settings: serde_json::Value = serde_json::from_str(&read("settings1.json")).unwrap();
    assert_eq!(settings["disableAllHooks"], true);
    let deny: Vec<String> = settings["permissions"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    // The clone and the run's dir stay readable, their surroundings not,
    // and neither do the clone's git dir and `.env` files.
    assert_eq!(
        settings["permissions"]["blockReadsOutsideWorkingDirectories"],
        true
    );
    for open in [&canonical_clone, &cwd] {
        for rule in [
            format!("Read(/{})", open.display()),
            format!("Read(/{}/**)", open.display()),
        ] {
            assert!(!deny.contains(&rule), "{rule} in {deny:?}");
        }
    }
    let run_dir = format!("Read(/{}/", cwd.display());
    assert!(!deny.iter().any(|r| r.starts_with(&run_dir)), "{deny:?}");
    for secret in [".git/**", "**/.env*"] {
        let rule = format!("Read(/{}/{secret})", canonical_clone.display());
        assert!(deny.contains(&rule), "{rule} not in {deny:?}");
    }
    #[cfg(unix)]
    assert!(
        deny.iter()
            .any(|r| r == "Read(//etc/**)" || r == "Read(//private/etc/**)"),
        "{deny:?}"
    );
    let stdin = read("stdin1.txt");
    assert!(stdin.contains(prompt::LATER_ROUND_TASK));
    assert!(stdin.contains("Fixture rule: no unwrap."));
    assert!(stdin.contains(&format!(
        "local clone of the repository is at {}",
        canonical_clone.display()
    )));
    assert!(stdin.contains(&format!("threadId: {t1}")));
    // Nothing was written into the clone.
    let entries: Vec<_> = std::fs::read_dir(clone.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

/// A later-round review with one open thread of the user's, `t1`, on head
/// `h1`; returns the store, the record and the thread's id.
fn later_round_with_thread() -> (Store, ReviewRecord, ThreadId) {
    let store = Store::in_memory().unwrap();
    let mut r = record(12);
    r.round = 2;
    r.last_reviewed_sha = Some("h0".into());
    r.last_reviewed_at = Some(at(9, 9));
    r.status = ReviewStatus::NeedsReReview;
    store.upsert_review(&r).unwrap();
    store
        .upsert_threads(
            r.id,
            &[thread("t1", ME, "h0", vec![note("n1", ME, at(1, 1))])],
            ME,
        )
        .unwrap();
    let t1 = ThreadId::from_key(r.id, "t1");
    (store, r, t1)
}

fn snapshot_at(sha: &str, lines: &[&str]) -> crate::history::CodeSnapshot {
    crate::history::CodeSnapshot {
        sha: sha.into(),
        path: "src/a.rs".into(),
        first_line: 10,
        lines: lines.iter().map(|l| l.to_string()).collect(),
        anchor_start: Some(10),
        anchor_end: Some(10),
        source: crate::history::SnapshotSource::File,
    }
}

const EMPTY_ANSWER: &str = r#"{"findings":[],"checklist":[],"verdicts":[]}"#;

#[test]
#[cfg_attr(windows, ignore)]
fn a_later_round_prompt_carries_the_cached_then_now_code() {
    use crate::history::cache::{CachedHistory, CachedMapping};
    use crate::history::LineStatus;

    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", FAKE_CLAUDE);
    std::fs::write(bin.path().join("out.jsonl"), result_line(EMPTY_ANSWER)).unwrap();
    let (store, r, t1) = later_round_with_thread();
    store
        .put_history(
            t1,
            "h1",
            &CachedHistory {
                mapping: CachedMapping {
                    status: LineStatus::Located {
                        path: "src/b.rs".into(),
                        start_line: 14,
                        line: 14,
                        moved: true,
                        renamed: true,
                        touched: true,
                    },
                    touched_by: vec!["h1".into()],
                    attributed: true,
                    then_sha: Some("h0".into()),
                    commits: vec!["h1".into()],
                    resolved: false,
                },
                then_snapshot: Some(snapshot_at("h0", &["let then_marker = 1;"])),
                now_snapshot: Some(snapshot_at("h1", &["let now_marker = 2;"])),
                entries: Vec::new(),
            },
        )
        .unwrap();
    let c = ctx(
        fake_settings(&script, "claude"),
        FakeProvider::new(&["src/a.rs"]),
    );

    job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    let stdin = std::fs::read_to_string(bin.path().join("stdin1.txt")).unwrap();
    assert!(
        stdin.contains("Code then:\nlet then_marker = 1;"),
        "{stdin}"
    );
    assert!(
        stdin.contains("Code now (src/b.rs:14):\nlet now_marker = 2;"),
        "{stdin}"
    );
}

/// A repository where the user's comment (on `src/cache.rs:10` at `h0`)
/// is still there at head `h1`: the only commit in between touches another
/// file.
struct HistoryProvider;

impl HistoryProvider {
    const FILE: &'static str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nthe commented line\nl11\n";
}

impl ReviewProvider for HistoryProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        Ok(ME.into())
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        Ok(Vec::new())
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        let mut d = detail("h1");
        d.review = review_ref(12);
        d.commits = ["h0", "h1"]
            .iter()
            .map(|sha| crate::provider::CommitInfo {
                sha: sha.to_string(),
                title: format!("Commit {sha}"),
                author: AUTHOR.into(),
                authored_at: Some(at(10, 9)),
            })
            .collect();
        Ok(d)
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(Vec::new())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, path: &str) -> CoreResult<String> {
        if path == "src/cache.rs" {
            Ok(Self::FILE.into())
        } else {
            Err(CoreError::NotFound)
        }
    }
    fn fetch_files(&self, _: &ReviewRef, _: &DiffRange) -> CoreResult<FileDiff> {
        Ok(FileDiff {
            base_sha: "base".into(),
            head_sha: "h1".into(),
            files: vec![changed("src/other.rs")],
            truncated: false,
        })
    }
    fn commit_diff(&self, _: &ReviewRef, _: &str) -> CoreResult<FileDiff> {
        Ok(FileDiff {
            base_sha: "h0".into(),
            head_sha: "h1".into(),
            files: vec![changed("src/other.rs")],
            truncated: false,
        })
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_run_computes_the_history_of_the_open_threads() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", FAKE_CLAUDE);
    std::fs::write(bin.path().join("out.jsonl"), result_line(EMPTY_ANSWER)).unwrap();
    let (store, r, t1) = later_round_with_thread();
    assert_eq!(store.snapshot_text(t1, "h1").unwrap(), None);
    let c = ctx(fake_settings(&script, "claude"), Arc::new(HistoryProvider));

    job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    let stdin = std::fs::read_to_string(bin.path().join("stdin1.txt")).unwrap();
    assert!(stdin.contains("Code then:"), "{stdin}");
    assert!(stdin.contains("Code now (src/cache.rs:10):"), "{stdin}");
    assert!(stdin.contains("the commented line"), "{stdin}");
    // Cached for the next run (and for the UI) at this head.
    assert!(store.snapshot_text(t1, "h1").unwrap().is_some());
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_failing_history_does_not_fail_the_run() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", FAKE_CLAUDE);
    std::fs::write(bin.path().join("out.jsonl"), result_line(EMPTY_ANSWER)).unwrap();
    let (store, r, t1) = later_round_with_thread();
    // `FakeProvider::fetch_review` fails, so the history cannot be computed.
    let c = ctx(
        fake_settings(&script, "claude"),
        FakeProvider::new(&["src/a.rs"]),
    );

    job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    let stdin = std::fs::read_to_string(bin.path().join("stdin1.txt")).unwrap();
    assert!(stdin.contains(&format!("threadId: {t1}")));
    assert!(!stdin.contains("Code then:") && !stdin.contains("Code now"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn invalid_answer_is_retried_once_then_fails() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", FAKE_CLAUDE);
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let c = ctx(
        fake_settings(&script, "claude"),
        FakeProvider::new(&["src/a.rs"]),
    );

    // Bad, then good: one retry with the repair instruction.
    std::fs::write(
        bin.path().join("out1.jsonl"),
        result_line("I found nothing wrong."),
    )
    .unwrap();
    std::fs::write(
        bin.path().join("out2.jsonl"),
        result_line(r#"{"findings":[],"checklist":[],"verdicts":[]}"#),
    )
    .unwrap();
    let out = job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        out,
        RunOutput {
            head_sha: Some("h1".into()),
            base_sha: Some("base".into()),
            ..RunOutput::default()
        }
    );
    let stdin2 = std::fs::read_to_string(bin.path().join("stdin2.txt")).unwrap();
    assert!(
        stdin2.contains("Your previous answer could not be used: the answer holds no JSON object")
    );

    // Bad twice: failed, and nothing reaches the store through the runtime.
    for f in ["count", "out1.jsonl", "out2.jsonl"] {
        let _ = std::fs::remove_file(bin.path().join(f));
    }
    std::fs::write(
        bin.path().join("out.jsonl"),
        result_line(r#"{"findings":[{"path":"src/a.rs"}],"checklist":[],"verdicts":[]}"#),
    )
    .unwrap();
    let err = job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap_err();
    assert!(
        err.starts_with("the agent's answer was invalid after a retry"),
        "{err}"
    );
    assert!(err.contains("findings[0]"), "{err}");
    assert_eq!(
        std::fs::read_to_string(bin.path().join("count"))
            .unwrap()
            .trim(),
        "2"
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn auggie_takes_an_instruction_file_and_answers_in_text() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(
        bin.path(),
        "auggie",
        r#"#!/bin/sh
dir="$(dirname "$0")"
if [ "$1" = "tools" ]; then
  pwd -P > "$dir/tools-cwd.txt"
  printf 'Integration Tools:\n  \342\234\223 add_todo_fixture   enabled\n  \342\234\223 view   enabled\n'
  exit 0
fi
printf '%s\n' "$@" > "$dir/argv.txt"
while [ $# -gt 0 ]; do
  if [ "$1" = "--instruction-file" ]; then cp "$2" "$dir/prompt.txt"; fi
  shift
done
printf 'Sure.\n```json\n{"findings":[],"checklist":[{"text":"Check the rollout"}],"verdicts":[]}\n```\n'
"#,
    );
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let c = ctx(
        fake_settings(&script, "auggie"),
        FakeProvider::new(&["src/a.rs"]),
    );
    let out = job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap();
    assert_eq!(out.checklist, vec!["Check the rollout"]);
    let argv = std::fs::read_to_string(bin.path().join("argv.txt")).unwrap();
    assert!(argv.starts_with("--print\n--quiet\n--ask\n--dont-save-session\n--workspace-root\n"));
    assert!(argv.contains("--remove-tool=launch-process\n"), "{argv}");
    // What `auggie tools list` named, in the run's own dir, goes too; the
    // read tools stay.
    assert!(argv.contains("--remove-tool=add_todo_fixture\n"), "{argv}");
    assert!(!argv.contains("--remove-tool=view\n"), "{argv}");
    let tools_cwd = std::fs::read_to_string(bin.path().join("tools-cwd.txt")).unwrap();
    assert!(tools_cwd.contains("argus-ai-"), "{tools_cwd}");
    assert!(argv.contains("--instruction-file\n"), "{argv}");
    assert!(!argv.contains("--add-workspace"), "{argv}");
    let prompt = std::fs::read_to_string(bin.path().join("prompt.txt")).unwrap();
    assert!(prompt.contains(prompt::OUTPUT_CONTRACT));
}

fn invocation(program: PathBuf, env: Vec<(std::ffi::OsString, std::ffi::OsString)>) -> Invocation {
    Invocation {
        program,
        args: Vec::new(),
        cwd: std::env::temp_dir(),
        env,
        stdin: Some("prompt".into()),
        timeout: Duration::from_secs(20),
        output: OutputKind::ClaudeStreamJson,
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_agent_env_has_no_tokens() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(
        bin.path(),
        "claude",
        r#"#!/bin/sh
env > "$(dirname "$0")/env.txt"
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
"#,
    );
    let parent = [
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/home/fixture"),
        ("GH_TOKEN", "fixture-gh-token"),
        ("GITLAB_TOKEN", "fixture-gl-token"),
        ("GITHUB_TOKEN", "fixture-gh2-token"),
    ]
    .map(|(k, v)| (k.into(), v.into()));
    let inv = invocation(script, adapter::sanitized_env(parent, None));
    assert_eq!(adapter::run(&inv, &AtomicBool::new(false)).unwrap(), "ok");
    let env = std::fs::read_to_string(bin.path().join("env.txt")).unwrap();
    assert!(!env.contains("fixture-"), "{env}");
    assert!(!env.contains("TOKEN="), "{env}");
    assert!(env.contains("HOME=/home/fixture"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn hanging_agents_are_killed_on_timeout_and_cancel() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(bin.path(), "claude", "#!/bin/sh\nexec sleep 30\n");
    let env = adapter::sanitized_env(std::env::vars_os(), None);
    let mut inv = invocation(script, env);
    inv.timeout = Duration::from_millis(300);
    let start = Instant::now();
    assert_eq!(
        adapter::run(&inv, &AtomicBool::new(false)),
        Err(AgentError::Timeout(Duration::from_millis(300)))
    );
    assert!(start.elapsed() < Duration::from_secs(5));

    inv.timeout = Duration::from_secs(30);
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        c2.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    assert_eq!(adapter::run(&inv, &cancel), Err(AgentError::Cancelled));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
#[cfg_attr(windows, ignore)]
fn agent_failures_have_app_owned_messages() {
    let bin = tempfile::tempdir().unwrap();
    let env = adapter::sanitized_env(std::env::vars_os(), None);
    let exit = write_script(
        bin.path(),
        "exit3",
        "#!/bin/sh\necho secret-ish >&2\nexit 3\n",
    );
    assert_eq!(
        adapter::run(&invocation(exit, env.clone()), &AtomicBool::new(false)),
        Err(AgentError::Exit(Some(3)))
    );
    let failed = write_script(
        bin.path(),
        "failed",
        "#!/bin/sh\necho '{\"type\":\"result\",\"subtype\":\"error_max_turns\",\"is_error\":true}'\n",
    );
    let e = adapter::run(&invocation(failed, env.clone()), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(e.message(), "the agent reported an error (error_max_turns)");
    let silent = write_script(bin.path(), "silent", "#!/bin/sh\nexit 0\n");
    assert_eq!(
        adapter::run(&invocation(silent, env), &AtomicBool::new(false)),
        Err(AgentError::NoAnswer)
    );

    // A missing configured binary fails the run before anything spawns.
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let c = ctx(
        fake_settings(&bin.path().join("missing"), "claude"),
        FakeProvider::new(&["src/a.rs"]),
    );
    let err = job::execute(&store, &r, &c, &AtomicBool::new(false)).unwrap_err();
    assert!(err.contains("`claude` was not found"), "{err}");
}

#[test]
fn snapshot_paths_cannot_escape_the_temp_dir() {
    let dir = job::TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    let mut evil = changed("../../escape.rs");
    evil.patch = Some("+x".into());
    let diff = FileDiff {
        base_sha: "b".into(),
        head_sha: "h".into(),
        files: vec![evil, changed("/abs/a.rs")],
        truncated: false,
    };
    job::write_snapshot(&path, &diff, &[]).unwrap();
    assert!(path.join("diff/escape.rs.diff").is_file());
    assert!(path.join("diff/abs/a.rs.diff").is_file());
    assert!(path.join("threads.json").is_file());
    drop(dir);
    assert!(!path.exists());
}

#[test]
#[cfg_attr(windows, ignore)]
fn an_answer_is_not_held_up_by_a_grandchild_keeping_stdout_open() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(
        bin.path(),
        "claude",
        "#!/bin/sh\nsleep 20 &\necho '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\"}'\nexit 0\n",
    );
    let inv = invocation(script, adapter::sanitized_env(std::env::vars_os(), None));
    let start = Instant::now();
    assert_eq!(adapter::run(&inv, &AtomicBool::new(false)).unwrap(), "ok");
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn shutting_the_core_down_kills_a_running_agent() {
    let bin = tempfile::tempdir().unwrap();
    let script = write_script(
        bin.path(),
        "claude",
        "#!/bin/sh\necho $$ > \"$(dirname \"$0\")/pid\"\nexec sleep 30\n",
    );
    let store = Store::in_memory().unwrap();
    let r = record(12);
    store.upsert_review(&r).unwrap();
    let provider: Arc<dyn ReviewProvider> = FakeProvider::new(&["src/a.rs"]);
    let core = Core::with_providers(store, vec![provider]);
    // Settings set in place: `update_settings` would rebuild the providers.
    *core.settings.lock().unwrap() = fake_settings(&script, "claude");
    core.run_ai(r.id).unwrap();
    let pid_file = bin.path().join("pid");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pid_file.exists() {
        assert!(Instant::now() < deadline, "the agent never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    let start = Instant::now();
    core.shutdown();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "the agent {pid} still runs");
    let st = core.ai_state(r.id).unwrap();
    assert_eq!(st.error.as_deref(), Some(CANCELLED));
    assert!(core.run_ai(r.id).is_ok_and(|s| !s.queued));
}

#[test]
fn stale_run_dirs_are_swept_and_nothing_else() {
    let base = tempfile::tempdir().unwrap();
    let stale = base
        .path()
        .join(format!("argus-ai-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(stale.join("diff")).unwrap();
    std::fs::write(stale.join("threads.json"), "[]").unwrap();
    let lookalikes = ["argus-ai-notauuid", "argus-ai-keep.txt", "other-dir"];
    for name in lookalikes {
        std::fs::create_dir_all(base.path().join(name)).unwrap();
    }
    assert_eq!(job::sweep_temp_dirs_in(base.path()), 1);
    assert!(!stale.exists());
    for name in lookalikes {
        assert!(base.path().join(name).exists(), "{name}");
    }
}

#[test]
fn over_long_names_do_not_fail_the_snapshot() {
    let dir = job::TempDir::new().unwrap();
    let long = "n".repeat(252);
    let diff = FileDiff {
        base_sha: "b".into(),
        head_sha: "h".into(),
        files: vec![
            changed(&format!("src/{long}")),
            changed(&format!("{}/x.rs", "d".repeat(300))),
        ],
        truncated: false,
    };
    job::write_snapshot(dir.path(), &diff, &[]).unwrap();
    let hashed = |name: &str, suffix: &str| {
        format!(
            "{}{suffix}",
            sha1_smol::Sha1::from(name.as_bytes()).digest()
        )
    };
    assert!(dir
        .path()
        .join("diff/src")
        .join(hashed(&long, ".diff"))
        .is_file());
    assert!(dir
        .path()
        .join("diff")
        .join(hashed(&"d".repeat(300), ""))
        .join("x.rs.diff")
        .is_file());
    assert!(dir.path().join("threads.json").is_file());
}
