//! A scripted platform for the guarantee tests: one GitHub-shaped review
//! whose state the test moves (commits, replies, re-requests, merges), and
//! a log of every provider call split into reads and writes.
//!
//! The repository: `src/cache.rs` is `line 1`..`line 30` at the base `b0`;
//! `c1` rewrites line 5, `c2` deletes line 10. The user's thread sits on
//! line 10 at `c1`, so once `c2` lands it is outdated and its line is gone.
//! All data is fictitious.

#![allow(dead_code)]

pub mod gitlab;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use argus_core::history::ThreadResolution;
use argus_core::post::{PlacedComment, PostError, PostErrorKind, PostedComment, PostedNote};
use argus_core::provider::{
    Anchor, ChangedFile, CommitInfo, DiffRange, FileDiff, FileStatus, MyReviewState, Note,
    ProviderId, ReviewDetail, ReviewProvider, ReviewRef, ReviewState, ReviewVerdict, Side, Thread,
};
use argus_core::store::Store;
use argus_core::{Core, CoreError, CoreResult};
use chrono::{DateTime, TimeZone, Utc};

pub const ME: &str = "rev-fixture";
pub const AUTHOR: &str = "alice-fixture";
pub const PATH: &str = "src/cache.rs";
/// The platform thread id of the user's thread on line 10.
pub const MY_THREAD: &str = "4001";
/// A review-level thread by the author (no code anchor).
pub const GENERAL_THREAD: &str = "4002";

/// 2026-10-<day> <hour>:00 UTC.
pub fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
}

/// Wait until the clock moved on. `mark_reviewed` and the post API stamp
/// the watermark with `Utc::now()`, and the platform dates new notes the
/// same way, so "after the watermark" is real time here: this keeps two
/// stamps from ever being equal, without a fixed sleep.
pub fn tick() {
    let start = Utc::now();
    while Utc::now() <= start {
        std::hint::spin_loop();
    }
}

pub fn file_v(sha: &str) -> String {
    let mut lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
    if sha != "b0" {
        lines[4] = "line five".into();
    }
    if sha == "c2" || sha == "c3" {
        lines.remove(9);
    }
    if sha == "c3" {
        lines.push("line 31".into());
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn changed(patch: &str, content_sha: &str) -> ChangedFile {
    let add = patch.lines().filter(|l| l.starts_with('+')).count() as u32;
    let del = patch.lines().filter(|l| l.starts_with('-')).count() as u32;
    ChangedFile {
        path: PATH.into(),
        old_path: None,
        status: FileStatus::Modified,
        additions: add,
        deletions: del,
        patch: Some(patch.into()),
        content_sha: content_sha.into(),
        binary: false,
        too_large: false,
    }
}

fn commit(sha: &str, day: u32) -> CommitInfo {
    CommitInfo {
        sha: sha.into(),
        title: format!("Commit {sha}"),
        author: AUTHOR.into(),
        authored_at: Some(at(day, 9)),
    }
}

pub fn review_ref() -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: "acme-fixture/widgets".into(),
        number: 12,
        url: "https://github.com/acme-fixture/widgets/pull/12".into(),
        title: "Add widget caching".into(),
        author: AUTHOR.into(),
        updated_at: Some(at(10, 9)),
    }
}

pub fn note(id: &str, author: &str, when: DateTime<Utc>, body: &str) -> Note {
    Note {
        id: id.into(),
        author: author.into(),
        body: body.into(),
        created_at: when,
        system: false,
    }
}

/// What the platform holds right now. Tests change it through
/// [`Platform::with`].
pub struct World {
    pub detail: ReviewDetail,
    pub threads: Vec<Thread>,
    /// The list call still returns the review.
    pub listed: bool,
    /// File and commit reads fail as a network error would.
    pub offline: bool,
    /// What `thread_resolutions` reports (`None`: it fails).
    pub resolutions: Option<Vec<ThreadResolution>>,
    next_id: u64,
}

impl World {
    /// Push commit `sha` (`c2` or `c3`) as the new head.
    pub fn push_commit(&mut self, sha: &str, day: u32) {
        self.detail.head_sha = sha.into();
        self.detail.commits.push(commit(sha, day));
        // GitHub flags a line comment outdated once its line is gone.
        if sha == "c2" {
            if let Some(t) = self.threads.iter_mut().find(|t| t.id == MY_THREAD) {
                t.outdated = true;
            }
        }
    }

    pub fn reply(&mut self, thread: &str, author: &str, body: &str) {
        self.next_id += 1;
        let id = format!("n{}", self.next_id);
        let t = self.threads.iter_mut().find(|t| t.id == thread).unwrap();
        tick();
        t.notes.push(note(&id, author, Utc::now(), body));
        tick();
    }
}

/// One provider call: the method and whether it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub method: &'static str,
    pub write: bool,
}

pub struct Platform {
    world: Mutex<World>,
    calls: Mutex<Vec<Call>>,
}

impl Platform {
    /// The review at head `c1`, requested from the user, with the user's
    /// thread on line 10 and the author's general thread.
    pub fn new() -> Arc<Self> {
        let detail = ReviewDetail {
            review: review_ref(),
            description: "Caches widgets for a minute.".into(),
            state: ReviewState::Open,
            draft: false,
            head_sha: "c1".into(),
            base_sha: "b0".into(),
            start_sha: None,
            commits: vec![commit("c1", 10)],
            versions: Vec::new(),
            reviewers: Vec::new(),
            requested_from_me: true,
            approved_by_me: false,
            my_review_state: Default::default(),
            my_reviewed_at: None,
            my_reviewed_sha: None,
            review_requested_at: None,
        };
        let mine = Thread {
            id: MY_THREAD.into(),
            author: ME.into(),
            created_at: at(10, 11),
            resolved: false,
            resolvable: true,
            outdated: false,
            anchor: Some(Anchor {
                path_old: Some(PATH.into()),
                path_new: Some(PATH.into()),
                line_old: None,
                line_new: Some(10),
                start_line: None,
                side: Side::New,
                base_sha: None,
                start_sha: None,
                head_sha: "c1".into(),
                diff_hunk: None,
                current_line: None,
            }),
            notes: vec![note(MY_THREAD, ME, at(10, 11), "Why does line 10 stay?")],
        };
        let general = Thread {
            id: GENERAL_THREAD.into(),
            author: AUTHOR.into(),
            created_at: at(10, 12),
            resolved: false,
            resolvable: false,
            outdated: false,
            anchor: None,
            notes: vec![note(
                GENERAL_THREAD,
                AUTHOR,
                at(10, 12),
                "Ready for review.",
            )],
        };
        Arc::new(Self {
            world: Mutex::new(World {
                detail,
                threads: vec![mine, general],
                listed: true,
                offline: false,
                resolutions: Some(Vec::new()),
                next_id: 0,
            }),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut World) -> T) -> T {
        f(&mut self.world.lock().unwrap())
    }

    fn log(&self, method: &'static str, write: bool) {
        self.calls.lock().unwrap().push(Call { method, write });
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn writes(&self) -> Vec<&'static str> {
        self.calls()
            .into_iter()
            .filter(|c| c.write)
            .map(|c| c.method)
            .collect()
    }

    pub fn reads(&self) -> usize {
        self.calls().iter().filter(|c| !c.write).count()
    }

    fn online(&self) -> CoreResult<()> {
        if self.world.lock().unwrap().offline {
            return Err(CoreError::Provider("could not reach GitHub".into()));
        }
        Ok(())
    }

    pub fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// The diff of `from..to` on `src/cache.rs` (two lines of context).
    fn diff(&self, from: &str, to: &str) -> FileDiff {
        let (old, new) = (file_v(from), file_v(to));
        let patch = similar::TextDiff::from_lines(&old, &new)
            .unified_diff()
            .context_radius(2)
            .to_string();
        FileDiff {
            base_sha: from.into(),
            head_sha: to.into(),
            files: if patch.is_empty() {
                Vec::new()
            } else {
                vec![changed(patch.trim_end(), &format!("blob-{to}"))]
            },
            truncated: false,
            base_approximate: false,
        }
    }
}

impl ReviewProvider for Platform {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }
    fn host(&self) -> &str {
        "github.com"
    }
    fn current_user(&self) -> CoreResult<String> {
        self.log("current_user", false);
        Ok(ME.into())
    }
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        self.log("list_reviews", false);
        let w = self.world.lock().unwrap();
        Ok(if w.listed && w.detail.state == ReviewState::Open {
            vec![w.detail.review.clone()]
        } else {
            Vec::new()
        })
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        self.log("fetch_review", false);
        Ok(self.world.lock().unwrap().detail.clone())
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        self.log("fetch_threads", false);
        Ok(self.world.lock().unwrap().threads.clone())
    }
    fn file_at(&self, _: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        self.log("file_at", false);
        self.online()?;
        if path != PATH {
            return Err(CoreError::NotFound);
        }
        Ok(file_v(sha))
    }
    fn fetch_files(&self, _: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        self.log("fetch_files", false);
        Ok(match range {
            DiffRange::Review { base, head } => self.diff(base, head),
            DiffRange::Compare { from, to } => self.diff(from, to),
        })
    }
    fn commit_diff(&self, _: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        self.log("commit_diff", false);
        self.online()?;
        let parent = match sha {
            "c1" => "b0",
            "c2" => "c1",
            "c3" => "c2",
            _ => return Err(CoreError::NotFound),
        };
        Ok(self.diff(parent, sha))
    }
    fn thread_resolutions(&self, _: &ReviewRef) -> CoreResult<Vec<ThreadResolution>> {
        self.log("thread_resolutions", false);
        self.world
            .lock()
            .unwrap()
            .resolutions
            .clone()
            .ok_or_else(|| CoreError::Provider("offline".into()))
    }

    fn post_comment(&self, r: &ReviewRef, c: &PlacedComment) -> Result<PostedComment, PostError> {
        self.log("post_comment", true);
        if c.body.contains("reject") {
            return Err(PostError::new(
                PostErrorKind::PostRejected,
                "GitHub: pull_request_review_thread.line must be part of the diff",
            ));
        }
        let mut w = self.world.lock().unwrap();
        w.next_id += 1;
        let id = format!("{}", 5000 + w.next_id);
        let line = c.line_number();
        w.threads.push(Thread {
            id: id.clone(),
            author: ME.into(),
            created_at: Utc::now(),
            resolved: false,
            resolvable: true,
            outdated: false,
            anchor: Some(Anchor {
                path_old: Some(c.old_path.clone()),
                path_new: Some(c.path.clone()),
                line_old: (c.side == Side::Old).then_some(line),
                line_new: (c.side == Side::New).then_some(line),
                start_line: c.start_number(),
                side: c.side,
                base_sha: None,
                start_sha: None,
                head_sha: c.head_sha.clone(),
                diff_hunk: None,
                current_line: None,
            }),
            notes: vec![note(&id, ME, Utc::now(), &c.body)],
        });
        Ok(PostedComment {
            url: format!("{}#discussion_r{id}", r.url),
            provider_thread_id: id.clone(),
            provider_note_id: id,
            path: c.path.clone(),
            side: c.side,
            line,
            start_line: c.start_number(),
            head_sha: c.head_sha.clone(),
        })
    }

    fn reply(&self, r: &ReviewRef, thread: &str, body: &str) -> Result<PostedNote, PostError> {
        self.log("reply", true);
        let mut w = self.world.lock().unwrap();
        w.next_id += 1;
        let id = format!("{}", 6000 + w.next_id);
        let t = w
            .threads
            .iter_mut()
            .find(|t| t.id == thread)
            .ok_or_else(|| PostError::new(PostErrorKind::NotFound, "no such thread"))?;
        t.notes.push(note(&id, ME, Utc::now(), body));
        Ok(PostedNote {
            url: format!("{}#discussion_r{id}", r.url),
            provider_thread_id: thread.into(),
            provider_note_id: id,
        })
    }

    fn approve(&self, _: &ReviewRef, head_sha: &str) -> Result<(), PostError> {
        self.log("approve", true);
        let mut w = self.world.lock().unwrap();
        if w.detail.head_sha != head_sha {
            return Err(PostError::new(PostErrorKind::HeadMoved, "head moved"));
        }
        w.detail.approved_by_me = true;
        Ok(())
    }

    /// Resolves or reopens a thread, recorded as GitLab reports it (a
    /// resolution by the user, now) so the history shows the event.
    fn set_thread_resolved(
        &self,
        _: &ReviewRef,
        thread: &str,
        resolved: bool,
    ) -> Result<(), PostError> {
        self.log("set_thread_resolved", true);
        let mut w = self.world.lock().unwrap();
        let t = w
            .threads
            .iter_mut()
            .find(|t| t.id == thread)
            .ok_or_else(|| PostError::new(PostErrorKind::NotFound, "no such thread"))?;
        if !t.resolvable {
            return Err(PostError::new(
                PostErrorKind::PostRejected,
                "GitHub: this thread cannot be resolved",
            ));
        }
        t.resolved = resolved;
        tick();
        let event = ThreadResolution {
            thread_id: thread.into(),
            resolved,
            by: ME.into(),
            at: Utc::now(),
        };
        if let Some(list) = w.resolutions.as_mut() {
            list.retain(|r| r.thread_id != thread);
            list.push(event);
        }
        Ok(())
    }

    // --- Submit review: one logged write for the verdict (approve through
    // `approve`), one more for a summary. The world then shows the state.
    fn submit_review(
        &self,
        r: &ReviewRef,
        verdict: ReviewVerdict,
        head_sha: &str,
        summary: Option<&str>,
    ) -> Result<(), PostError> {
        match verdict {
            ReviewVerdict::Approve => self.approve(r, head_sha)?,
            ReviewVerdict::RequestChanges | ReviewVerdict::Comment => {
                self.log(
                    if verdict == ReviewVerdict::Comment {
                        "comment_review"
                    } else {
                        "request_changes"
                    },
                    true,
                );
                let mut w = self.world.lock().unwrap();
                if w.detail.head_sha != head_sha {
                    return Err(PostError::new(PostErrorKind::HeadMoved, "head moved"));
                }
                w.detail.my_review_state = if verdict == ReviewVerdict::Comment {
                    MyReviewState::Commented
                } else {
                    MyReviewState::RequestedChanges
                };
                w.detail.requested_from_me = false;
                w.detail.my_reviewed_at = Some(Utc::now());
                w.detail.my_reviewed_sha = Some(head_sha.into());
            }
        }
        if summary.is_some() {
            self.log("summary", true);
        }
        Ok(())
    }
}

/// A core over `platform`, refreshed once.
pub fn core_over(platform: &Arc<Platform>) -> Core {
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![platform.clone()]);
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    core
}

/// The stored review (there is one).
pub fn the_review(core: &Core) -> argus_core::ReviewRecord {
    let all = core.reviews().unwrap();
    assert_eq!(all.len(), 1);
    all.into_iter().next().unwrap()
}

/// What the list shows: the active reviews.
pub fn active(core: &Core) -> Vec<argus_core::ReviewRecord> {
    core.reviews()
        .unwrap()
        .into_iter()
        .filter(|r| r.is_active())
        .collect()
}

pub fn thread_map(core: &Core) -> HashMap<String, argus_core::StoredThread> {
    let id = the_review(core).id;
    core.threads(id)
        .unwrap()
        .into_iter()
        .map(|t| (t.thread.id.clone(), t))
        .collect()
}
