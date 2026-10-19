use super::*;
use crate::testutil::{at, detail, note, thread, AUTHOR, ME};

const OTHER: &str = "dana-fixture";

/// Fetch → record, from `prev`.
fn step(
    prev: Option<&ReviewRecord>,
    d: &ReviewDetail,
    threads: &[Thread],
    now: DateTime<Utc>,
) -> ReviewRecord {
    advance(prev, d, threads, ME, true, now)
}

/// A review the user marked reviewed at head `h1` on day 10, 12:00.
fn reviewed() -> (ReviewRecord, ReviewDetail) {
    let d = detail("h1");
    let first = step(None, &d, &[], at(10, 10));
    (mark_reviewed(&first, &[], at(10, 12)), d)
}

#[test]
fn first_sight_needs_review() {
    let r = step(None, &detail("h1"), &[], at(10, 10));
    assert_eq!(r.status, ReviewStatus::NeedsReview);
    assert!(r.round_reasons.is_empty());
    assert_eq!(r.round, 1);
    assert!(r.is_active());
    assert_eq!(r.last_reviewed_sha, None);
}

#[test]
fn mark_reviewed_moves_the_watermark_and_waits_on_the_author() {
    let (r, d) = reviewed();
    assert_eq!(r.last_reviewed_sha.as_deref(), Some("h1"));
    // Dated by what the user saw: the fetch at 10:10, not the click.
    assert_eq!(r.last_reviewed_at, Some(at(10, 10)));
    assert_eq!(r.status, ReviewStatus::WaitingOnAuthor);
    // Still waiting after a refresh that brings nothing new, even though
    // the platform still lists the user as requested.
    let again = step(Some(&r), &d, &[], at(10, 15));
    assert_eq!(again.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(again.round, 1);
}

#[test]
fn new_commits_start_a_new_round() {
    let (r, _) = reviewed();
    let next = step(Some(&r), &detail("h2"), &[], at(11, 9));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::NewCommits]);
    assert_eq!(next.round, 2);

    // Staying in re-review over later polls does not bump the round again.
    let later = step(Some(&next), &detail("h3"), &[], at(11, 12));
    assert_eq!(later.round, 2);

    // Marked reviewed, then more commits: round 3.
    let done = mark_reviewed(&later, &[], at(11, 13));
    assert_eq!(done.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(done.last_reviewed_sha.as_deref(), Some("h3"));
    let round3 = step(Some(&done), &detail("h4"), &[], at(12, 9));
    assert_eq!(round3.round, 3);
}

#[test]
fn a_reply_in_my_thread_starts_a_new_round() {
    let (r, d) = reviewed();
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 11)), note("2", AUTHOR, at(11, 8))],
    );
    let next = step(Some(&r), &d, &[mine], at(11, 9));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn replies_that_do_not_count() {
    let (r, d) = reviewed();
    // Before the watermark.
    let old = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 8)), note("2", AUTHOR, at(10, 9))],
    );
    // In someone else's thread.
    let theirs = thread(
        "2",
        OTHER,
        "h1",
        vec![note("3", OTHER, at(11, 7)), note("4", AUTHOR, at(11, 8))],
    );
    // The user's own reply, and a system note.
    let mut system = note("6", AUTHOR, at(11, 8));
    system.system = true;
    let own = thread("3", ME, "h1", vec![note("5", ME, at(11, 7)), system]);
    let next = step(Some(&r), &d, &[old, theirs, own], at(11, 9));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn a_re_request_is_a_transition() {
    let (r, mut d) = reviewed();
    // The user reviewed on the platform: no longer requested.
    d.requested_from_me = false;
    let dropped = step(Some(&r), &d, &[], at(11, 8));
    assert_eq!(dropped.status, ReviewStatus::WaitingOnAuthor);
    assert!(!dropped.requested);

    d.requested_from_me = true;
    let again = step(Some(&dropped), &d, &[], at(11, 9));
    assert_eq!(again.status, ReviewStatus::NeedsReReview);
    assert_eq!(again.round_reasons, [RoundReason::ReRequested]);
    assert_eq!(again.rerequested_at, Some(at(11, 9)));

    // The reason sticks until the user marks the round reviewed.
    let later = step(Some(&again), &d, &[], at(11, 12));
    assert_eq!(later.round_reasons, [RoundReason::ReRequested]);
    let done = mark_reviewed(&later, &[], at(11, 13));
    assert_eq!(done.status, ReviewStatus::WaitingOnAuthor);
}

#[test]
fn several_reasons_at_once() {
    let (r, mut d) = reviewed();
    d.requested_from_me = false;
    let dropped = step(Some(&r), &d, &[], at(11, 8));
    let mut d = detail("h2");
    d.requested_from_me = true;
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![note("1", ME, at(10, 11)), note("2", AUTHOR, at(11, 8))],
    );
    let next = step(Some(&dropped), &d, &[mine], at(11, 9));
    assert_eq!(
        next.round_reasons,
        [
            RoundReason::NewCommits,
            RoundReason::AuthorReplied,
            RoundReason::ReRequested
        ]
    );
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round, 2);
}

#[test]
fn approval_removes_the_item() {
    let (r, mut d) = reviewed();
    d.approved_by_me = true;
    d.head_sha = "h2".into(); // even with new commits
    let approved = step(Some(&r), &d, &[], at(11, 9));
    assert_eq!(approved.status, ReviewStatus::Approved);
    assert_eq!(approved.approved_at, Some(at(11, 9)));
    assert!(approved.round_reasons.is_empty());
    assert!(!approved.is_active());

    // approved_at is set once, not moved by later polls.
    let later = step(Some(&approved), &d, &[], at(12, 9));
    assert_eq!(later.approved_at, Some(at(11, 9)));

    // Merged after approval stays approved.
    d.state = ReviewState::Merged;
    assert_eq!(
        step(Some(&later), &d, &[], at(12, 10)).status,
        ReviewStatus::Approved
    );

    // The approval went away on the platform (dismissed / revoked).
    let mut d = detail("h2");
    d.approved_by_me = false;
    let revoked = step(Some(&later), &d, &[], at(12, 11));
    assert_eq!(revoked.approved_at, None);
    assert_eq!(revoked.status, ReviewStatus::NeedsReReview);

    // Approved on first sight.
    let mut d = detail("h1");
    d.approved_by_me = true;
    assert_eq!(
        step(None, &d, &[], at(10, 10)).status,
        ReviewStatus::Approved
    );
}

#[test]
fn closed_or_merged_without_approval() {
    for state in [ReviewState::Closed, ReviewState::Merged] {
        let (r, mut d) = reviewed();
        d.state = state;
        d.head_sha = "h2".into();
        let closed = step(Some(&r), &d, &[], at(11, 9));
        assert_eq!(closed.status, ReviewStatus::ClosedWithoutApproval);
        assert!(closed.round_reasons.is_empty());
        assert!(closed.is_active());
    }
    // Also when never reviewed.
    let mut d = detail("h1");
    d.state = ReviewState::Closed;
    assert_eq!(
        step(None, &d, &[], at(10, 10)).status,
        ReviewStatus::ClosedWithoutApproval
    );
}

#[test]
fn dropped_from_the_list_is_not_active() {
    let r = advance(None, &detail("h1"), &[], ME, false, at(10, 10));
    assert_eq!(r.status, ReviewStatus::NeedsReview);
    assert!(!r.is_active());
}

#[test]
fn untouched_threads() {
    let (r, d) = reviewed();
    let quiet = thread("1", ME, "h1", vec![note("1", ME, at(10, 11))]);
    let answered = thread(
        "2",
        ME,
        "h1",
        vec![note("2", ME, at(10, 11)), note("3", AUTHOR, at(11, 8))],
    );
    // The author replied, then the user answered: back on the author.
    let answered_back = thread(
        "3",
        ME,
        "h1",
        vec![
            note("4", ME, at(10, 11)),
            note("5", AUTHOR, at(11, 8)),
            note("6", ME, at(11, 9)),
        ],
    );
    let mut resolved = thread("4", ME, "h1", vec![note("7", ME, at(10, 11))]);
    resolved.resolved = true;
    let theirs = thread("5", OTHER, "h1", vec![note("8", OTHER, at(10, 11))]);
    let all = [quiet, answered, answered_back, resolved, theirs];

    let rec = step(Some(&r), &d, &all, at(11, 10));
    assert_eq!(rec.untouched_threads, 2);

    // Any new commit counts as touching every thread (M2 approximation).
    let rec = step(Some(&r), &detail("h2"), &all, at(11, 10));
    assert_eq!(rec.untouched_threads, 0);

    // Before any watermark: unchanged since the commit the thread was left on.
    let fresh = step(None, &detail("h1"), &all[..1], at(10, 12));
    assert_eq!(fresh.untouched_threads, 1);
    let moved = step(None, &detail("h2"), &all[..1], at(10, 12));
    assert_eq!(moved.untouched_threads, 0);
}

#[test]
fn reasons_and_statuses_round_trip_their_keys() {
    for r in [
        RoundReason::NewCommits,
        RoundReason::AuthorReplied,
        RoundReason::ReRequested,
    ] {
        assert_eq!(RoundReason::parse(r.as_str()), Some(r));
        assert_eq!(serde_json::to_value(r).unwrap(), r.as_str());
    }
    for s in [
        ReviewStatus::NeedsReview,
        ReviewStatus::NeedsReReview,
        ReviewStatus::WaitingOnAuthor,
        ReviewStatus::ClosedWithoutApproval,
        ReviewStatus::Approved,
    ] {
        assert_eq!(ReviewStatus::parse(s.as_str()), Some(s));
        assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
    }
}

#[test]
fn untouched_threads_with_line_walks() {
    use crate::history::{LineTouch, LineTouches};

    let (r, _) = reviewed();
    let quiet = thread("1", ME, "h1", vec![note("1", ME, at(10, 11))]);
    let rewritten = thread("2", ME, "h1", vec![note("2", ME, at(10, 11))]);
    let unwalked = thread("3", ME, "h1", vec![note("3", ME, at(10, 11))]);
    let all = [quiet, rewritten, unwalked];
    let touch = |touched_by: &[&str], usable| LineTouch {
        then_sha: "h1".into(),
        commits: ["h1", "h2", "h3"].map(String::from).to_vec(),
        touched_by: touched_by.iter().map(|s| s.to_string()).collect(),
        usable,
    };
    let touches: LineTouches = [
        ("1".to_string(), touch(&[], true)),
        ("2".to_string(), touch(&["h3"], true)),
    ]
    .into();
    let input = |head: &'static str, last: Option<&'static str>| RoundInput {
        state: ReviewState::Open,
        head_sha: head,
        last_reviewed_sha: last,
        last_reviewed_at: r.last_reviewed_at,
        approved_at: None,
        rerequested_at: None,
        threads: &all,
        me: ME,
    };

    // Watermark h1, head h3: the walk says thread 1's lines were left
    // alone; thread 2's were rewritten in h3; thread 3 has no walk and
    // falls back to "any new commit touches it".
    assert_eq!(untouched_with(&input("h3", Some("h1")), Some(&touches)), 1);
    // Without walks: the M2 approximation, nothing is untouched.
    assert_eq!(untouched(&input("h3", Some("h1"))), 0);
    // Watermark at h3: h3's rewrite is no longer "since".
    assert_eq!(untouched_with(&input("h3", Some("h3")), Some(&touches)), 3);
    // A walk that cannot answer falls back as well.
    let unusable: LineTouches = [("1".to_string(), touch(&[], false))].into();
    assert_eq!(untouched_with(&input("h3", Some("h1")), Some(&unusable)), 0);
    // A note from someone else still counts, walk or not.
    let mut answered = all.clone();
    answered[0].notes.push(note("9", AUTHOR, at(11, 9)));
    let input = RoundInput {
        threads: &answered,
        ..input("h3", Some("h1"))
    };
    assert_eq!(untouched_with(&input, Some(&touches)), 0);
}

#[test]
fn a_reply_between_the_last_fetch_and_the_click_still_counts() {
    // Fetched at 10:10 with the user's own thread; the author replies at
    // 10:30 (not fetched yet); the user marks it reviewed at 11:00.
    let d = detail("h1");
    let mine = thread("1", ME, "h1", vec![note("1", ME, at(10, 9))]);
    let fetched = step(None, &d, std::slice::from_ref(&mine), at(10, 10));
    let done = mark_reviewed(&fetched, std::slice::from_ref(&mine), at(11, 0));
    assert_eq!(done.last_reviewed_at, Some(at(10, 10)));
    assert_eq!(done.status, ReviewStatus::WaitingOnAuthor);

    let mut replied = mine.clone();
    replied.notes.push(note(
        "2",
        AUTHOR,
        at(10, 10) + chrono::Duration::minutes(20),
    ));
    let next = step(Some(&done), &d, &[replied], at(11, 3));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn a_reply_a_fetch_missed_is_never_taken_as_seen() {
    // The poll started at 10:10 and read the thread pages one by one: the
    // author replied at 10:10:30 on a page already read (missed), and a
    // note from 10:11 on a later page made it in. The watermark is the
    // fetch's start, so the missed reply still counts once fetched.
    let d = detail("h1");
    let mine = thread("1", ME, "h1", vec![note("1", ME, at(10, 9))]);
    let other = thread(
        "2",
        AUTHOR,
        "h1",
        vec![note("2", AUTHOR, at(10, 10) + chrono::Duration::minutes(1))],
    );
    let stored = [mine.clone(), other.clone()];
    let fetched = step(None, &d, &stored, at(10, 10));
    let done = mark_reviewed(&fetched, &stored, at(12, 0));
    assert_eq!(done.last_reviewed_at, Some(at(10, 10)));

    let mut replied = mine;
    replied.notes.push(note(
        "3",
        AUTHOR,
        at(10, 10) + chrono::Duration::seconds(30),
    ));
    let next = step(Some(&done), &d, &[replied, other], at(12, 5));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
}

#[test]
fn the_watermark_is_the_last_fetch_and_never_after_the_click() {
    let d = detail("h1");
    let fetched = step(None, &d, &[], at(10, 10));
    assert_eq!(
        mark_reviewed(&fetched, &[], at(12, 0)).last_reviewed_at,
        Some(at(10, 10))
    );
    let click = at(10, 10) - chrono::Duration::seconds(30);
    assert_eq!(
        mark_reviewed(&fetched, &[], click).last_reviewed_at,
        Some(click)
    );
}

#[test]
fn a_thread_i_replied_in_is_mine_for_the_round() {
    let (r, d) = reviewed();
    // A colleague's thread: their note and the author's after the
    // watermark, then the user's reply. Only what comes after the user's
    // own last note is new to them.
    let mut replied = thread(
        "1",
        OTHER,
        "h1",
        vec![
            note("1", OTHER, at(10, 9)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    );
    let quiet = step(Some(&r), &d, std::slice::from_ref(&replied), at(11, 9));
    assert_eq!(quiet.status, ReviewStatus::WaitingOnAuthor);
    // Untouched since the user's reply: it counts like an own thread.
    assert_eq!(quiet.untouched_threads, 1);

    replied.notes.push(note("4", AUTHOR, at(11, 10)));
    let next = step(Some(&quiet), &d, &[replied], at(11, 11));
    assert_eq!(next.status, ReviewStatus::NeedsReReview);
    assert_eq!(next.round_reasons, [RoundReason::AuthorReplied]);
    assert_eq!(next.untouched_threads, 0);
}

#[test]
fn a_system_note_of_mine_does_not_make_a_thread_mine() {
    let (r, d) = reviewed();
    // The user resolved a colleague's thread; the author answered after.
    let mut resolve = note("2", ME, at(11, 7));
    resolve.system = true;
    let t = thread(
        "1",
        OTHER,
        "h1",
        vec![
            note("1", OTHER, at(10, 9)),
            resolve,
            note("3", AUTHOR, at(11, 8)),
        ],
    );
    let next = step(Some(&r), &d, &[t], at(11, 9));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(next.untouched_threads, 0);
}

#[test]
fn a_reply_the_user_already_answered_is_not_new() {
    let (r, d) = reviewed();
    // In the user's own thread: the author replied after the watermark and
    // the user answered that before the fetch.
    let mine = thread(
        "1",
        ME,
        "h1",
        vec![
            note("1", ME, at(10, 11)),
            note("2", AUTHOR, at(11, 7)),
            note("3", ME, at(11, 8)),
        ],
    );
    let next = step(Some(&r), &d, &[mine], at(11, 9));
    assert_eq!(next.status, ReviewStatus::WaitingOnAuthor);
}
