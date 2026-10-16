//! Thread history for the threads that are easy to lose (spec 1, 2): on a
//! removed line, on a whole file, on a commit the walk cannot place,
//! resolved, and while the platform is unreachable. Each stays readable:
//! its notes, the code it was left on where that can be read, and a status
//! that says what Argus knows instead of a guessed line.

mod support;

use argus_core::history::{SnapshotSource, ThreadResolution, TimelineEvent};
use argus_core::provider::{Anchor, Side, Thread};
use argus_core::{Core, LineStatus};

use support::*;

fn anchor(side: Side) -> Anchor {
    Anchor {
        path_old: Some(PATH.into()),
        path_new: Some(PATH.into()),
        line_old: None,
        line_new: None,
        start_line: None,
        side,
        base_sha: None,
        start_sha: None,
        head_sha: "c1".into(),
        diff_hunk: None,
        current_line: None,
    }
}

fn add_thread(p: &Platform, id: &str, anchor: Anchor, outdated: bool) {
    p.with(|w| {
        w.threads.push(Thread {
            id: id.into(),
            author: ME.into(),
            created_at: at(10, 13),
            resolved: false,
            outdated,
            anchor: Some(anchor),
            notes: vec![note(id, ME, at(10, 13), &format!("Comment {id}"))],
        })
    });
}

fn history(core: &Core, id: &str) -> argus_core::ThreadHistory {
    core.thread_history(thread_map(core)[id].thread_id).unwrap()
}

#[test]
fn a_comment_on_a_removed_line_shows_the_base_code() {
    let p = Platform::new();
    add_thread(
        &p,
        "4010",
        Anchor {
            line_old: Some(5),
            base_sha: Some("b0".into()),
            ..anchor(Side::Old)
        },
        false,
    );
    let core = core_over(&p);
    let h = history(&core, "4010");
    assert_eq!(
        h.status,
        LineStatus::Deleted {
            path: PATH.into(),
            removed_by: None
        }
    );
    let then = h.then_snapshot.unwrap();
    assert_eq!(
        (then.sha.as_str(), then.source),
        ("b0", SnapshotSource::File)
    );
    let i = (then.anchor_end.unwrap() - then.first_line) as usize;
    assert_eq!(then.lines[i], "line 5");
    assert!(h.now_snapshot.is_none());
    assert!(
        matches!(&h.timeline[0].event, TimelineEvent::Note { body, .. } if body == "Comment 4010")
    );
}

#[test]
fn without_the_base_the_diff_hunk_stands_in() {
    let p = Platform::new();
    add_thread(
        &p,
        "4011",
        Anchor {
            line_old: Some(5),
            diff_hunk: Some(
                "@@ -3,5 +3,5 @@\n line 3\n line 4\n-line 5\n+line five\n line 6".into(),
            ),
            ..anchor(Side::Old)
        },
        true,
    );
    let core = core_over(&p);
    let h = history(&core, "4011");
    assert!(h.outdated);
    let then = h.then_snapshot.expect("a snapshot from the hunk");
    assert_eq!(then.source, SnapshotSource::DiffHunk);
    assert!(then.lines.iter().any(|l| l == "line 5"));
}

#[test]
fn a_comment_on_a_whole_file_is_unlocated_with_a_reason() {
    let p = Platform::new();
    add_thread(&p, "4012", anchor(Side::New), false);
    let core = core_over(&p);
    let h = history(&core, "4012");
    assert_eq!(
        h.status,
        LineStatus::Unlocated {
            reason: "the comment is on the file, not on a line".into()
        }
    );
    assert!(h.then_snapshot.is_none());
    assert_eq!(h.timeline.len(), 1, "its note is still there");
}

#[test]
fn githubs_current_line_places_a_thread_the_walk_cannot() {
    let p = Platform::new();
    // Left on a commit the review no longer has (a force push), still
    // current on GitHub at line 12.
    add_thread(
        &p,
        "4013",
        Anchor {
            line_new: Some(12),
            head_sha: "f0rce".into(),
            current_line: Some(12),
            ..anchor(Side::New)
        },
        false,
    );
    let core = core_over(&p);
    let h = history(&core, "4013");
    assert!(
        matches!(h.status, LineStatus::Located { line: 12, .. }),
        "{:?}",
        h.status
    );
    assert_eq!(h.now_snapshot.unwrap().sha, "c1");
}

#[test]
fn resolve_events_join_the_timeline_and_a_failed_read_is_retried() {
    let p = Platform::new();
    p.with(|w| {
        let t = w.threads.iter_mut().find(|t| t.id == MY_THREAD).unwrap();
        t.resolved = true;
        w.resolutions = None;
    });
    let core = core_over(&p);
    // The resolve events cannot be read: the rest is there, not cached.
    let h = history(&core, MY_THREAD);
    assert!(matches!(h.status, LineStatus::Located { .. }));
    assert!(!h
        .timeline
        .iter()
        .any(|e| matches!(e.event, TimelineEvent::Resolved { .. })));

    p.with(|w| {
        w.resolutions = Some(vec![ThreadResolution {
            thread_id: MY_THREAD.into(),
            resolved: true,
            by: AUTHOR.into(),
            at: at(11, 9),
        }])
    });
    let h = history(&core, MY_THREAD);
    assert!(h
        .timeline
        .iter()
        .any(|e| matches!(&e.event, TimelineEvent::Resolved { by } if by == AUTHOR)));
}

#[test]
fn an_unreachable_platform_degrades_and_recovers() {
    let p = Platform::new();
    let core = core_over(&p);
    p.with(|w| w.push_commit("c2", 11));
    core.refresh();
    // The walk to c2 needs its diff, which cannot be read now.
    p.with(|w| w.offline = true);
    let h = history(&core, MY_THREAD);
    assert!(
        matches!(&h.status, LineStatus::Unlocated { reason } if reason.contains("try again")),
        "{:?}",
        h.status
    );
    assert!(h.then_snapshot.is_none());
    assert_eq!(h.timeline.len(), 1, "the notes are still there");
    // Nothing cached for the head: the snapshot for the AI prompt is empty.
    let id = thread_map(&core)[MY_THREAD].thread_id;
    assert!(core.cached_snapshot(id).unwrap().is_none());

    // Back online: computed again, not stuck on the failure.
    p.with(|w| w.offline = false);
    let h = history(&core, MY_THREAD);
    assert_eq!(
        h.status,
        LineStatus::Deleted {
            path: PATH.into(),
            removed_by: Some("c2".into())
        }
    );
    assert!(h.then_snapshot.is_some());
    assert!(core.cached_snapshot(id).unwrap().is_some());
}

#[test]
fn a_review_without_its_provider_still_shows_its_threads() {
    let p = Platform::new();
    let core = core_over(&p);
    // GitHub turned off: the providers are rebuilt without it.
    let mut s = core.settings();
    s.github.enabled = false;
    core.update_settings(s).unwrap();
    let id = the_review(&core).id;
    assert_eq!(core.threads(id).unwrap().len(), 2);
    let h = history(&core, MY_THREAD);
    assert!(matches!(h.status, LineStatus::Unlocated { .. }));
    assert_eq!(h.timeline.len(), 1);
    // The code view says why instead of failing silently.
    let err = core.list_files(id, false).unwrap_err();
    assert!(err.to_string().contains("not set up"), "{err}");
}
