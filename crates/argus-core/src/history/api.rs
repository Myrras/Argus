//! The [`Core`] entry points of thread history, and how one thread's
//! history is computed.
//!
//! Per thread: the then snapshot from the file at the commit the comment
//! was made on (GitHub's `diff_hunk` as the fallback), the walk to head,
//! the now snapshot, the commit and resolve timeline entries. Cached at
//! (thread, head) unless a provider call failed on the way, so a network
//! hiccup is retried on the next call instead of sticking until the head
//! moves.

use std::collections::HashMap;
use std::sync::Arc;

use super::cache::{CachedHistory, CachedMapping};
use super::snapshot::{anchored_text, around, diff_rows, from_diff_hunk, window};
use super::timeline::{commit_entries, merge, note_entries, resolution_entries};
use super::walk::{walk_line, ApiSource, LineMapping, WalkStart};
use super::{
    CodeSnapshot, LineStatus, SnapshotSource, SnapshotText, ThreadHistory, ThreadResolution,
};
use crate::error::{CoreError, CoreResult};
use crate::ids::{ReviewId, ThreadId};
use crate::provider::{Anchor, CommitInfo, ReviewProvider, ReviewRef, Side, Thread};
use crate::review::{untouched_with, RoundInput};
use crate::store::{ReviewRecord, Store, StoredThread};
use crate::Core;

/// How many threads of one review are computed at the same time.
const PARALLELISM: usize = 4;

impl Core {
    /// The then/now history of one thread at its review's current head
    /// (blocking: computed on a cache miss, which calls the provider; run it
    /// from `spawn_blocking`). Provider failures degrade to
    /// [`LineStatus::Unlocated`] and are not cached; the error cases are an
    /// unknown thread or a database failure.
    pub fn thread_history(&self, thread_id: ThreadId) -> CoreResult<ThreadHistory> {
        let review = self
            .store
            .thread_review(thread_id)?
            .ok_or(CoreError::NotFound)?;
        self.histories(review, Some(&[thread_id]))?
            .pop()
            .ok_or(CoreError::NotFound)
    }

    /// [`thread_history`](Self::thread_history) for every thread of a
    /// review, in the stored order (oldest first). The threads share the
    /// commit diffs they need, so this is cheaper than one call per thread.
    pub fn review_histories(&self, review_id: ReviewId) -> CoreResult<Vec<ThreadHistory>> {
        self.histories(review_id, None)
    }

    /// A thread's cached then/now text at its review's current head, for
    /// the AI prompt (M5's `ThreadSnapshot`). Cheap: two SQLite reads, no
    /// provider call, no computing; `None` until a history call cached it
    /// for this head (or for an unknown thread).
    pub fn cached_snapshot(&self, thread_id: ThreadId) -> CoreResult<Option<SnapshotText>> {
        let Some(review) = self.store.thread_review(thread_id)? else {
            return Ok(None);
        };
        let Some(record) = self.store.review(review)? else {
            return Ok(None);
        };
        self.store.snapshot_text(thread_id, &record.head_sha)
    }

    fn histories(&self, id: ReviewId, only: Option<&[ThreadId]>) -> CoreResult<Vec<ThreadHistory>> {
        let record = self.review(id)?;
        let provider = self.provider_for(&record);
        compute_histories(&self.store, &record, &provider, only)
    }
}

/// The histories of `record`'s threads (or just those in `only`): cached ones as
/// they are, the rest computed through `provider` and cached. Free of
/// [`Core`] so the AI pass can run it from its job thread, where it holds
/// the store and the provider but no `Core`.
pub(crate) fn compute_histories(
    store: &Store,
    record: &ReviewRecord,
    provider: &CoreResult<Arc<dyn ReviewProvider>>,
    only: Option<&[ThreadId]>,
) -> CoreResult<Vec<ThreadHistory>> {
    let threads: Vec<StoredThread> = store
        .threads(record.id)?
        .into_iter()
        .filter(|t| only.is_none_or(|o| o.contains(&t.thread_id)))
        .collect();
    let mut cached: Vec<Option<CachedHistory>> = Vec::with_capacity(threads.len());
    for t in &threads {
        let hit = store
            .cached_history(t.thread_id, &record.head_sha)?
            .filter(|c| c.mapping.resolved == t.thread.resolved);
        cached.push(hit);
    }
    let misses: Vec<usize> = (0..threads.len())
        .filter(|&i| cached[i].is_none())
        .collect();
    if !misses.is_empty() {
        let wanted: Vec<&Thread> = misses.iter().map(|&i| &threads[i].thread).collect();
        let computed = compute(provider, record, &wanted);
        for (&i, (history, cacheable)) in misses.iter().zip(computed) {
            if cacheable {
                store.put_history(threads[i].thread_id, &record.head_sha, &history)?;
            }
            cached[i] = Some(history);
        }
        refresh_untouched(store, record)?;
    }
    Ok(threads
        .iter()
        .zip(cached)
        .map(|(t, c)| assemble(record, t, c.expect("every miss was computed")))
        .collect())
}

/// Compute `threads` (all of `record`): each with whether it may be
/// cached.
fn compute(
    provider: &CoreResult<Arc<dyn ReviewProvider>>,
    record: &ReviewRecord,
    threads: &[&Thread],
) -> Vec<(CachedHistory, bool)> {
    let provider = match provider {
        Ok(p) => p,
        Err(e) => return threads.iter().map(|t| degraded(t, e)).collect(),
    };
    // The review's commits, in order: the walk's steps and the
    // timeline's commit details.
    let detail = match provider.fetch_review(&record.review) {
        Ok(d) => d,
        Err(e) => return threads.iter().map(|t| degraded(t, &e)).collect(),
    };
    let (resolutions, resolutions_ok) = if threads.iter().any(|t| t.resolved) {
        match provider.thread_resolutions(&record.review) {
            Ok(list) => (group(list), true),
            Err(_) => (HashMap::new(), false),
        }
    } else {
        (HashMap::new(), true)
    };
    let ctx = Ctx {
        provider: provider.as_ref(),
        review: &record.review,
        head: &record.head_sha,
        commits: &detail.commits,
        source: ApiSource::new(provider.as_ref(), &record.review, &detail.commits),
        resolutions,
        resolutions_ok,
    };
    let mut out = Vec::with_capacity(threads.len());
    for chunk in threads.chunks(PARALLELISM) {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|t| (t, s.spawn(|| compute_one(&ctx, t))))
                .collect();
            for (t, h) in handles {
                out.push(h.join().unwrap_or_else(|_| {
                    degraded(t, &CoreError::Provider("the history crashed".into()))
                }));
            }
        });
    }
    out
}

/// Store the review's untouched count as the cached walks now tell it.
fn refresh_untouched(store: &Store, record: &ReviewRecord) -> CoreResult<()> {
    let threads: Vec<Thread> = store
        .threads(record.id)?
        .into_iter()
        .map(|t| t.thread)
        .collect();
    let touches = store.line_touches(record.id, &record.head_sha)?;
    let n = untouched_with(
        &RoundInput {
            state: record.state,
            head_sha: &record.head_sha,
            last_reviewed_sha: record.last_reviewed_sha.as_deref(),
            last_reviewed_at: record.last_reviewed_at,
            approved_at: record.approved_at,
            rerequested_at: record.rerequested_at,
            threads: &threads,
            me: &record.viewer,
        },
        Some(&touches),
    );
    if n != record.untouched_threads {
        store.set_round(
            record.id,
            record.round,
            &record.round_reasons,
            record.status,
            n,
        )?;
    }
    Ok(())
}

/// What every thread of one computation shares.
struct Ctx<'a> {
    provider: &'a dyn ReviewProvider,
    review: &'a ReviewRef,
    head: &'a str,
    commits: &'a [CommitInfo],
    source: ApiSource<'a>,
    resolutions: HashMap<String, Vec<ThreadResolution>>,
    /// The resolve events could be read (or were not needed).
    resolutions_ok: bool,
}

fn group(list: Vec<ThreadResolution>) -> HashMap<String, Vec<ThreadResolution>> {
    let mut out: HashMap<String, Vec<ThreadResolution>> = HashMap::new();
    for r in list {
        out.entry(r.thread_id.clone()).or_default().push(r);
    }
    out
}

/// A provider error that may pass (network, rate limit, auth): the result
/// is not cached. A missing file or a bad sha is an answer, not a failure.
fn is_transient(e: &CoreError) -> bool {
    !matches!(e, CoreError::NotFound | CoreError::InvalidInput(_))
}

/// The Unlocated reason for a provider failure: the error's own text when
/// it says what to fix, a generic retry hint otherwise.
fn failure_reason(e: &CoreError) -> String {
    if e.is_unavailable() {
        e.to_string()
    } else {
        "could not fetch what is needed to follow the line; try again later".into()
    }
}

/// The history of a thread that could not be computed at all.
fn degraded(thread: &Thread, e: &CoreError) -> (CachedHistory, bool) {
    let status = match thread.anchor {
        Some(_) => LineStatus::unlocated(failure_reason(e)),
        None => LineStatus::NotCode,
    };
    (
        CachedHistory {
            mapping: CachedMapping {
                status,
                touched_by: Vec::new(),
                attributed: false,
                then_sha: None,
                commits: Vec::new(),
                resolved: thread.resolved,
            },
            then_snapshot: None,
            now_snapshot: None,
            entries: Vec::new(),
        },
        false,
    )
}

/// Where the anchor points: the revision, file and lines it was left on.
enum AnchorAt<'a> {
    /// On the head side (added or context lines): followed to head.
    New {
        sha: &'a str,
        path: &'a str,
        start: u32,
        end: u32,
    },
    /// On lines the review's diff removes: they exist only in the base.
    Old {
        base: Option<&'a str>,
        path: &'a str,
        start: u32,
        end: u32,
    },
    /// On the file, not a line (or nothing usable).
    NoLine,
}

fn anchor_at(a: &Anchor) -> AnchorAt<'_> {
    let range = |end: u32| (a.start_line.unwrap_or(end).min(end).max(1), end);
    match a.side {
        Side::New => match (a.line_new, a.path_new.as_deref().or(a.path_old.as_deref())) {
            (Some(end), Some(path)) if end > 0 && !a.head_sha.is_empty() => {
                let (start, end) = range(end);
                AnchorAt::New {
                    sha: &a.head_sha,
                    path,
                    start,
                    end,
                }
            }
            _ => AnchorAt::NoLine,
        },
        Side::Old => match (a.line_old, a.path_old.as_deref().or(a.path_new.as_deref())) {
            (Some(end), Some(path)) if end > 0 => {
                let (start, end) = range(end);
                AnchorAt::Old {
                    base: a.base_sha.as_deref().filter(|s| !s.is_empty()),
                    path,
                    start,
                    end,
                }
            }
            _ => AnchorAt::NoLine,
        },
    }
}

fn compute_one(ctx: &Ctx<'_>, thread: &Thread) -> (CachedHistory, bool) {
    let mut transient = false;
    let resolutions = ctx
        .resolutions
        .get(&thread.id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if thread.resolved && !ctx.resolutions_ok {
        transient = true;
    }
    let mut entries = resolution_entries(resolutions);
    let mapping_of = |status, then_sha: Option<&str>| CachedMapping {
        status,
        touched_by: Vec::new(),
        attributed: false,
        then_sha: then_sha.map(str::to_string),
        commits: ctx.commits.iter().map(|c| c.sha.clone()).collect(),
        resolved: thread.resolved,
    };
    let Some(anchor) = &thread.anchor else {
        let history = CachedHistory {
            mapping: mapping_of(LineStatus::NotCode, None),
            then_snapshot: None,
            now_snapshot: None,
            entries,
        };
        return (history, !transient);
    };
    let history = match anchor_at(anchor) {
        AnchorAt::NoLine => CachedHistory {
            mapping: mapping_of(
                LineStatus::unlocated("the comment is on the file, not on a line"),
                None,
            ),
            then_snapshot: None,
            now_snapshot: None,
            entries,
        },
        AnchorAt::Old {
            base,
            path,
            start,
            end,
        } => {
            let then = base
                .and_then(|sha| fetch_window(ctx, sha, path, start, end, &mut transient))
                .or_else(|| hunk_window(anchor, path, Side::Old, start, end));
            CachedHistory {
                mapping: mapping_of(
                    LineStatus::Deleted {
                        path: path.to_string(),
                        removed_by: None,
                    },
                    None,
                ),
                then_snapshot: then,
                now_snapshot: None,
                entries,
            }
        }
        AnchorAt::New {
            sha,
            path,
            start,
            end,
        } => {
            let then = fetch_window(ctx, sha, path, start, end, &mut transient)
                .or_else(|| hunk_window(anchor, path, Side::New, start, end));
            let walk_start = WalkStart {
                sha: sha.to_string(),
                path: path.to_string(),
                start_line: start,
                line: end,
            };
            let mut mapping = match walk_line(&ctx.source, &walk_start, ctx.head) {
                Ok(m) => m,
                Err(e) => {
                    transient |= is_transient(&e);
                    unlocated_mapping(failure_reason(&e))
                }
            };
            // GitHub's own current line, for a thread it does not consider
            // outdated, beats giving up.
            if matches!(mapping.status, LineStatus::Unlocated { .. }) && !thread.outdated {
                if let Some(line) = anchor.current_line {
                    mapping = LineMapping {
                        status: LineStatus::Located {
                            path: path.to_string(),
                            start_line: line,
                            line,
                            moved: line != end,
                            renamed: false,
                            touched: false,
                        },
                        events: Vec::new(),
                        touched_by: Vec::new(),
                        attributed: false,
                        near: None,
                    };
                }
            }
            let now = now_window(ctx, &mut mapping, then.as_ref(), &mut transient);
            entries.extend(commit_entries(&mapping.events, ctx.commits));
            CachedHistory {
                mapping: CachedMapping {
                    status: mapping.status,
                    touched_by: mapping.touched_by,
                    attributed: mapping.attributed,
                    then_sha: Some(sha.to_string()),
                    commits: ctx.commits.iter().map(|c| c.sha.clone()).collect(),
                    resolved: thread.resolved,
                },
                then_snapshot: then,
                now_snapshot: now,
                entries,
            }
        }
    };
    (history, !transient)
}

fn unlocated_mapping(reason: impl Into<String>) -> LineMapping {
    LineMapping {
        status: LineStatus::unlocated(reason),
        events: Vec::new(),
        touched_by: Vec::new(),
        attributed: false,
        near: None,
    }
}

/// The window of `path` at `sha` around `start..=end`; `None` when the file
/// cannot be read or does not reach the lines.
fn fetch_window(
    ctx: &Ctx<'_>,
    sha: &str,
    path: &str,
    start: u32,
    end: u32,
    transient: &mut bool,
) -> Option<CodeSnapshot> {
    match ctx.provider.file_at(ctx.review, sha, path) {
        Ok(text) => window(sha, path, &text, start, end),
        Err(e) => {
            *transient |= is_transient(&e);
            None
        }
    }
}

fn hunk_window(
    anchor: &Anchor,
    path: &str,
    side: Side,
    start: u32,
    end: u32,
) -> Option<CodeSnapshot> {
    let hunk = anchor.diff_hunk.as_deref()?;
    from_diff_hunk(&anchor.head_sha, path, hunk, side, start, end)
}

/// The now window for where the walk put the lines. A located line that is
/// not in the file at head, or an untouched line whose text differs from
/// then, means the walk went wrong: the status turns Unlocated rather than
/// show a wrong line.
fn now_window(
    ctx: &Ctx<'_>,
    mapping: &mut LineMapping,
    then: Option<&CodeSnapshot>,
    transient: &mut bool,
) -> Option<CodeSnapshot> {
    match &mapping.status {
        LineStatus::Located {
            path,
            start_line,
            line,
            touched,
            ..
        } => {
            let text = match ctx.provider.file_at(ctx.review, ctx.head, path) {
                Ok(t) => t,
                Err(e) => {
                    *transient |= is_transient(&e);
                    return None;
                }
            };
            let Some(now) = window(ctx.head, path, &text, *start_line, *line) else {
                mapping.status =
                    LineStatus::unlocated("the line is past the end of the file at head");
                return None;
            };
            if !touched && !same_anchored_text(then, &now) {
                mapping.status = LineStatus::unlocated("the line could not be followed reliably");
                return None;
            }
            Some(now)
        }
        LineStatus::Deleted { path, .. } => {
            let near = mapping.near?;
            match ctx.provider.file_at(ctx.review, ctx.head, path) {
                Ok(text) => around(ctx.head, path, &text, near),
                Err(e) => {
                    *transient |= is_transient(&e);
                    None
                }
            }
        }
        _ => None,
    }
}

/// An untouched line must read the same then and now. Compared on the whole
/// range when both hold it at the same length, on the commented (last) line
/// otherwise; `true` when the then side is not a full file to compare with.
fn same_anchored_text(then: Option<&CodeSnapshot>, now: &CodeSnapshot) -> bool {
    let Some(then) = then.filter(|t| t.source == SnapshotSource::File) else {
        return true;
    };
    match (anchored_text(then), anchored_text(now)) {
        (Some(a), Some(b)) if a.len() == b.len() => a == b,
        (Some(a), Some(b)) => a.last() == b.last(),
        _ => true,
    }
}

/// The wire shape: the cached parts, the diff of the two windows, and the
/// timeline with the thread's current notes merged in.
fn assemble(record: &ReviewRecord, t: &StoredThread, c: CachedHistory) -> ThreadHistory {
    let diff_lines = match (&c.then_snapshot, &c.now_snapshot) {
        (Some(a), Some(b)) => diff_rows(a, b),
        _ => Vec::new(),
    };
    let mut timeline = note_entries(&t.thread);
    timeline.extend(c.entries);
    ThreadHistory {
        thread_id: t.thread_id,
        review_id: record.id,
        head_sha: record.head_sha.clone(),
        outdated: t.thread.outdated,
        status: c.mapping.status,
        touched_by: c.mapping.touched_by,
        then_snapshot: c.then_snapshot,
        now_snapshot: c.now_snapshot,
        diff_lines,
        timeline: merge(timeline),
    }
}
