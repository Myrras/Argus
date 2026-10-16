//! Refreshing: list, fetch, diff against the store, classify, persist — and
//! a background loop that does it on an interval.
//!
//! Providers are blocking (as in Podium), so the loop is a plain thread, not
//! a tokio task. Each provider refreshes on its own thread; one failing
//! never stops the other, and one review failing never drops the others.
//! A rate-limited provider is skipped until its limit lifts.

use std::collections::HashSet;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::history::cache::read_line_touches;
use crate::ids::ReviewId;
use crate::provider::{
    is_rate_limited, rate_limited, ProviderId, ReviewDetail, ReviewProvider, ReviewRef,
    ReviewState, Thread,
};
use crate::review::advance_with;
use crate::store::{self, ReviewRecord, Store, WriteColumns};

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
    /// Set when the platform rate-limited the provider: when Argus tries it
    /// again. The message says so too; this is the machine-readable form.
    pub rate_limited_until: Option<DateTime<Utc>>,
}

impl RefreshError {
    fn new(provider: ProviderId, review: Option<ReviewId>, e: &CoreError) -> Self {
        Self {
            provider,
            review,
            message: e.to_string(),
            unavailable: e.is_unavailable(),
            rate_limited_until: None,
        }
    }

    /// `e` as a provider-level error; a rate limit also carries the time
    /// the provider names as its reset.
    fn for_provider(provider: &dyn ReviewProvider, e: &CoreError) -> Self {
        let mut err = Self::new(provider.id(), None, e);
        if is_rate_limited(e) {
            err.rate_limited_until = provider.rate_limited_until();
        }
        err
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
/// closed, or just dropped off, then daily: see [`unlisted_due`]). Merged
/// and closed reviews are final and not fetched again.
///
/// While the provider is rate-limited nothing is called: one provider-level
/// "rate limited until HH:MM" error, the stored reviews untouched. A limit
/// hit halfway is one such error too, not one per review it cut off.
pub fn refresh_provider(
    provider: &dyn ReviewProvider,
    store: &Store,
    now: DateTime<Utc>,
) -> (Vec<ReviewId>, Vec<RefreshError>) {
    let pid = provider.id();
    let fail = |e: CoreError| (Vec::new(), vec![RefreshError::for_provider(provider, &e)]);
    if let Some(until) = provider.rate_limited_until().filter(|u| *u > now) {
        return fail(rate_limited(pid, until));
    }
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
                    && unlisted_due(r, now)
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
    let mut limited: Option<CoreError> = None;
    for (r, listed, result) in fetched {
        let id = r.id();
        let prev = stored.iter().find(|s| s.id == id);
        let outcome = match result {
            Ok((detail, threads)) => persist(store, prev, &detail, &threads, &me, listed, now),
            // An unlisted review that is gone (deleted, access lost): just
            // no longer listed.
            Err(CoreError::NotFound) if !listed => match prev {
                Some(p) => {
                    let dropped = p.listed;
                    let marked = if dropped {
                        store.set_listed(id, false)
                    } else {
                        Ok(())
                    };
                    marked
                        .and_then(|_| store.touch_fetched(id, now))
                        .map(|_| dropped)
                }
                None => Ok(false),
            },
            Err(e) => Err(e),
        };
        match outcome {
            Ok(true) => changed.push(id),
            Ok(false) => {}
            Err(e) if is_rate_limited(&e) => {
                limited.get_or_insert(e);
            }
            Err(e) => errors.push(RefreshError::new(pid, Some(id), &e)),
        }
    }
    if let Some(e) = limited {
        // The latest reset, when calls in parallel hit different ones.
        let e = provider
            .rate_limited_until()
            .map_or(e, |until| rate_limited(pid, until));
        errors.push(RefreshError::for_provider(provider, &e));
    }
    (changed, errors)
}

/// How often a stored open review the list no longer has is fetched (to
/// learn whether it was merged or closed meanwhile): once when it drops
/// off, then at most this often.
const UNLISTED_EVERY: chrono::Duration = chrono::Duration::days(1);

/// Whether the unlisted review `r` is fetched in a refresh at `now`: right
/// after it dropped off (the last list still had it), then once per
/// [`UNLISTED_EVERY`] since its last fetch.
fn unlisted_due(r: &ReviewRecord, now: DateTime<Utc>) -> bool {
    r.listed || now - r.fetched_at >= UNLISTED_EVERY
}

/// Classify a fresh fetch against the stored record and store it if
/// anything changed. `now` is when the fetch started.
///
/// The stored record is re-read, classified against and written in one
/// transaction, so whatever landed while the fetch was on the network (the
/// user marking it reviewed, an approval from Argus) is what the fetch is
/// folded into, never overwritten by an older copy. The poll's own `prev`
/// is therefore not used (kept for callers that still pass one). The
/// watermark is never written here (see [`WriteColumns::Fetched`]).
pub(crate) fn persist(
    store: &Store,
    _prev: Option<&ReviewRecord>,
    detail: &ReviewDetail,
    threads: &[Thread],
    me: &str,
    listed: bool,
    now: DateTime<Utc>,
) -> CoreResult<bool> {
    let id = detail.review.id();
    store.transaction(|conn| {
        let prev = store::read_review(conn, id)?;
        // The cached line walks at this head sharpen the untouched count;
        // none (or a failed read) falls back to the approximation.
        let touches = read_line_touches(conn, id, &detail.head_sha).ok();
        let mut next = advance_with(
            prev.as_ref(),
            detail,
            threads,
            me,
            listed,
            now,
            touches.as_ref(),
        );
        next.fingerprint = fingerprint(detail, threads);
        let changed = match &prev {
            Some(p) => {
                let mut same_time = p.clone();
                same_time.fetched_at = next.fetched_at;
                same_time != next
            }
            None => true,
        };
        if changed {
            store::write_review(conn, &next, WriteColumns::Fetched)?;
            store::write_threads(conn, id, threads, me)?;
        } else if !listed {
            // An unlisted review is fetched daily (see `unlisted_due`),
            // timed from its last fetch, changed or not.
            store::touch_fetched(conn, id, now)?;
        }
        Ok(changed)
    })
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
