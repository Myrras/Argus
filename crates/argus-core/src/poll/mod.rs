//! Refreshing: list, fetch, diff against the store, classify, persist — and
//! a background loop that does it on an interval.
//!
//! Providers are blocking (as in Podium), so the loop is a plain thread, not
//! a tokio task. Each provider refreshes on its own thread; one failing
//! never stops the other.

use std::collections::HashSet;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::ids::ReviewId;
use crate::provider::{ProviderId, ReviewDetail, ReviewProvider, ReviewRef, ReviewState, Thread};
use crate::review::advance;
use crate::store::{ReviewRecord, Store};

/// How many reviews of one provider are fetched at the same time.
const FETCH_PARALLELISM: usize = 4;

/// One failure during a refresh. `message` is the error's app-owned text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshError {
    pub provider: ProviderId,
    /// `None` when the provider as a whole failed (listing, auth, `gh`).
    pub review: Option<ReviewId>,
    pub message: String,
    /// The provider cannot be used right now (`gh` missing or logged out,
    /// no GitLab token, token rejected).
    pub unavailable: bool,
}

impl RefreshError {
    fn new(provider: ProviderId, review: Option<ReviewId>, e: &CoreError) -> Self {
        Self {
            provider,
            review,
            message: e.to_string(),
            unavailable: e.is_unavailable(),
        }
    }
}

/// What one refresh did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    /// Reviews whose stored state changed (new ones included).
    pub changed: Vec<ReviewId>,
    pub errors: Vec<RefreshError>,
    pub refreshed_at: DateTime<Utc>,
}

/// Refresh every provider, in parallel.
pub fn refresh_all(
    providers: &[Arc<dyn ReviewProvider>],
    store: &Store,
    now: DateTime<Utc>,
) -> RefreshReport {
    let results: Vec<(Vec<ReviewId>, Vec<RefreshError>)> = std::thread::scope(|s| {
        let handles: Vec<_> = providers
            .iter()
            .map(|p| (p.id(), s.spawn(|| refresh_provider(p.as_ref(), store, now))))
            .collect();
        handles
            .into_iter()
            .map(|(id, h)| {
                h.join().unwrap_or_else(|_| {
                    let e = CoreError::Provider("the refresh crashed".into());
                    (Vec::new(), vec![RefreshError::new(id, None, &e)])
                })
            })
            .collect()
    });
    let mut report = RefreshReport {
        changed: Vec::new(),
        errors: Vec::new(),
        refreshed_at: now,
    };
    for (changed, errors) in results {
        report.changed.extend(changed);
        report.errors.extend(errors);
    }
    report
}

/// One fetched review, or why it could not be.
type Fetched = (ReviewRef, bool, CoreResult<(ReviewDetail, Vec<Thread>)>);

/// Refresh one provider: what it lists now, plus the stored open reviews it
/// no longer lists (fetched once more to learn whether they were merged,
/// closed, or just dropped off). Merged and closed reviews are final and
/// not fetched again.
pub fn refresh_provider(
    provider: &dyn ReviewProvider,
    store: &Store,
    now: DateTime<Utc>,
) -> (Vec<ReviewId>, Vec<RefreshError>) {
    let pid = provider.id();
    let fail = |e: CoreError| (Vec::new(), vec![RefreshError::new(pid, None, &e)]);
    let me = match provider.current_user() {
        Ok(me) => me,
        Err(e) => return fail(e),
    };
    let listed = match provider.list_reviews() {
        Ok(l) => l,
        Err(e) => return fail(e),
    };
    let stored = match store.reviews() {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let listed_ids: HashSet<ReviewId> = listed.iter().map(ReviewRef::id).collect();
    let mut targets: Vec<(ReviewRef, bool)> = listed.into_iter().map(|r| (r, true)).collect();
    targets.extend(
        stored
            .iter()
            .filter(|r| {
                r.review.provider == pid
                    && r.review.host == provider.host()
                    && r.state == ReviewState::Open
                    && !listed_ids.contains(&r.id)
            })
            .map(|r| (r.review.clone(), false)),
    );

    let mut fetched: Vec<Fetched> = Vec::with_capacity(targets.len());
    for chunk in targets.chunks(FETCH_PARALLELISM) {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(r, listed)| {
                    let h = s.spawn(move || {
                        let detail = provider.fetch_review(r)?;
                        let threads = provider.fetch_threads(r)?;
                        Ok((detail, threads))
                    });
                    (r, *listed, h)
                })
                .collect();
            for (r, listed, h) in handles {
                let result = h
                    .join()
                    .unwrap_or_else(|_| Err(CoreError::Provider("a fetch crashed".into())));
                fetched.push((r.clone(), listed, result));
            }
        });
    }

    let mut changed = Vec::new();
    let mut errors = Vec::new();
    for (r, listed, result) in fetched {
        let id = r.id();
        let prev = stored.iter().find(|s| s.id == id);
        let outcome = match result {
            Ok((detail, threads)) => persist(store, prev, &detail, &threads, &me, listed, now),
            // An unlisted review that is gone (deleted, access lost): just
            // no longer listed.
            Err(CoreError::NotFound) if !listed => match prev {
                Some(p) if p.listed => {
                    let mut next = p.clone();
                    next.listed = false;
                    store.upsert_review(&next).map(|_| true)
                }
                _ => Ok(false),
            },
            Err(e) => Err(e),
        };
        match outcome {
            Ok(true) => changed.push(id),
            Ok(false) => {}
            Err(e) => errors.push(RefreshError::new(pid, Some(id), &e)),
        }
    }
    (changed, errors)
}

/// Classify a fresh fetch against `prev` and store it if anything changed.
pub(crate) fn persist(
    store: &Store,
    prev: Option<&ReviewRecord>,
    detail: &ReviewDetail,
    threads: &[Thread],
    me: &str,
    listed: bool,
    now: DateTime<Utc>,
) -> CoreResult<bool> {
    let mut next = advance(prev, detail, threads, me, listed, now);
    next.fingerprint = fingerprint(detail, threads);
    let changed = match prev {
        Some(p) => {
            let mut same_time = p.clone();
            same_time.fetched_at = next.fetched_at;
            same_time != next
        }
        None => true,
    };
    if changed {
        store.upsert_review(&next)?;
        store.upsert_threads(next.id, threads, me)?;
    }
    Ok(changed)
}

/// FNV-1a over the fetched data: stable across runs and releases (unlike
/// `DefaultHasher`), and plenty to tell "something changed".
fn fingerprint(detail: &ReviewDetail, threads: &[Thread]) -> String {
    let json = serde_json::to_string(&(detail, threads)).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

enum Command {
    Now,
    Stop,
}

/// A running background refresh loop. Dropping it stops the loop (and waits
/// for an in-flight refresh to finish).
pub struct Poller {
    tx: Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl Poller {
    /// Refresh right away, then every `interval`, calling `on_refresh` with
    /// each report (the shell emits `review:changed` per id in
    /// `report.changed` and one `reviews:refreshed`).
    pub fn start<F>(
        providers: Vec<Arc<dyn ReviewProvider>>,
        store: Arc<Store>,
        interval: Duration,
        on_refresh: F,
    ) -> Self
    where
        F: Fn(&RefreshReport) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("argus-poll".into())
            .spawn(move || loop {
                let report = refresh_all(&providers, &store, Utc::now());
                on_refresh(&report);
                match rx.recv_timeout(interval) {
                    Ok(Command::Now) | Err(RecvTimeoutError::Timeout) => {}
                    Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                }
                // Collapse a burst of "now" requests into one refresh.
                loop {
                    match rx.try_recv() {
                        Ok(Command::Now) => {}
                        Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => return,
                        Err(mpsc::TryRecvError::Empty) => break,
                    }
                }
            })
            .expect("spawn the poll thread");
        Self {
            tx,
            handle: Some(handle),
        }
    }

    /// Refresh now instead of waiting for the interval.
    pub fn refresh_now(&self) {
        let _ = self.tx.send(Command::Now);
    }

    /// Stop the loop and wait for it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
