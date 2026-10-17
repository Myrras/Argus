use std::sync::{Arc, Mutex};

use super::anchor::{parse_hunks, validate};
use super::*;
use crate::error::{CoreError, CoreResult};
use crate::ids::ThreadId;
use crate::provider::{
    ChangedFile, DiffRange, FileDiff, FileStatus, ProviderId, ReviewDetail, ReviewProvider,
    ReviewRef, Side, Thread,
};
use crate::review::ReviewStatus;
use crate::store::Store;
use crate::testutil::{at, detail, note, thread, AUTHOR, ME};
use crate::Core;

/// The file at `h1`, against its base: two hunks.
const PATCH_H1: &str = "@@ -1,4 +1,5 @@
 fn a() {}
-fn b() {}
+fn b2() {}
+fn c() {}
 fn d() {}
 fn e() {}
@@ -10,2 +11,3 @@
 fn x() {}
+fn y() {}
 fn z() {}";

/// The same file at `h2`: two lines added at the top since `h1`.
const PATCH_H2: &str = "@@ -1,4 +1,7 @@
+// one
+// two
 fn a() {}
-fn b() {}
+fn b2() {}
+fn c() {}
 fn d() {}
 fn e() {}
@@ -10,2 +13,3 @@
 fn x() {}
+fn y() {}
 fn z() {}";

/// `h1` → `h2` for that file.
const SINCE_H1_H2: &str = "@@ -0,0 +1,2 @@
+// one
+// two";

fn changed(path: &str, patch: &str) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        old_path: None,
        status: FileStatus::Modified,
        additions: 1,
        deletions: 1,
        patch: Some(patch.into()),
        content_sha: format!("sha-{path}"),
        binary: false,
        too_large: false,
    }
}

fn diff(base: &str, head: &str, files: Vec<ChangedFile>) -> FileDiff {
    FileDiff {
        base_sha: base.into(),
        head_sha: head.into(),
        files,
        truncated: false,
        base_approximate: false,
    }
}

fn comment(line: u32, side: Side) -> NewComment {
    NewComment {
        path: "src/cache.rs".into(),
        line,
        side,
        start_line: None,
        body: "Why this?".into(),
        head_sha: None,
        base_sha: None,
    }
}

fn kind(r: Result<PlacedComment, PostError>) -> PostErrorKind {
    r.unwrap_err().kind
}

// --- the pure anchor rules -------------------------------------------------

#[test]
fn parses_hunks_with_gitlab_counters() {
    let hunks = parse_hunks(
        "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n\\ No newline at end of file\n\n\
         @@ -0,0 +1 @@\n+new",
    );
    assert_eq!(hunks.len(), 2);
    let rows: Vec<(LineKind, u32, u32)> = hunks[0]
        .rows
        .iter()
        .map(|r| (r.kind, r.old_pos, r.new_pos))
        .collect();
    // An added row keeps the old counter it sits at (GitLab's line codes);
    // the blank line is a context row whose space was stripped.
    assert_eq!(
        rows,
        [
            (LineKind::Context, 1, 1),
            (LineKind::Removed, 2, 2),
            (LineKind::Added, 3, 2),
            (LineKind::Context, 3, 3),
        ]
    );
    assert_eq!(hunks[1].rows[0].new_line(), Some(1));
    assert_eq!(hunks[1].rows[0].old_line(), None);
}

#[test]
fn lists_commentable_lines_per_hunk() {
    let hunks = commentable_hunks(PATCH_H1);
    assert_eq!(hunks.len(), 2);
    assert_eq!(hunks[0].new, [1, 2, 3, 4, 5]);
    assert_eq!(hunks[0].old, [1, 2, 3, 4]);
    assert_eq!(hunks[1].new, [11, 12, 13]);
    assert_eq!(hunks[1].old, [10, 11]);
    assert!(commentable_hunks("").is_empty());
}

#[test]
fn places_lines_by_side() {
    // Added: new side only.
    let p = place(PATCH_H1, Side::New, 3, None).unwrap();
    assert_eq!(p.line.kind, LineKind::Added);
    assert_eq!((p.line.old_line(), p.line.new_line()), (None, Some(3)));
    // Context: both numbers, from either side.
    let p = place(PATCH_H1, Side::New, 4, None).unwrap();
    assert_eq!((p.line.old_line(), p.line.new_line()), (Some(3), Some(4)));
    let p = place(PATCH_H1, Side::Old, 3, None).unwrap();
    assert_eq!(p.line.new_line(), Some(4));
    // Removed: old side only.
    let p = place(PATCH_H1, Side::Old, 2, None).unwrap();
    assert_eq!(p.line.kind, LineKind::Removed);
    assert_eq!(p.line.new_line(), None);
    // Old line 2 is removed, so new line 2 is the added `b2`.
    assert_eq!(
        place(PATCH_H1, Side::New, 2, None).unwrap().line.kind,
        LineKind::Added
    );
    // Outside every hunk, on either side.
    assert!(place(PATCH_H1, Side::New, 8, None).is_none());
    assert!(place(PATCH_H1, Side::Old, 12, None).is_none());
}

#[test]
fn places_ranges_inside_one_hunk_only() {
    let p = place(PATCH_H1, Side::New, 5, Some(2)).unwrap();
    assert_eq!(p.start.unwrap().new_line(), Some(2));
    assert_eq!(p.line.new_line(), Some(5));
    // Across hunks, reversed, or the start outside a hunk.
    assert!(place(PATCH_H1, Side::New, 12, Some(4)).is_none());
    assert!(place(PATCH_H1, Side::New, 2, Some(5)).is_none());
    assert!(place(PATCH_H1, Side::New, 12, Some(9)).is_none());
    // A one-line "range" is a single line.
    assert!(place(PATCH_H1, Side::New, 3, Some(3))
        .unwrap()
        .start
        .is_none());
    // Old side: removed + context of the first hunk.
    let p = place(PATCH_H1, Side::Old, 4, Some(2)).unwrap();
    assert_eq!(p.start.unwrap().kind, LineKind::Removed);
}

#[test]
fn maps_lines_forward_through_a_diff() {
    // Two lines inserted at the top: everything moves down by two.
    assert_eq!(map_forward(SINCE_H1_H2, 1), Some(3));
    assert_eq!(map_forward(SINCE_H1_H2, 12), Some(14));

    let patch = "@@ -3,3 +3,2 @@\n c\n-d\n e\n@@ -10,0 +10,2 @@\n+n1\n+n2";
    // Before any hunk: unchanged.
    assert_eq!(map_forward(patch, 2), Some(2));
    // A context row inside a hunk: its new number.
    assert_eq!(map_forward(patch, 3), Some(3));
    assert_eq!(map_forward(patch, 5), Some(4));
    // A removed (changed) line has no counterpart.
    assert_eq!(map_forward(patch, 4), None);
    // Between the hunks: shifted up by the removal.
    assert_eq!(map_forward(patch, 8), Some(7));
    // A pure insertion after old line 10: line 10 stays above it.
    assert_eq!(map_forward(patch, 10), Some(9));
    assert_eq!(map_forward(patch, 11), Some(12));
}

#[test]
fn resolves_at_an_unchanged_head() {
    let current = diff("base", "h1", vec![changed("src/cache.rs", PATCH_H1)]);
    let placed = resolve(&comment(12, Side::New), "h1", &current, None).unwrap();
    assert_eq!(placed.line_number(), 12);
    assert_eq!(placed.head_sha, "h1");
    assert_eq!(placed.base_sha, "base");
    assert_eq!(placed.old_path, "src/cache.rs");

    // Not on a hunk row: the line is not in the diff.
    assert_eq!(
        kind(resolve(&comment(8, Side::New), "h1", &current, None)),
        PostErrorKind::LineNotInDiff
    );
    let err = resolve(&comment(8, Side::New), "h1", &current, None).unwrap_err();
    assert!(err.message.contains("src/cache.rs:8"));
    assert!(err.message.contains("no longer in the diff"));
    // A file not in the diff, or one without a text patch.
    let mut other = comment(1, Side::New);
    other.path = "src/other.rs".into();
    assert_eq!(
        kind(resolve(&other, "h1", &current, None)),
        PostErrorKind::LineNotInDiff
    );
    let mut binary = current.clone();
    binary.files[0].patch = None;
    assert_eq!(
        kind(resolve(&comment(3, Side::New), "h1", &binary, None)),
        PostErrorKind::LineNotInDiff
    );
}

#[test]
fn resolves_a_shifted_line_after_the_head_moved() {
    let current = diff("base", "h2", vec![changed("src/cache.rs", PATCH_H2)]);
    let since = diff("h1", "h2", vec![changed("src/cache.rs", SINCE_H1_H2)]);
    let placed = resolve(&comment(12, Side::New), "h1", &current, Some(&since)).unwrap();
    assert_eq!(placed.line_number(), 14);
    assert_eq!(placed.head_sha, "h2");

    // A range moves as a whole.
    let mut range = comment(5, Side::New);
    range.start_line = Some(3);
    let placed = resolve(&range, "h1", &current, Some(&since)).unwrap();
    assert_eq!((placed.start_number(), placed.line_number()), (Some(5), 7));

    // The file is untouched since: the lines stand.
    let untouched = diff("h1", "h2", vec![]);
    let current_h1 = diff("base", "h2", vec![changed("src/cache.rs", PATCH_H1)]);
    let placed = resolve(&comment(12, Side::New), "h1", &current_h1, Some(&untouched)).unwrap();
    assert_eq!(placed.line_number(), 12);
}

#[test]
fn a_range_moves_only_when_every_line_of_it_survived() {
    // h1 lines 3..=5 of a file; since h1 line 4 changed, the ends did not.
    let current = diff("base", "h2", vec![changed("src/cache.rs", PATCH_H2)]);
    let changed_middle = diff(
        "h1",
        "h2",
        vec![changed(
            "src/cache.rs",
            "@@ -4 +4 @@\n-fn d() {}\n+fn d2() {}",
        )],
    );
    let mut range = comment(5, Side::New);
    range.start_line = Some(3);
    let err = resolve(&range, "h1", &current, Some(&changed_middle)).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::LineNotInDiff);
    assert!(err.message.contains("changed since"), "{}", err.message);

    // A line inserted inside the range: the ends map, the range grew.
    let inserted = diff(
        "h1",
        "h2",
        vec![changed("src/cache.rs", "@@ -3,0 +4 @@\n+// new")],
    );
    let err = resolve(&range, "h1", &current, Some(&inserted)).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::LineNotInDiff);
}

#[test]
fn fails_a_line_removed_since_or_a_rewritten_branch() {
    let current = diff("base", "h2", vec![changed("src/cache.rs", PATCH_H2)]);
    // `fn y` (h1 line 12) was changed between h1 and h2.
    let since = diff(
        "h1",
        "h2",
        vec![changed(
            "src/cache.rs",
            "@@ -12 +12 @@\n-fn y() {}\n+fn y2() {}",
        )],
    );
    let err = resolve(&comment(12, Side::New), "h1", &current, Some(&since)).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::LineNotInDiff);
    assert!(err.message.contains("changed since"));

    // The diff since is from another commit: the branch was rebased.
    let rebased = diff("elsewhere", "h2", vec![]);
    assert_eq!(
        kind(resolve(
            &comment(12, Side::New),
            "h1",
            &current,
            Some(&rebased)
        )),
        PostErrorKind::LineNotInDiff
    );

    // The file was deleted since.
    let mut gone = changed("src/cache.rs", "@@ -1,13 +0,0 @@\n-x");
    gone.status = FileStatus::Removed;
    let deleted = diff("h1", "h2", vec![gone]);
    let err = resolve(&comment(12, Side::New), "h1", &current, Some(&deleted)).unwrap_err();
    assert!(err.message.contains("deleted"));
}

#[test]
fn follows_a_rename_since() {
    let mut renamed_now = changed("src/store.rs", PATCH_H1);
    renamed_now.old_path = Some("src/cache.rs".into());
    renamed_now.status = FileStatus::Renamed;
    let current = diff("base", "h2", vec![renamed_now.clone()]);
    let mut moved = changed("src/store.rs", "@@ -1 +1 @@\n fn a() {}");
    moved.old_path = Some("src/cache.rs".into());
    moved.status = FileStatus::Renamed;
    let since = diff("h1", "h2", vec![moved]);
    let placed = resolve(&comment(12, Side::New), "h1", &current, Some(&since)).unwrap();
    assert_eq!(placed.path, "src/store.rs");
    assert_eq!(placed.old_path, "src/cache.rs");
}

#[test]
fn keeps_old_side_lines_unless_the_base_moved() {
    let current = diff("base", "h2", vec![changed("src/cache.rs", PATCH_H2)]);
    // The old side is the base: new commits do not move it.
    let placed = resolve(&comment(2, Side::Old), "h1", &current, None).unwrap();
    assert_eq!(placed.line_number(), 2);
    assert_eq!(placed.line.kind, LineKind::Removed);
    let mut pinned = comment(2, Side::Old);
    pinned.base_sha = Some("base".into());
    assert!(resolve(&pinned, "h1", &current, None).is_ok());
    pinned.base_sha = Some("old-base".into());
    let err = resolve(&pinned, "h1", &current, None).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::LineNotInDiff);
    assert!(
        err.message.contains("since-last-review AI run") && err.message.contains("full diff"),
        "{}",
        err.message
    );
}

#[test]
fn refuses_old_side_lines_against_a_stand_in_left_side() {
    // GitHub had no merge base and no clone gave one: the old side shown
    // may not line up with the patch, so a removed line is never placed.
    let mut current = diff("tip", "h1", vec![changed("src/cache.rs", PATCH_H1)]);
    current.base_approximate = true;
    for base in [None, Some("tip".to_string())] {
        let mut c = comment(2, Side::Old);
        c.base_sha = base;
        let err = resolve(&c, "h1", &current, None).unwrap_err();
        assert_eq!(err.kind, PostErrorKind::LineNotInDiff);
        assert!(
            err.message.contains(
                "Removed lines can't take comments for this file until Argus knows its merge base"
            ),
            "{}",
            err.message
        );
    }
    // The new side does not depend on the left side: it still posts.
    assert!(resolve(&comment(3, Side::New), "h1", &current, None).is_ok());
}

#[test]
fn validates_input() {
    let ok = comment(3, Side::New);
    assert!(validate(&ok).is_ok());
    for bad in [
        NewComment {
            body: "  ".into(),
            ..ok.clone()
        },
        NewComment {
            line: 0,
            ..ok.clone()
        },
        NewComment {
            start_line: Some(4),
            ..ok.clone()
        },
        NewComment {
            path: String::new(),
            ..ok.clone()
        },
    ] {
        assert_eq!(
            validate(&bad).unwrap_err().kind,
            PostErrorKind::InvalidInput
        );
    }
}

#[test]
fn sanitizes_platform_text() {
    let s = sanitize("bad\ntoken glpat-x\u{7} here", &["glpat-x", ""]);
    assert_eq!(s, "bad token [redacted] here");
    assert_eq!(sanitize(&"x".repeat(400), &[]).chars().count(), 301);
}

#[test]
fn serializes_results_for_the_wire() {
    let posted = PostResult::Posted(PostedComment {
        url: "u".into(),
        provider_thread_id: "t".into(),
        provider_note_id: "n".into(),
        path: "p".into(),
        side: Side::New,
        line: 3,
        start_line: None,
        head_sha: "h".into(),
    });
    let v = serde_json::to_value(&posted).unwrap();
    assert_eq!(v["status"], "posted");
    assert_eq!(v["providerThreadId"], "t");
    let failed = PostResult::Failed(PostError::new(PostErrorKind::LineNotInDiff, "gone"));
    let v = serde_json::to_value(&failed).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "status": "failed", "kind": "lineNotInDiff", "message": "gone" })
    );
    let c: NewComment = serde_json::from_value(serde_json::json!({
        "path": "a.rs", "line": 4, "side": "new", "body": "b", "startLine": 2
    }))
    .unwrap();
    assert_eq!((c.start_line, c.head_sha), (Some(2), None));
    assert_eq!(PostErrorKind::HeadMoved.as_str(), "headMoved");
    let from: PostError = CoreError::GitlabAuth.into();
    assert_eq!(from.kind, PostErrorKind::GitlabAuth);
}

// --- the post API through the core -----------------------------------------

/// One review whose head, diff and threads a test controls; records every
/// write.
struct Fake {
    detail: Mutex<ReviewDetail>,
    files: Mutex<Vec<ChangedFile>>,
    since: Mutex<Vec<ChangedFile>>,
    threads: Mutex<Vec<Thread>>,
    posted: Mutex<Vec<PlacedComment>>,
    replies: Mutex<Vec<(String, String)>>,
    approvals: Mutex<u32>,
    /// The head each approval was pinned to.
    approved_heads: Mutex<Vec<String>>,
    /// Whether an approval shows on the next fetch.
    approval_shows: Mutex<bool>,
    fetches: Mutex<u32>,
    /// `fetch_files` calls.
    file_reads: Mutex<u32>,
    /// The next review-diff read lands a push to `h2` right after it (the
    /// diff stays the `h1` one): a push between reading the diff and
    /// posting.
    push_after_files: Mutex<bool>,
}

impl Fake {
    fn writes(&self) -> usize {
        self.posted.lock().unwrap().len()
            + self.replies.lock().unwrap().len()
            + *self.approvals.lock().unwrap() as usize
    }
}

impl ReviewProvider for Fake {
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
        Ok(vec![self.detail.lock().unwrap().review.clone()])
    }
    fn fetch_review(&self, _: &ReviewRef) -> CoreResult<ReviewDetail> {
        *self.fetches.lock().unwrap() += 1;
        Ok(self.detail.lock().unwrap().clone())
    }
    fn fetch_threads(&self, _: &ReviewRef) -> CoreResult<Vec<Thread>> {
        Ok(self.threads.lock().unwrap().clone())
    }
    fn file_at(&self, _: &ReviewRef, _: &str, _: &str) -> CoreResult<String> {
        Err(CoreError::NotFound)
    }
    fn fetch_files(&self, _: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        *self.file_reads.lock().unwrap() += 1;
        Ok(match range {
            DiffRange::Review { head, .. } => {
                let out = diff("mergebase", head, self.files.lock().unwrap().clone());
                if std::mem::take(&mut *self.push_after_files.lock().unwrap()) {
                    push_h2(self);
                }
                out
            }
            DiffRange::Compare { from, to } => diff(from, to, self.since.lock().unwrap().clone()),
        })
    }
    fn post_comment(&self, r: &ReviewRef, c: &PlacedComment) -> Result<PostedComment, PostError> {
        if c.body.contains("reject") {
            return Err(PostError::new(PostErrorKind::PostRejected, "Fake: refused"));
        }
        // As GitLab's own version check: a push landed right before this
        // comment, which is refused in its slot (not the whole batch).
        if c.body.contains("push first") {
            push_h2(self);
            return Err(PostError::new(PostErrorKind::HeadMoved, "Fake: moved"));
        }
        let mut posted = self.posted.lock().unwrap();
        posted.push(c.clone());
        let id = format!("{}", 500 + posted.len());
        let mut t = thread(&id, ME, &c.head_sha, vec![note(&id, ME, at(11, 9))]);
        t.notes[0].body = c.body.clone();
        self.threads.lock().unwrap().push(t);
        Ok(PostedComment {
            url: format!("{}#r{id}", r.url),
            provider_thread_id: id.clone(),
            provider_note_id: id,
            path: c.path.clone(),
            side: c.side,
            line: c.line_number(),
            start_line: c.start_number(),
            head_sha: c.head_sha.clone(),
        })
    }
    fn reply(&self, _: &ReviewRef, thread: &str, body: &str) -> Result<PostedNote, PostError> {
        self.replies
            .lock()
            .unwrap()
            .push((thread.to_string(), body.to_string()));
        Ok(PostedNote {
            url: "u".into(),
            provider_thread_id: thread.into(),
            provider_note_id: "77".into(),
        })
    }
    fn approve(&self, _: &ReviewRef, head_sha: &str) -> Result<(), PostError> {
        *self.approvals.lock().unwrap() += 1;
        self.approved_heads
            .lock()
            .unwrap()
            .push(head_sha.to_string());
        if *self.approval_shows.lock().unwrap() {
            let mut d = self.detail.lock().unwrap();
            d.approved_by_me = true;
            d.requested_from_me = false;
        }
        Ok(())
    }
}

fn setup() -> (Core, Arc<Fake>, ReviewId) {
    let fake = Arc::new(Fake {
        detail: Mutex::new(detail("h1")),
        files: Mutex::new(vec![changed("src/cache.rs", PATCH_H1)]),
        since: Mutex::new(Vec::new()),
        threads: Mutex::new(vec![thread(
            "900",
            AUTHOR,
            "h1",
            vec![note("900", AUTHOR, at(10, 9))],
        )]),
        posted: Mutex::new(Vec::new()),
        replies: Mutex::new(Vec::new()),
        approvals: Mutex::new(0),
        approved_heads: Mutex::new(Vec::new()),
        approval_shows: Mutex::new(true),
        fetches: Mutex::new(0),
        file_reads: Mutex::new(0),
        push_after_files: Mutex::new(false),
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![fake.clone()]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    (core, fake, id)
}

/// Moves the platform to `h2` (two lines added at the top of the file).
fn push_h2(fake: &Fake) {
    fake.detail.lock().unwrap().head_sha = "h2".into();
    *fake.files.lock().unwrap() = vec![changed("src/cache.rs", PATCH_H2)];
    *fake.since.lock().unwrap() = vec![changed("src/cache.rs", SINCE_H1_H2)];
}

#[test]
fn nothing_is_written_without_a_post_call() {
    let (core, fake, _) = setup();
    core.refresh();
    core.refresh();
    assert_eq!(fake.writes(), 0);
}

#[test]
fn a_batch_posts_each_comment_and_reports_each_failure() {
    let (core, fake, id) = setup();
    let mut rejected = comment(3, Side::New);
    rejected.body = "please reject".into();
    let results = core
        .post_comments(
            id,
            vec![
                comment(12, Side::New),
                comment(8, Side::New),
                rejected,
                NewComment {
                    body: String::new(),
                    ..comment(3, Side::New)
                },
                comment(2, Side::Old),
            ],
        )
        .unwrap();
    let kinds: Vec<Option<PostErrorKind>> = results
        .iter()
        .map(|r| match r {
            PostResult::Posted(_) => None,
            PostResult::Failed(e) => Some(e.kind),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            None,
            Some(PostErrorKind::LineNotInDiff),
            Some(PostErrorKind::PostRejected),
            Some(PostErrorKind::InvalidInput),
            None,
        ]
    );
    let PostResult::Posted(first) = &results[0] else {
        unreachable!()
    };
    assert_eq!((first.line, first.head_sha.as_str()), (12, "h1"));
    // Only the placeable ones reached the platform; both went out.
    let posted = fake.posted.lock().unwrap();
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[1].side, Side::Old);
    drop(posted);

    // The refresh stored the new threads.
    let threads = core.threads(id).unwrap();
    assert_eq!(threads.len(), 3);
    assert!(threads.iter().filter(|t| t.mine).count() == 2);
}

#[test]
fn posting_advances_the_watermark_and_failures_do_not() {
    let (core, _fake, id) = setup();
    let before = core.review(id).unwrap();
    assert_eq!(before.status, ReviewStatus::NeedsReview);

    // Nothing posted: nothing moves.
    let results = core.post_comments(id, vec![comment(8, Side::New)]).unwrap();
    assert!(!results[0].is_posted());
    assert_eq!(core.review(id).unwrap().last_reviewed_sha, None);

    core.post_comments(id, vec![comment(12, Side::New)])
        .unwrap();
    let after = core.review(id).unwrap();
    assert_eq!(after.last_reviewed_sha.as_deref(), Some("h1"));
    assert!(after.last_reviewed_at.is_some());
    assert_eq!(after.status, ReviewStatus::WaitingOnAuthor);
    assert!(after.is_active());
}

#[test]
fn a_comment_follows_the_head_and_the_watermark_stays_at_what_was_seen() {
    let (core, fake, id) = setup();
    push_h2(&fake);
    // Written against h1 (the stored head): `fn y` is at 14 now.
    let results = core
        .post_comments(id, vec![comment(12, Side::New)])
        .unwrap();
    let PostResult::Posted(p) = &results[0] else {
        panic!("{results:?}")
    };
    assert_eq!((p.line, p.head_sha.as_str()), (14, "h2"));
    assert_eq!(fake.posted.lock().unwrap()[0].line_number(), 14);

    // The user saw h1, not h2: watermark at h1, so h2 still needs a look.
    let after = core.review(id).unwrap();
    assert_eq!(after.head_sha, "h2");
    assert_eq!(after.last_reviewed_sha.as_deref(), Some("h1"));
    assert_eq!(after.status, ReviewStatus::NeedsReReview);
}

#[test]
fn a_push_between_reading_the_diff_and_posting_fails_the_batch_and_refreshes() {
    let (core, fake, id) = setup();
    *fake.push_after_files.lock().unwrap() = true;
    let err = core
        .post_comments(id, vec![comment(12, Side::New)])
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    assert_eq!(fake.writes(), 0);
    // The stored review caught up, so a retry is placed on the new head.
    assert_eq!(core.review(id).unwrap().head_sha, "h2");
    let results = core
        .post_comments(id, vec![comment(14, Side::New)])
        .unwrap();
    assert!(results[0].is_posted(), "{results:?}");
}

#[test]
fn a_diff_of_another_head_than_the_platforms_fails_the_batch() {
    struct Lagging(Arc<Fake>);
    impl ReviewProvider for Lagging {
        fn id(&self) -> ProviderId {
            self.0.id()
        }
        fn host(&self) -> &str {
            "github.com"
        }
        fn current_user(&self) -> CoreResult<String> {
            self.0.current_user()
        }
        fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
            self.0.list_reviews()
        }
        fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
            self.0.fetch_review(r)
        }
        fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
            self.0.fetch_threads(r)
        }
        fn file_at(&self, r: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
            self.0.file_at(r, sha, path)
        }
        // The platform served the diff of an older head than asked for.
        fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
            let mut d = self.0.fetch_files(r, range)?;
            d.head_sha = "h0".into();
            Ok(d)
        }
        fn post_comment(
            &self,
            r: &ReviewRef,
            c: &PlacedComment,
        ) -> Result<PostedComment, PostError> {
            self.0.post_comment(r, c)
        }
    }
    let (_, fake, id) = setup();
    let store = Store::in_memory().unwrap();
    let core = Core::with_providers(store, vec![Arc::new(Lagging(fake.clone()))]);
    assert!(core.refresh().errors.is_empty());
    let err = core
        .post_comments(id, vec![comment(12, Side::New)])
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    assert_eq!(fake.writes(), 0);
}

#[test]
fn a_batch_with_nothing_valid_never_reaches_the_platform() {
    let (core, fake, id) = setup();
    let fetches = *fake.fetches.lock().unwrap();
    let results = core
        .post_comments(
            id,
            vec![NewComment {
                line: 0,
                ..comment(3, Side::New)
            }],
        )
        .unwrap();
    assert!(!results[0].is_posted());
    assert_eq!(*fake.fetches.lock().unwrap(), fetches);
    assert!(core.post_comments(id, Vec::new()).unwrap().is_empty());
    assert_eq!(
        core.post_comments(ReviewId::from_key("missing"), vec![comment(3, Side::New)])
            .unwrap_err()
            .kind,
        PostErrorKind::NotFound
    );
}

#[test]
fn approving_sets_approved_at_and_hides_the_review() {
    let (core, fake, id) = setup();
    let record = core.approve_review(id, "h1").unwrap();
    assert_eq!(*fake.approvals.lock().unwrap(), 1);
    assert_eq!(*fake.approved_heads.lock().unwrap(), ["h1"]);
    assert!(record.approved_at.is_some());
    assert_eq!(record.status, ReviewStatus::Approved);
    assert!(!record.is_active());
    assert_eq!(record.last_reviewed_sha.as_deref(), Some("h1"));
    assert_eq!(core.review(id).unwrap(), record);
    // The platform agrees: the next refresh keeps it.
    core.refresh();
    assert!(core.review(id).unwrap().approved_at.is_some());
}

#[test]
fn an_approval_not_shown_yet_still_counts() {
    let (core, fake, id) = setup();
    *fake.approval_shows.lock().unwrap() = false;
    let record = core.approve_review(id, "h1").unwrap();
    assert!(record.approved_at.is_some());
    assert!(!record.is_active());
}

#[test]
fn an_approval_is_pinned_to_the_head_the_user_saw() {
    let (core, fake, id) = setup();
    // The user looked at h1; the platform got h2 since (not polled yet).
    push_h2(&fake);
    let err = core.approve_review(id, "h1").unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    // The refusal refreshed the stored review: the UI re-reads h2.
    assert_eq!(core.review(id).unwrap().head_sha, "h2");
    // A head other than the stored one (the view was stale).
    let err = core.approve_review(id, "h0").unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    assert_eq!(
        core.approve_review(id, " ").unwrap_err().kind,
        PostErrorKind::InvalidInput
    );
    assert_eq!(fake.writes(), 0);
    let record = core.review(id).unwrap();
    assert!(record.approved_at.is_none());
    assert!(record.is_active());

    // After a refresh the user sees h2 and can approve exactly that.
    core.refresh();
    core.approve_review(id, "h2").unwrap();
    assert_eq!(*fake.approved_heads.lock().unwrap(), ["h2"]);
}

#[test]
fn replies_go_to_the_platform_thread() {
    let (core, fake, id) = setup();
    let thread = core.threads(id).unwrap()[0].thread_id;
    assert_eq!(core.thread_review(thread).unwrap(), id);
    assert!(matches!(
        core.thread_review(ThreadId::from_key(id, "nope")),
        Err(CoreError::NotFound)
    ));
    let note = core.reply_to_thread(thread, "Fixed?").unwrap();
    assert_eq!(note.provider_thread_id, "900");
    assert_eq!(
        *fake.replies.lock().unwrap(),
        [("900".to_string(), "Fixed?".to_string())]
    );
    assert_eq!(
        core.review(id).unwrap().last_reviewed_sha.as_deref(),
        Some("h1")
    );
    assert_eq!(
        core.reply_to_thread(thread, " ").unwrap_err().kind,
        PostErrorKind::InvalidInput
    );
    assert_eq!(
        core.reply_to_thread(ThreadId::from_key(id, "nope"), "x")
            .unwrap_err()
            .kind,
        PostErrorKind::NotFound
    );
    assert_eq!(fake.replies.lock().unwrap().len(), 1);
}

#[test]
fn commentable_lines_come_from_the_stored_heads_diff() {
    let (core, _fake, id) = setup();
    let lines = core.commentable_lines(id, "src/cache.rs").unwrap();
    assert_eq!(lines.head_sha, "h1");
    assert_eq!(lines.hunks.len(), 2);
    assert_eq!(lines.hunks[1].new, [11, 12, 13]);
    let json = serde_json::to_value(&lines).unwrap();
    assert_eq!(json["hunks"][0]["old"], serde_json::json!([1, 2, 3, 4]));
    assert!(matches!(
        core.commentable_lines(id, "src/none.rs"),
        Err(CoreError::InvalidInput(_))
    ));
}

#[test]
fn commentable_lines_read_the_diff_once_per_head() {
    let (core, fake, id) = setup();
    let reads = || *fake.file_reads.lock().unwrap();
    let before = reads();
    core.commentable_lines(id, "src/cache.rs").unwrap();
    core.commentable_lines(id, "src/cache.rs").unwrap();
    let _ = core.commentable_lines(id, "src/none.rs");
    assert_eq!(reads(), before + 1);
    // A new stored head reads it again.
    push_h2(&fake);
    core.refresh();
    let after_refresh = reads();
    let lines = core.commentable_lines(id, "src/cache.rs").unwrap();
    assert_eq!(lines.head_sha, "h2");
    assert_eq!(reads(), after_refresh + 1);
}

#[test]
fn commentable_lines_are_read_again_when_the_base_moves() {
    // A retarget changes the diff but not the head.
    let (core, fake, id) = setup();
    let before = core.commentable_lines(id, "src/cache.rs").unwrap();
    assert_eq!(before.hunks[1].new, [11, 12, 13]);
    let reads = || *fake.file_reads.lock().unwrap();
    let after_first = reads();

    fake.detail.lock().unwrap().base_sha = "retargeted".into();
    *fake.files.lock().unwrap() = vec![changed("src/cache.rs", PATCH_H2)];
    core.refresh();
    assert_eq!(core.review(id).unwrap().head_sha, "h1");
    let lines = core.commentable_lines(id, "src/cache.rs").unwrap();
    assert_eq!(reads(), after_first + 1);
    assert_eq!(lines.hunks[1].new, [13, 14, 15]);
}

#[test]
fn the_watermark_rule_after_a_write() {
    use super::api::advance_after_write;
    let (core, _fake, id) = setup();
    let record = core.review(id).unwrap();
    // Seen = head: plain mark_reviewed.
    let next = advance_after_write(&record, &[], "h1", false, at(12, 9));
    assert_eq!(next, crate::review::mark_reviewed(&record, &[], at(12, 9)));
    // Seen an older head: re-review, a new round.
    let next = advance_after_write(&record, &[], "h0", false, at(12, 9));
    assert_eq!(next.last_reviewed_sha.as_deref(), Some("h0"));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round, record.round + 1);
}

#[test]
fn a_comment_refused_for_a_moved_head_refreshes_the_review() {
    let (core, _fake, id) = setup();
    let mut moved = comment(12, Side::New);
    moved.body = "push first".into();
    let results = core.post_comments(id, vec![moved]).unwrap();
    let PostResult::Failed(e) = &results[0] else {
        panic!("{results:?}")
    };
    assert_eq!(e.kind, PostErrorKind::HeadMoved);
    // Nothing was posted, yet the stored review caught up with the push,
    // so the retry the user makes is placed on the new head.
    let after = core.review(id).unwrap();
    assert_eq!(after.head_sha, "h2");
    assert_eq!(after.last_reviewed_sha, None);
    let results = core
        .post_comments(id, vec![comment(14, Side::New)])
        .unwrap();
    assert!(results[0].is_posted(), "{results:?}");
}

#[test]
fn a_poll_storing_while_the_watermark_moves_is_kept() {
    let (core, fake, id) = setup();
    let store = core.store.clone();
    let mut renamed = fake.detail.lock().unwrap().clone();
    renamed.review.title = "Renamed".into();
    let platform = fake.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let poll: Arc<Mutex<Option<std::thread::JoinHandle<()>>>> = Arc::default();
    let handle = poll.clone();
    // Right after the post read the stored review back, a poll stores a
    // change (a new title). It is given a moment to land: with the read
    // and the write in one transaction it waits for the write instead.
    super::api::BEFORE_WATERMARK_WRITE.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            let threads = platform.threads.lock().unwrap().clone();
            *handle.lock().unwrap() = Some(std::thread::spawn(move || {
                // Started after the post's own refresh (an older one is
                // dropped, see `persist`).
                let now = chrono::Utc::now();
                let seq = store.begin_fetch();
                crate::poll::persist(&store, None, &renamed, &threads, ME, true, now, seq).unwrap();
                let _ = done_tx.send(());
            }));
            let _ = done_rx.recv_timeout(std::time::Duration::from_millis(300));
        }));
    });
    let results = core
        .post_comments(id, vec![comment(12, Side::New)])
        .unwrap();
    assert!(results[0].is_posted(), "{results:?}");
    poll.lock()
        .unwrap()
        .take()
        .expect("the hook ran")
        .join()
        .unwrap();
    let rec = core.review(id).unwrap();
    assert_eq!(rec.review.title, "Renamed");
    assert_eq!(rec.last_reviewed_sha.as_deref(), Some("h1"));
    assert_eq!(rec.status, ReviewStatus::WaitingOnAuthor);
}
