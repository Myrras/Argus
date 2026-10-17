use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};

use super::*;
use crate::provider::ProviderId;
use crate::review::{mark_reviewed, ReviewStatus, RoundReason};
use crate::testutil::{at, detail, note, review_ref, thread, AUTHOR, ME};

/// A scripted provider: what it lists, what each review fetches as, and an
/// optional failure for the whole provider.
struct Fake {
    id: ProviderId,
    host: String,
    list: Mutex<Vec<ReviewRef>>,
    details: Mutex<HashMap<u64, CoreResult<ReviewDetail>>>,
    threads: Mutex<HashMap<u64, Vec<Thread>>>,
    down: Mutex<Option<fn() -> CoreError>>,
    /// Reviews whose thread fetch fails, and those whose fetch panics.
    threads_fail: Mutex<HashSet<u64>>,
    threads_panic: Mutex<HashSet<u64>>,
    limited: Mutex<Option<DateTime<Utc>>>,
    /// `current_user` calls: the first call of every refresh.
    calls: AtomicUsize,
    /// When set, the next `fetch_review` says it started and waits for the
    /// go-ahead: a test runs something in the middle of a refresh.
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    /// `fetch_review` calls per review number.
    fetches: Mutex<HashMap<u64, usize>>,
}

impl Fake {
    fn new(id: ProviderId, host: &str) -> Self {
        Self {
            id,
            host: host.into(),
            list: Mutex::new(Vec::new()),
            details: Mutex::new(HashMap::new()),
            threads: Mutex::new(HashMap::new()),
            down: Mutex::new(None),
            threads_fail: Mutex::new(HashSet::new()),
            threads_panic: Mutex::new(HashSet::new()),
            limited: Mutex::new(None),
            calls: AtomicUsize::new(0),
            gate: Mutex::new(None),
            fetches: Mutex::new(HashMap::new()),
        }
    }

    /// Hold the next review fetch: returns (started, go).
    fn hold_next_fetch(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        *self.gate.lock().unwrap() = Some((started_tx, go_rx));
        (started_rx, go_tx)
    }

    fn fetched(&self, n: u64) -> usize {
        self.fetches.lock().unwrap().get(&n).copied().unwrap_or(0)
    }

    /// List `d` and serve it.
    fn serve(&self, d: ReviewDetail, threads: Vec<Thread>) {
        let n = d.review.number;
        let mut list = self.list.lock().unwrap();
        list.retain(|r| r.number != n);
        list.push(d.review.clone());
        self.threads.lock().unwrap().insert(n, threads);
        self.details.lock().unwrap().insert(n, Ok(d));
    }

    fn unlist(&self, n: u64) {
        self.list.lock().unwrap().retain(|r| r.number != n);
    }
}

fn clone_err(e: &CoreError) -> CoreError {
    match e {
        CoreError::NotFound => CoreError::NotFound,
        CoreError::Provider(m) => CoreError::Provider(m.clone()),
        _ => CoreError::Provider("scripted failure".into()),
    }
}

impl ReviewProvider for Fake {
    fn id(&self) -> ProviderId {
        self.id
    }
    fn host(&self) -> &str {
        &self.host
    }
    fn current_user(&self) -> CoreResult<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match *self.down.lock().unwrap() {
            Some(e) => Err(e()),
            None => Ok(ME.into()),
        }
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        Ok(self.list.lock().unwrap().clone())
    }
    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
        *self.fetches.lock().unwrap().entry(r.number).or_default() += 1;
        // Read before the gate: a held fetch returns what the platform had
        // when it started, like a response already on its way.
        let answer = match self.details.lock().unwrap().get(&r.number) {
            Some(Ok(d)) => Some(Ok(d.clone())),
            Some(Err(e)) => Some(Err(clone_err(e))),
            None => None,
        };
        let gate = self.gate.lock().unwrap().take();
        if let Some((started, go)) = gate {
            started.send(()).unwrap();
            go.recv().unwrap();
        }
        match answer {
            Some(Ok(d)) => Ok(d),
            Some(Err(e)) => {
                // Like a real provider: a rate-limit reply starts its pause.
                if crate::provider::is_rate_limited(&e) {
                    *self.limited.lock().unwrap() = Some(at(10, 13));
                }
                Err(e)
            }
            None => Err(CoreError::NotFound),
        }
    }
    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
        if self.threads_panic.lock().unwrap().contains(&r.number) {
            panic!("scripted panic");
        }
        if self.threads_fail.lock().unwrap().contains(&r.number) {
            return Err(CoreError::Provider(
                "could not reach GitLab (timed out)".into(),
            ));
        }
        Ok(self
            .threads
            .lock()
            .unwrap()
            .get(&r.number)
            .cloned()
            .unwrap_or_default())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, _: &str) -> CoreResult<String> {
        Err(CoreError::NotFound)
    }
    fn fetch_files(
        &self,
        _: &ReviewRef,
        _: &crate::provider::DiffRange,
    ) -> CoreResult<crate::provider::FileDiff> {
        Err(CoreError::NotFound)
    }
    fn rate_limited_until(&self) -> Option<DateTime<Utc>> {
        *self.limited.lock().unwrap()
    }
}

fn detail_n(n: u64, head: &str) -> ReviewDetail {
    let mut d = detail(head);
    d.review = review_ref(n);
    d
}

fn gitlab_detail(n: u64, head: &str) -> ReviewDetail {
    let mut d = detail_n(n, head);
    d.review.provider = ProviderId::Gitlab;
    d.review.host = "gitlab.example.test".into();
    d.review.repo = "grp/app".into();
    d
}

fn github() -> Arc<Fake> {
    Arc::new(Fake::new(ProviderId::Github, "github.com"))
}

fn as_providers(fakes: &[Arc<Fake>]) -> Vec<Arc<dyn ReviewProvider>> {
    fakes
        .iter()
        .map(|f| f.clone() as Arc<dyn ReviewProvider>)
        .collect()
}

#[test]
fn persists_new_reviews_and_is_quiet_when_nothing_changed() {
    let gh = github();
    let t = thread("501", ME, "h1", vec![note("501", ME, at(10, 11))]);
    gh.serve(detail_n(12, "h1"), vec![t]);
    gh.serve(detail_n(7, "g1"), vec![]);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(&[gh]);

    let report = refresh_all(&providers, &store, at(10, 12));
    assert!(report.errors.is_empty());
    assert_eq!(report.changed.len(), 2);
    assert_eq!(report.refreshed_at, at(10, 12));
    let id = review_ref(12).id();
    let rec = store.review(id).unwrap().unwrap();
    assert_eq!(rec.status, ReviewStatus::NeedsReview);
    assert_eq!(rec.untouched_threads, 1);
    assert!(!rec.fingerprint.is_empty());
    assert_eq!(store.threads(id).unwrap().len(), 1);

    let again = refresh_all(&providers, &store, at(10, 13));
    assert!(again.changed.is_empty());
    assert!(again.errors.is_empty());
}

#[test]
fn new_commits_after_mark_reviewed_show_up_as_a_new_round() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(std::slice::from_ref(&gh));
    refresh_all(&providers, &store, at(10, 12));
    let id = review_ref(12).id();
    let rec = store.review(id).unwrap().unwrap();
    store
        .upsert_review(&mark_reviewed(&rec, &[], at(10, 13)))
        .unwrap();

    let reply = thread(
        "501",
        ME,
        "h1",
        vec![note("501", ME, at(10, 11)), note("502", AUTHOR, at(11, 8))],
    );
    gh.serve(detail_n(12, "h2"), vec![reply]);
    let report = refresh_all(&providers, &store, at(11, 9));
    assert_eq!(report.changed, [id]);
    let rec = store.review(id).unwrap().unwrap();
    assert_eq!(rec.status, ReviewStatus::NeedsReReview);
    assert_eq!(
        rec.round_reasons,
        [RoundReason::NewCommits, RoundReason::AuthorReplied]
    );
    assert_eq!(rec.round, 2);
}

#[test]
fn one_provider_failing_does_not_stop_the_other() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let gl = Arc::new(Fake::new(ProviderId::Gitlab, "gitlab.example.test"));
    *gl.down.lock().unwrap() = Some(|| CoreError::GitlabAuth);
    let store = Store::in_memory().unwrap();

    let report = refresh_all(&as_providers(&[gh, gl]), &store, at(10, 12));
    assert_eq!(report.changed, [review_ref(12).id()]);
    assert_eq!(report.errors.len(), 1);
    let e = &report.errors[0];
    assert_eq!(e.provider, ProviderId::Gitlab);
    assert_eq!(e.review, None);
    assert!(e.unavailable);
    assert_eq!(e.message, CoreError::GitlabAuth.to_string());
}

#[test]
fn a_failing_review_is_reported_and_the_rest_persisted() {
    let gl = Arc::new(Fake::new(ProviderId::Gitlab, "gitlab.example.test"));
    gl.serve(gitlab_detail(21, "a1"), vec![]);
    gl.serve(gitlab_detail(22, "b1"), vec![]);
    gl.details
        .lock()
        .unwrap()
        .insert(22, Err(CoreError::Provider("x".into())));
    let store = Store::in_memory().unwrap();
    let report = refresh_all(&as_providers(&[gl]), &store, at(10, 12));
    assert_eq!(report.changed, [gitlab_detail(21, "a1").review.id()]);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        report.errors[0].review,
        Some(gitlab_detail(22, "b1").review.id())
    );
    assert!(!report.errors[0].unavailable);
}

#[test]
fn reviews_that_drop_off_the_list_are_fetched_once_more() {
    let gh = github();
    gh.serve(detail_n(1, "h1"), vec![]);
    gh.serve(detail_n(2, "h1"), vec![]);
    gh.serve(detail_n(3, "h1"), vec![]);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(std::slice::from_ref(&gh));
    refresh_all(&providers, &store, at(10, 12));

    // #1 merged without approval, #2 still open but no longer involves the
    // user, #3 deleted.
    let mut merged = detail_n(1, "h1");
    merged.state = ReviewState::Merged;
    gh.serve(merged, vec![]);
    gh.unlist(1);
    gh.unlist(2);
    gh.unlist(3);
    gh.details.lock().unwrap().remove(&3);

    let report = refresh_all(&providers, &store, at(11, 9));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.changed.len(), 3);
    let get = |n| store.review(review_ref(n).id()).unwrap().unwrap();
    assert_eq!(get(1).status, ReviewStatus::ClosedWithoutApproval);
    assert!(get(1).is_active());
    assert!(!get(2).listed && !get(2).is_active());
    assert!(!get(3).listed);

    // Merged is final: not fetched again.
    gh.details.lock().unwrap().remove(&1);
    let report = refresh_all(&providers, &store, at(11, 10));
    assert!(report.errors.is_empty());
}

#[test]
fn the_loop_refreshes_on_an_interval_and_on_demand_and_stops() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let store = Arc::new(Store::in_memory().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let changed = Arc::new(Mutex::new(Vec::new()));
    let log = changed.clone();
    let poller = Poller::start(
        as_providers(&[gh]),
        store.clone(),
        Duration::from_millis(30),
        move |r: &RefreshReport| {
            seen.fetch_add(1, Ordering::SeqCst);
            log.lock().unwrap().extend(r.changed.clone());
        },
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while count.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    poller.refresh_now();
    poller.stop();
    let after_stop = count.load(Ordering::SeqCst);
    assert!(after_stop >= 3);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(count.load(Ordering::SeqCst), after_stop);
    // Only the first refresh changed anything.
    assert_eq!(*changed.lock().unwrap(), [review_ref(12).id()]);
    assert!(store.review(review_ref(12).id()).unwrap().is_some());
}

#[test]
fn a_review_whose_threads_fail_or_panic_does_not_drop_the_others() {
    let gl = Arc::new(Fake::new(ProviderId::Gitlab, "gitlab.example.test"));
    for n in [21, 22, 23, 24, 25] {
        gl.serve(gitlab_detail(n, "a1"), vec![]);
    }
    gl.threads_fail.lock().unwrap().insert(22);
    gl.threads_panic.lock().unwrap().insert(24);
    let store = Store::in_memory().unwrap();
    let report = refresh_all(&as_providers(&[gl]), &store, at(10, 12));
    let id = |n| gitlab_detail(n, "a1").review.id();
    // Results keep the listing order.
    assert_eq!(report.changed, [id(21), id(23), id(25)]);
    let failed: Vec<_> = report.errors.iter().map(|e| e.review).collect();
    assert_eq!(failed, [Some(id(22)), Some(id(24))]);
    assert!(store.review(id(22)).unwrap().is_none());
}

#[test]
fn a_rate_limited_provider_is_skipped_until_the_reset() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let gl = Arc::new(Fake::new(ProviderId::Gitlab, "gitlab.example.test"));
    gl.serve(gitlab_detail(21, "a1"), vec![]);
    let until = at(10, 13);
    *gl.limited.lock().unwrap() = Some(until);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(&[gh.clone(), gl.clone()]);

    let report = refresh_all(&providers, &store, at(10, 12));
    // GitHub refreshed; GitLab was not called at all.
    assert_eq!(report.changed, [review_ref(12).id()]);
    assert_eq!(gl.calls.load(Ordering::SeqCst), 0);
    assert_eq!(report.errors.len(), 1);
    let e = &report.errors[0];
    assert_eq!(
        (e.provider, e.review, e.unavailable),
        (ProviderId::Gitlab, None, false)
    );
    let hhmm = until
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string();
    assert!(
        e.message
            .contains(&format!("GitLab is rate limited until {hhmm}")),
        "{}",
        e.message
    );
    assert_eq!(e.rate_limited_until, Some(until));
    // Only a rate limit sets it.
    let json = serde_json::to_value(e).unwrap();
    assert_eq!(json["rateLimitedUntil"], "2026-10-10T13:00:00Z");

    // Past the reset it is refreshed again.
    let report = refresh_all(&providers, &store, at(10, 14));
    assert!(report.errors.is_empty());
    assert_eq!(report.changed, [gitlab_detail(21, "a1").review.id()]);
    assert_eq!(gl.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_limit_hit_halfway_is_one_provider_error() {
    let gl = Arc::new(Fake::new(ProviderId::Gitlab, "gitlab.example.test"));
    for n in [21, 22, 23] {
        gl.serve(gitlab_detail(n, "a1"), vec![]);
    }
    let until = at(10, 13);
    for n in [22, 23] {
        gl.details.lock().unwrap().insert(
            n,
            Err(crate::provider::rate_limited(ProviderId::Gitlab, until)),
        );
    }
    let store = Store::in_memory().unwrap();
    let report = refresh_all(&as_providers(&[gl]), &store, at(10, 12));
    assert_eq!(report.changed, [gitlab_detail(21, "a1").review.id()]);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].review, None);
    assert!(report.errors[0].message.contains("rate limited until"));
    assert_eq!(report.errors[0].rate_limited_until, Some(until));
}

#[test]
fn a_poll_that_overlaps_mark_reviewed_keeps_the_watermark() {
    let gh = github();
    let mine = thread("501", ME, "h1", vec![note("501", ME, at(10, 11))]);
    gh.serve(detail_n(12, "h1"), vec![mine]);
    let core = crate::Core::with_providers(
        Store::in_memory().unwrap(),
        as_providers(std::slice::from_ref(&gh)),
    );
    assert!(core.refresh().errors.is_empty());
    let id = review_ref(12).id();

    // The poll has read the review (its "before") and fetches a change (a
    // new title); the user marks it reviewed in the middle; the poll
    // stores after.
    let mut d = detail_n(12, "h1");
    d.review.title = "Renamed".into();
    gh.serve(d, vec![]);
    let (started, go) = gh.hold_next_fetch();
    std::thread::scope(|s| {
        let poll = s.spawn(|| core.refresh());
        started.recv().unwrap();
        let marked = core.mark_reviewed(id).unwrap();
        assert_eq!(marked.status, ReviewStatus::WaitingOnAuthor);
        go.send(()).unwrap();
        assert_eq!(poll.join().unwrap().changed, [id]);
    });
    let rec = core.review(id).unwrap();
    assert_eq!(rec.review.title, "Renamed");
    assert_eq!(rec.last_reviewed_sha.as_deref(), Some("h1"));
    assert!(rec.last_reviewed_at.is_some());
    assert_eq!(rec.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn the_poll_never_writes_the_watermark_it_did_not_read() {
    // `persist` with a stale `prev` (no watermark) over a stored one.
    let store = Store::in_memory().unwrap();
    let d = detail_n(12, "h1");
    assert!(persist(&store, None, &d, &[], ME, true, at(10, 12)).unwrap());
    let stale = store.review(d.review.id()).unwrap().unwrap();
    store.set_watermark(stale.id, "h1", at(10, 13)).unwrap();

    let mut moved = detail_n(12, "h2");
    moved.review.title = "Moved".into();
    assert!(persist(&store, Some(&stale), &moved, &[], ME, true, at(10, 14)).unwrap());
    let rec = store.review(stale.id).unwrap().unwrap();
    assert_eq!(rec.last_reviewed_sha.as_deref(), Some("h1"));
    assert_eq!(rec.last_reviewed_at, Some(at(10, 13)));
    // Classified against the stored watermark, not the stale one.
    assert_eq!(rec.status, ReviewStatus::NeedsReReview);
    assert_eq!(rec.round_reasons, [RoundReason::NewCommits]);
    assert_eq!(rec.round, 2);
}

#[test]
fn an_approval_made_in_argus_after_the_fetch_started_survives_it() {
    let store = Store::in_memory().unwrap();
    let d = detail_n(12, "h1");
    persist(&store, None, &d, &[], ME, true, at(10, 12)).unwrap();
    let id = d.review.id();
    // Approved from Argus at 10:30, while a poll that started at 10:20
    // still read the review as not approved.
    store
        .set_approved_at(id, Some(at(10, 12) + chrono::Duration::minutes(30)))
        .unwrap();
    let mut late = d.clone();
    late.review.title = "Late".into();
    persist(
        &store,
        None,
        &late,
        &[],
        ME,
        true,
        at(10, 12) + chrono::Duration::minutes(20),
    )
    .unwrap();
    let rec = store.review(id).unwrap().unwrap();
    assert!(rec.approved_at.is_some());
    assert_eq!(rec.status, ReviewStatus::Approved);

    // A poll that started after the approval and still sees none: revoked.
    late.review.title = "Later".into();
    persist(&store, None, &late, &[], ME, true, at(10, 14)).unwrap();
    let rec = store.review(id).unwrap().unwrap();
    assert_eq!(rec.approved_at, None);
    assert_eq!(rec.status, ReviewStatus::NeedsReview);
}

#[test]
fn a_review_off_the_list_is_fetched_once_then_daily() {
    let gh = github();
    gh.serve(detail_n(2, "h1"), vec![]);
    gh.serve(detail_n(3, "h1"), vec![]);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(std::slice::from_ref(&gh));
    refresh_all(&providers, &store, at(10, 12));
    assert_eq!((gh.fetched(2), gh.fetched(3)), (1, 1));

    // #2 no longer involves the user; #3 is deleted.
    gh.unlist(2);
    gh.unlist(3);
    gh.details.lock().unwrap().remove(&3);
    // Dropping off: fetched once more, to learn why.
    refresh_all(&providers, &store, at(11, 9));
    assert_eq!((gh.fetched(2), gh.fetched(3)), (2, 2));
    assert!(!store.review(review_ref(2).id()).unwrap().unwrap().listed);
    // Then not on every poll...
    refresh_all(&providers, &store, at(11, 10));
    refresh_all(&providers, &store, at(11, 20));
    assert_eq!((gh.fetched(2), gh.fetched(3)), (2, 2));
    // ...but once a day, in case it was merged or closed meanwhile.
    refresh_all(&providers, &store, at(12, 9));
    assert_eq!((gh.fetched(2), gh.fetched(3)), (3, 3));
    refresh_all(&providers, &store, at(12, 10));
    assert_eq!((gh.fetched(2), gh.fetched(3)), (3, 3));

    // Listed again: every poll, as before.
    gh.serve(detail_n(2, "h1"), vec![]);
    refresh_all(&providers, &store, at(12, 11));
    refresh_all(&providers, &store, at(12, 12));
    assert_eq!(gh.fetched(2), 5);
    assert!(store.review(review_ref(2).id()).unwrap().unwrap().listed);
}

#[test]
fn quitting_does_not_wait_for_a_refresh_under_way() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let core = crate::Core::with_providers(
        Store::in_memory().unwrap(),
        as_providers(std::slice::from_ref(&gh)),
    );
    let reports = Arc::new(AtomicUsize::new(0));
    let seen = reports.clone();
    let (started, go) = gh.hold_next_fetch();
    core.start_polling(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    // A refresh is on the network (a hung `gh` or a slow instance).
    started.recv().unwrap();
    let quit = std::time::Instant::now();
    core.shutdown();
    assert!(
        quit.elapsed() < Duration::from_secs(2),
        "quit waited {:?}",
        quit.elapsed()
    );
    // The refresh still ends on its own, and reports nothing after the
    // quit.
    go.send(()).unwrap();
    let id = review_ref(12).id();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while core.store.review(id).unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(reports.load(Ordering::SeqCst), 0);
}

#[test]
fn a_fetch_that_started_earlier_never_overwrites_a_later_one() {
    // A refresh started at 10:12 stored h2 and the user marked it; a fetch
    // that started at 10:11 (still h1) lands after it.
    let store = Store::in_memory().unwrap();
    let id = review_ref(12).id();
    assert!(persist(&store, None, &detail_n(12, "h2"), &[], ME, true, at(10, 12)).unwrap());
    let rec = store.review(id).unwrap().unwrap();
    store
        .upsert_review(&mark_reviewed(&rec, &[], at(10, 13)))
        .unwrap();

    let stored = persist(&store, None, &detail_n(12, "h1"), &[], ME, true, at(10, 11)).unwrap();
    assert!(!stored);
    let rec = store.review(id).unwrap().unwrap();
    assert_eq!(rec.head_sha, "h2");
    assert_eq!(rec.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(rec.round, 1);
    assert_eq!(rec.fetched_at, at(10, 12));
}

#[test]
fn an_unchanged_fetch_still_counts_as_the_latest() {
    let store = Store::in_memory().unwrap();
    let id = review_ref(12).id();
    persist(&store, None, &detail_n(12, "h2"), &[], ME, true, at(10, 11)).unwrap();
    // Nothing changed at 10:14, but it is the newest look at the review.
    assert!(!persist(&store, None, &detail_n(12, "h2"), &[], ME, true, at(10, 14)).unwrap());
    assert_eq!(store.review(id).unwrap().unwrap().fetched_at, at(10, 14));
    // So a fetch from 10:12 that still saw the old head is older than it.
    assert!(!persist(&store, None, &detail_n(12, "h1"), &[], ME, true, at(10, 12)).unwrap());
    assert_eq!(store.review(id).unwrap().unwrap().head_sha, "h2");
}

#[test]
fn a_slow_poll_cannot_undo_a_refresh_that_overtook_it() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(std::slice::from_ref(&gh));
    refresh_all(&providers, &store, at(10, 10));
    let id = review_ref(12).id();

    // A poll starts at 10:11 and its fetch is on the network (h1); the
    // author pushes h2; a refresh after a post (10:12) stores h2 and the
    // user marks it reviewed; then the slow poll lands.
    let (started, go) = gh.hold_next_fetch();
    std::thread::scope(|s| {
        let poll = s.spawn(|| refresh_all(&providers, &store, at(10, 11)));
        started.recv().unwrap();
        persist(&store, None, &detail_n(12, "h2"), &[], ME, true, at(10, 12)).unwrap();
        let rec = store.review(id).unwrap().unwrap();
        store
            .upsert_review(&mark_reviewed(&rec, &[], at(10, 13)))
            .unwrap();
        go.send(()).unwrap();
        let report = poll.join().unwrap();
        assert!(report.changed.is_empty(), "{:?}", report.changed);
    });
    let rec = store.review(id).unwrap().unwrap();
    assert_eq!(rec.head_sha, "h2");
    assert_eq!(rec.last_reviewed_sha.as_deref(), Some("h2"));
    assert_eq!(rec.status, ReviewStatus::WaitingOnAuthor);
    assert!(rec.round_reasons.is_empty());
    assert_eq!(rec.round, 1);
}

#[test]
fn refreshes_run_one_at_a_time() {
    let gh = github();
    gh.serve(detail_n(12, "h1"), vec![]);
    let store = Store::in_memory().unwrap();
    let providers = as_providers(std::slice::from_ref(&gh));
    let (started, go) = gh.hold_next_fetch();
    let busy = std::thread::scope(|s| {
        let first = s.spawn(|| refresh_all(&providers, &store, at(10, 11)));
        started.recv().unwrap();
        // A second refresh (Shift+R while the poll runs) waits for the
        // first instead of fetching alongside it.
        let busy = store.refreshing.try_lock().is_err();
        go.send(()).unwrap();
        first.join().unwrap();
        busy
    });
    assert!(busy, "a refresh under way holds the refresh lock");
    assert!(store.refreshing.try_lock().is_ok());
    assert_eq!(refresh_all(&providers, &store, at(10, 12)).errors, []);
}
