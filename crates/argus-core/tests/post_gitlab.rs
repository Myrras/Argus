//! GitLab writes through the fake `glab` of `support::glab` against the
//! local HTTP double of `support::gitlab`, which records each request's
//! method, path, token header and body, and answers reads from
//! `tests/fixtures/gitlab/`. No network, no real token. Unix-only: the fake
//! is a bash script.

mod support;

use std::sync::Arc;
use std::time::Duration;

use argus_core::post::{DiffLine, LineKind, OutsideDiff, PlacedComment, PostErrorKind};
use argus_core::provider::{MyReviewState, ReviewProvider, ReviewVerdict, Side};
use argus_core::store::Store;
use argus_core::{Core, NewComment, PostResult, ReviewStatus};
use serde_json::{json, Value};

use support::gitlab::*;

fn row(kind: LineKind, old_pos: u32, new_pos: u32) -> DiffLine {
    DiffLine {
        kind,
        old_pos,
        new_pos,
    }
}

fn placed(path: &str, side: Side, line: DiffLine, start: Option<DiffLine>) -> PlacedComment {
    PlacedComment {
        path: path.into(),
        old_path: path.into(),
        side,
        line,
        start,
        in_diff: true,
        excerpt: None,
        head_sha: HEAD_SHA.into(),
        base_sha: BASE_SHA.into(),
        body: "Why three?".into(),
    }
}

/// sha1("src/importer.rs") (from `shasum -a 1`), which GitLab's line codes
/// for that file start with.
const IMPORTER_SHA1: &str = "f98efccf8a5b8ca0e6bd59bffd9fd43cbc9b45b6";

#[test]
#[cfg_attr(windows, ignore)]
fn posts_an_added_line_with_the_latest_versions_position() {
    let (base, log) = serve(instance);
    let gl = provider(&base);
    let c = placed(
        "src/importer.rs",
        Side::New,
        row(LineKind::Added, 3, 2),
        None,
    );
    let posted = gl.post_comment(&mr(&base), &c).unwrap();
    assert_eq!(
        posted.provider_thread_id,
        "d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9"
    );
    assert_eq!(posted.provider_note_id, "9001");
    assert_eq!(
        posted.url,
        "https://gitlab.example.test/grp/app/-/merge_requests/21#note_9001"
    );

    // The versions were read right before the post.
    let seen = log.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path.as_str()),
        ("GET", &*format!("{MR21}/versions"))
    );
    let post = &seen[1];
    assert_eq!(post.path, format!("{MR21}/discussions"));
    assert!(post.token);
    assert_eq!(
        post.content_type.as_deref(),
        Some("application/json; charset=utf-8")
    );
    assert!(!post.body.contains(TOKEN));
    assert_eq!(
        post.json(),
        json!({
            "body": "Why three?",
            "position": {
                "position_type": "text",
                "base_sha": BASE_SHA,
                "start_sha": BASE_SHA,
                "head_sha": HEAD_SHA,
                "old_path": "src/importer.rs",
                "new_path": "src/importer.rs",
                "new_line": 2,
            }
        })
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn removed_context_and_renamed_lines_get_their_sides() {
    let (base, log) = serve(instance);
    let gl = provider(&base);
    let r = mr(&base);
    gl.post_comment(
        &r,
        &placed(
            "src/importer.rs",
            Side::Old,
            row(LineKind::Removed, 2, 2),
            None,
        ),
    )
    .unwrap();
    gl.post_comment(
        &r,
        &placed(
            "src/importer.rs",
            Side::New,
            row(LineKind::Context, 3, 3),
            None,
        ),
    )
    .unwrap();
    let mut renamed = placed(
        "src/new_name.rs",
        Side::Old,
        row(LineKind::Removed, 4, 4),
        None,
    );
    renamed.old_path = "src/old_name.rs".into();
    gl.post_comment(&r, &renamed).unwrap();

    let bodies: Vec<Value> = posts(&log)
        .iter()
        .map(|p| p.json()["position"].clone())
        .collect();
    // Removed: old_line only.
    assert_eq!(bodies[0]["old_line"], 2);
    assert!(bodies[0].get("new_line").is_none());
    // Unchanged: both.
    assert_eq!(
        (bodies[1]["old_line"].clone(), bodies[1]["new_line"].clone()),
        (json!(3), json!(3))
    );
    // Renamed: each path on its side.
    assert_eq!(bodies[2]["old_path"], "src/old_name.rs");
    assert_eq!(bodies[2]["new_path"], "src/new_name.rs");
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_range_carries_a_line_range_with_line_codes() {
    let (base, log) = serve(instance);
    let c = placed(
        "src/importer.rs",
        Side::New,
        row(LineKind::Context, 3, 3),
        Some(row(LineKind::Added, 3, 2)),
    );
    provider(&base).post_comment(&mr(&base), &c).unwrap();
    let position = posts(&log)[0].json()["position"].clone();
    let sha = IMPORTER_SHA1;
    assert_eq!(position["old_line"], 3);
    assert_eq!(position["new_line"], 3);
    assert_eq!(
        position["line_range"],
        json!({
            "start": {
                "line_code": format!("{sha}_3_2"),
                "type": "new",
                "old_line": null,
                "new_line": 2,
            },
            "end": {
                "line_code": format!("{sha}_3_3"),
                "type": "old",
                "old_line": 3,
                "new_line": 3,
            }
        })
    );
}

/// A suggestion is markdown: the fence and its `-N+0` go out as written,
/// N counted back from the note's line, the last of its `line_range`.
#[test]
#[cfg_attr(windows, ignore)]
fn a_suggestion_goes_out_unchanged_on_its_line_range() {
    let (base, log) = serve(instance);
    let body = "```suggestion:-1+0\nlet a = 1;\nlet b = 2;\n```";
    let c = PlacedComment {
        body: body.into(),
        ..placed(
            "src/importer.rs",
            Side::New,
            row(LineKind::Context, 3, 3),
            Some(row(LineKind::Added, 3, 2)),
        )
    };
    provider(&base).post_comment(&mr(&base), &c).unwrap();
    provider(&base)
        .reply(
            &mr(&base),
            "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1",
            "````suggestion:-0+0\n```\n````",
        )
        .unwrap();
    let sent = posts(&log);
    let first = sent[0].json();
    assert_eq!(first["body"], body);
    let position = &first["position"];
    // The note sits on line 3; -1 reaches line 2, the range's start.
    assert_eq!(position["new_line"], 3);
    assert_eq!(position["line_range"]["start"]["new_line"], 2);
    assert_eq!(position["line_range"]["end"]["new_line"], 3);
    // A reply takes its discussion's position.
    assert_eq!(
        sent[1].json(),
        json!({ "body": "````suggestion:-0+0\n```\n````" })
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_moved_head_fails_before_posting() {
    let (base, log) = serve(instance);
    let mut c = placed(
        "src/importer.rs",
        Side::New,
        row(LineKind::Added, 3, 2),
        None,
    );
    c.head_sha = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1".into();
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    assert!(posts(&log).is_empty());
}

fn failing(status: &'static str, body: &'static str) -> (String, Log) {
    serve(move |req| match req.method.as_str() {
        "POST" => reply(status, body),
        _ => instance(req),
    })
}

#[test]
#[cfg_attr(windows, ignore)]
fn surfaces_gitlabs_error_message_verbatim() {
    let c = placed(
        "src/importer.rs",
        Side::New,
        row(LineKind::Context, 3, 3),
        None,
    );
    let (base, _) = failing(
        "400 Bad Request",
        r#"{"message":"400 Bad request - Note {:line_code=>[\"can't be blank\", \"must be a valid line code\"]}"}"#,
    );
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(
        err.message,
        r#"GitLab: 400 Bad request - Note {:line_code=>["can't be blank", "must be a valid line code"]}"#
    );

    // A field → errors object; the token never comes back.
    let (base, _) = failing(
        "422 Unprocessable Entity",
        r#"{"message":{"note":["is too long"],"base":["glpat-fixture-not-a-real-token is bad"]}}"#,
    );
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(
        err.message,
        "GitLab: base: [redacted] is bad; note: is too long"
    );

    // A read-only token.
    let (base, _) = failing(
        "403 Forbidden",
        r#"{"error":"insufficient_scope","error_description":"The request requires higher privileges than provided by the access token."}"#,
    );
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert!(err
        .message
        .starts_with("GitLab: insufficient_scope: The request requires higher"));

    // A token GitLab rejects; a server error page that is not JSON.
    let (base, _) = failing("401 Unauthorized", r#"{"message":"401 Unauthorized"}"#);
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::GitlabAuth);
    let (base, _) = failing("500 Internal Server Error", "<html>oops glpat</html>");
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::Provider);
    assert_eq!(err.message, "GitLab: could not post (HTTP 500)");
}

#[test]
#[cfg_attr(windows, ignore)]
fn replies_to_a_discussion() {
    let (base, log) = serve(instance);
    let note = provider(&base)
        .reply(
            &mr(&base),
            "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1",
            "Done",
        )
        .unwrap();
    assert_eq!(note.provider_note_id, "9002");
    assert!(note.url.ends_with("/merge_requests/21#note_9002"));
    let p = &posts(&log)[0];
    assert_eq!(
        p.path,
        format!("{MR21}/discussions/d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1/notes")
    );
    assert_eq!(p.json(), json!({ "body": "Done" }));
    assert!(p.token);
}

#[test]
#[cfg_attr(windows, ignore)]
fn approves_and_explains_a_refusal() {
    let (base, log) = serve(instance);
    provider(&base).approve(&mr(&base), HEAD_SHA).unwrap();
    let p = &posts(&log)[0];
    assert_eq!(p.path, format!("{MR21}/approve"));
    assert_eq!(p.json(), json!({ "sha": HEAD_SHA }));
    assert!(p.token);

    let (base, _) = failing("401 Unauthorized", r#"{"message":"401 Unauthorized"}"#);
    let err = provider(&base).approve(&mr(&base), HEAD_SHA).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert!(err.message.contains("already approved"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn an_approval_of_a_moved_head_is_refused() {
    // GitLab answers 409 when `sha` is not the source branch's head.
    let (base, _) = failing(
        "409 Conflict",
        r#"{"message":"SHA does not match HEAD of source branch"}"#,
    );
    let err = provider(&base).approve(&mr(&base), HEAD_SHA).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    assert!(!err.message.contains(TOKEN));
    // A 409 on another write stays a rejection with GitLab's reason.
    let c = placed(
        "src/importer.rs",
        Side::New,
        row(LineKind::Context, 3, 3),
        None,
    );
    let err = provider(&base).post_comment(&mr(&base), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
}

#[test]
#[cfg_attr(windows, ignore)]
fn posting_and_approving_through_the_core() {
    let (base, log) = serve(instance);
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    assert_eq!(core.review(id).unwrap().status, ReviewStatus::NeedsReview);

    let lines = core.commentable_lines(id, "src/importer.rs").unwrap();
    assert_eq!(lines.hunks[0].new, [1, 2, 3]);
    assert_eq!(lines.hunks[0].old, [1, 2, 3]);

    let comment = |line, side| NewComment {
        path: "src/importer.rs".into(),
        line,
        side,
        start_line: None,
        body: "Why three?".into(),
        head_sha: None,
        base_sha: None,
    };
    // GitLab takes unchanged lines outside the hunks too.
    assert_eq!(lines.outside, OutsideDiff::Native);
    // A range from the hunk into the code below it is two regions.
    let across = NewComment {
        start_line: Some(2),
        ..comment(9, Side::New)
    };
    let results = core
        .post_comments(
            id,
            vec![
                comment(2, Side::New),
                comment(9, Side::New),
                comment(2, Side::Old),
                across,
            ],
        )
        .unwrap();
    assert!(results[0].is_posted() && results[1].is_posted() && results[2].is_posted());
    let PostResult::Failed(e) = &results[3] else {
        panic!("{results:?}")
    };
    assert_eq!(e.kind, PostErrorKind::InvalidInput);
    assert!(e.message.contains("outside the diff"), "{}", e.message);

    let bodies: Vec<Value> = posts(&log).iter().map(|p| p.json()).collect();
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[0]["position"]["new_line"], 2);
    assert!(bodies[0]["position"].get("old_line").is_none());
    // Outside the diff: an unchanged line, both numbers (the hunk keeps
    // the count, so they are equal here).
    assert_eq!(bodies[1]["position"]["new_line"], 9);
    assert_eq!(bodies[1]["position"]["old_line"], 9);
    assert_eq!(bodies[2]["position"]["old_line"], 2);
    assert_eq!(bodies[0]["position"]["head_sha"], HEAD_SHA);

    // Posting moves the watermark; the review stays the user's while
    // GitLab still awaits their review (only a submitted review sets it).
    let after = core.review(id).unwrap();
    assert_eq!(after.last_reviewed_sha.as_deref(), Some(HEAD_SHA));
    assert_eq!(after.status, ReviewStatus::NeedsReview);

    let approved = core.approve_review(id, HEAD_SHA).unwrap();
    assert!(approved.approved_at.is_some());
    assert!(!approved.is_active());
    let approval = posts(&log).last().unwrap().clone();
    assert_eq!(approval.path, format!("{MR21}/approve"));
    assert_eq!(approval.json(), json!({ "sha": HEAD_SHA }));
    // Every request carried the token, and no body did.
    let seen = log.lock().unwrap();
    assert!(seen.iter().all(|r| r.token && !r.body.contains(TOKEN)));
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_push_between_reading_the_head_and_the_diff_fails_the_post_and_refreshes() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const PUSHED: &str = "a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3";
    // Once armed, the push lands right before the second versions read:
    // the post read the head (its first), the diff reads the second.
    let armed = Arc::new(AtomicBool::new(false));
    let versions_read = Arc::new(AtomicUsize::new(0));
    let (a, v) = (armed.clone(), versions_read.clone());
    let (base, log) = serve(move |req| {
        let route = req
            .path
            .split_once('?')
            .map_or(req.path.as_str(), |(r, _)| r);
        let versions = route == format!("{MR21}/versions");
        if !a.load(Ordering::SeqCst) {
            return instance(req);
        }
        if versions {
            v.fetch_add(1, Ordering::SeqCst);
        }
        if v.load(Ordering::SeqCst) < 2 {
            return instance(req);
        }
        if route == MR21 {
            return reply("200 OK", fixture("mr_21.json").replace(HEAD_SHA, PUSHED));
        }
        if versions {
            let newest = format!(
                r#"[{{"id":803,"head_commit_sha":"{PUSHED}","base_commit_sha":"{BASE_SHA}","start_commit_sha":"{BASE_SHA}","created_at":"2026-10-11T08:00:00.000Z","merge_request_id":9021,"state":"collected","real_size":"3"}},"#
            );
            let rest = fixture("mr_21_versions.json");
            return reply(
                "200 OK",
                format!("{newest}{}", rest.trim_start().trim_start_matches('[')),
            );
        }
        instance(req)
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    assert_eq!(core.review(id).unwrap().head_sha, HEAD_SHA);

    armed.store(true, Ordering::SeqCst);
    let err = core
        .post_comments(
            id,
            vec![NewComment {
                path: "src/importer.rs".into(),
                line: 2,
                side: Side::New,
                start_line: None,
                body: "Why three?".into(),
                head_sha: None,
                base_sha: None,
            }],
        )
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::HeadMoved);
    assert!(posts(&log).is_empty());
    // The refusal stored the new head: a retry starts from it.
    assert_eq!(core.review(id).unwrap().head_sha, PUSHED);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_failing_reviewers_call_does_not_block_approving() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let down = Arc::new(AtomicBool::new(false));
    let flag = down.clone();
    let (base, log) = serve(move |req| {
        let route = req
            .path
            .split_once('?')
            .map_or(req.path.as_str(), |(r, _)| r);
        if flag.load(Ordering::SeqCst) && route == format!("{MR21}/reviewers") {
            return reply("500 Internal Server Error", "{}");
        }
        instance(req)
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;

    // The reviewer states are down: the head check before approving only
    // needs the MR's head, so the approval goes through.
    down.store(true, Ordering::SeqCst);
    core.approve_review(id, HEAD_SHA).unwrap();
    let approval = posts(&log).last().unwrap().clone();
    assert_eq!(approval.path, format!("{MR21}/approve"));
}

// --- Resolving discussions ---

const D1: &str = "d1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
#[cfg_attr(windows, ignore)]
fn resolves_and_reopens_a_discussion_with_a_put() {
    let (base, log) = serve(instance);
    let gl = provider(&base);
    gl.set_thread_resolved(&mr(&base), D1, true).unwrap();
    gl.set_thread_resolved(&mr(&base), D1, false).unwrap();
    let sent = writes(&log);
    assert_eq!(sent.len(), 2);
    for (req, resolved) in sent.iter().zip([true, false]) {
        assert_eq!(req.method, "PUT");
        assert_eq!(req.path, format!("{MR21}/discussions/{D1}"));
        assert_eq!(req.json(), json!({ "resolved": resolved }));
        assert_eq!(
            req.content_type.as_deref(),
            Some("application/json; charset=utf-8")
        );
        assert!(req.token);
    }
    // The discussion id is one path segment, whatever it holds.
    gl.set_thread_resolved(&mr(&base), "d1/../../x", true)
        .unwrap();
    assert_eq!(
        writes(&log)[2].path,
        format!("{MR21}/discussions/d1%2F..%2F..%2Fx")
    );
    // Nothing to name: refused before any request.
    let err = gl.set_thread_resolved(&mr(&base), " ", true).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::InvalidInput);
    assert_eq!(writes(&log).len(), 3);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_refused_resolve_surfaces_gitlabs_message() {
    let refuse = |status: &'static str, body: &'static str| {
        serve(move |req| match req.method.as_str() {
            "PUT" => reply(status, body),
            _ => instance(req),
        })
    };
    // Only the MR author and maintainers may resolve on this instance.
    let (base, _) = refuse("403 Forbidden", r#"{"message":"403 Forbidden"}"#);
    let err = provider(&base)
        .set_thread_resolved(&mr(&base), D1, true)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(err.message, "GitLab: 403 Forbidden");
    // A discussion that cannot be resolved; the token never comes back.
    let (base, _) = refuse(
        "400 Bad Request",
        r#"{"message":"400 Bad request - glpat-fixture-not-a-real-token not resolvable"}"#,
    );
    let err = provider(&base)
        .set_thread_resolved(&mr(&base), D1, true)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(
        err.message,
        "GitLab: 400 Bad request - [redacted] not resolvable"
    );
    // A token GitLab rejects is an auth problem, not a refusal.
    let (base, _) = refuse("401 Unauthorized", r#"{"message":"401 Unauthorized"}"#);
    let err = provider(&base)
        .set_thread_resolved(&mr(&base), D1, true)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::GitlabAuth);
}

#[test]
#[cfg_attr(windows, ignore)]
fn resolving_through_the_core_writes_once_and_refreshes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let resolved = Arc::new(AtomicBool::new(false));
    let flag = resolved.clone();
    // A stateful instance: the PUT resolves `d1`, later reads show it.
    let (base, log) = serve(move |req| {
        let (route, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
        match (req.method.as_str(), route) {
            ("PUT", r) if r == format!("{MR21}/discussions/{D1}") => {
                let body: Value = serde_json::from_str(&req.body).unwrap();
                flag.store(body["resolved"] == json!(true), Ordering::SeqCst);
                instance(req)
            }
            ("GET", r) if r == format!("{MR21}/discussions") && !query.contains("page=2") => {
                let mut page: Value =
                    serde_json::from_str(&fixture("mr_21_discussions_page1.json")).unwrap();
                for n in page[0]["notes"].as_array_mut().unwrap() {
                    n["resolved"] = flag.load(Ordering::SeqCst).into();
                }
                Reply {
                    headers: "X-Next-Page: 2\r\n".into(),
                    ..reply("200 OK", page.to_string())
                }
            }
            _ => instance(req),
        }
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    let thread = |key: &str| {
        core.threads(id)
            .unwrap()
            .into_iter()
            .find(|t| t.thread.id.starts_with(key))
            .unwrap()
    };
    let d1 = thread("d1");
    assert!(d1.thread.resolvable && !d1.thread.resolved);
    let before = log.lock().unwrap().len();

    let after = core.resolve_thread(id, d1.thread_id, true).unwrap();
    // One write, then the review read again: the store has the new state.
    assert_eq!(writes(&log).len(), 1);
    assert!(after.thread.resolved);
    assert!(thread("d1").thread.resolved);
    let reads_after: Vec<String> = log.lock().unwrap()[before + 1..]
        .iter()
        .map(|r| r.path.clone())
        .collect();
    assert!(reads_after
        .iter()
        .any(|p| p.starts_with(&format!("{MR21}/discussions"))));

    // Reopened the same way.
    let after = core.resolve_thread(id, d1.thread_id, false).unwrap();
    assert!(!after.thread.resolved);
    assert_eq!(writes(&log).len(), 2);

    // A single comment cannot be resolved: refused, nothing written.
    let d5 = thread("d5");
    assert!(!d5.thread.resolvable);
    let err = core.resolve_thread(id, d5.thread_id, true).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::InvalidInput);
    // A thread of another review: refused as well.
    let other = argus_core::ReviewId::from_key("gitlab|elsewhere|grp/app|99");
    let err = core.resolve_thread(other, d1.thread_id, true).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::InvalidInput);
    assert_eq!(writes(&log).len(), 2);
}

// ---------------------------------------------------------------------------
// Submit review: approve / request changes, with a summary note.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(windows, ignore)]
fn requests_changes_with_the_quick_action_then_posts_the_summary() {
    let (base, log) = submitting(SUBMIT);
    provider(&base)
        .submit_review(
            &mr(&base),
            ReviewVerdict::RequestChanges,
            HEAD_SHA,
            Some("Please add the retry test."),
        )
        .unwrap();
    let posts = posts(&log);
    let sent: Vec<(String, Value)> = posts.iter().map(|p| (p.path.clone(), p.json())).collect();
    assert_eq!(
        sent,
        [
            (
                format!("{MR21}/notes"),
                json!({ "body": "/request_changes" })
            ),
            (
                format!("{MR21}/notes"),
                json!({ "body": "Please add the retry test." })
            ),
        ]
    );
    assert!(posts.iter().all(|p| p.token));
    // The version and the reviewer were read before the write.
    let seen = log.lock().unwrap();
    let first_post = seen.iter().position(|r| r.method == "POST").unwrap();
    assert!(seen[..first_post]
        .iter()
        .any(|r| r.path == "/api/v4/version"));
    assert!(seen[..first_post]
        .iter()
        .any(|r| r.path.starts_with(&format!("{MR21}/reviewers"))));
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_gitlab_without_request_changes_is_refused_before_any_write() {
    let (base, log) = submitting(Submit {
        version: "17.1.4-ee",
        ..SUBMIT
    });
    let err = provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::Unsupported);
    assert!(err
        .message
        .contains("Your GitLab doesn't support requesting changes via the API"));
    assert!(posts(&log).is_empty());

    // Not a reviewer either: still refused before the add.
    let (base, log) = submitting(Submit {
        version: "17.1.4-ee",
        reviewer: false,
        ..SUBMIT
    });
    let err = provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::Unsupported);
    assert!(posts(&log).is_empty());
}

/// The user is not a reviewer yet: Argus adds them (GraphQL
/// `mergeRequestSetReviewers`, `APPEND`), reads `/reviewers` until they are
/// listed, then requests changes as for a reviewer.
#[test]
#[cfg_attr(windows, ignore)]
fn a_non_reviewer_is_added_then_requests_changes() {
    let (base, log) = submitting(Submit {
        reviewer: false,
        ..SUBMIT
    });
    provider(&base)
        .submit_review(
            &mr(&base),
            ReviewVerdict::RequestChanges,
            HEAD_SHA,
            Some("Please add the retry test."),
        )
        .unwrap();
    assert_eq!(
        sequence(&log),
        [
            "GET /api/v4/version".to_string(),
            "GET /api/v4/user".into(),
            format!("GET {MR21}/reviewers"),
            format!("POST {GRAPHQL}"),
            format!("GET {MR21}/reviewers"),
            format!("POST {MR21}/notes"),
            format!("GET {MR21}/reviewers"),
            format!("POST {MR21}/notes"),
        ]
    );
    let posts = posts(&log);
    // The add: GraphQL through glab (its login as `PRIVATE-TOKEN`, as for
    // REST), the user appended by name.
    let add = &posts[0];
    assert!(add.token);
    assert_eq!(
        add.content_type.as_deref(),
        Some("application/json; charset=utf-8")
    );
    let body = add.json();
    let query = body["query"].as_str().unwrap();
    assert!(query.contains("mergeRequestSetReviewers("));
    assert!(query.contains("operationMode: APPEND"));
    assert_eq!(
        body["variables"],
        json!({ "projectPath": "grp/app", "iid": "21", "usernames": ["rev-fixture"] })
    );
    assert!(!add.body.contains(TOKEN));
    // Then the change request and the summary, as for a reviewer.
    let notes: Vec<Value> = posts[1..].iter().map(Request::json).collect();
    assert_eq!(
        notes,
        [
            json!({ "body": "/request_changes" }),
            json!({ "body": "Please add the retry test." }),
        ]
    );
    assert!(posts[1..].iter().all(|p| p.token));
}

#[test]
#[cfg_attr(windows, ignore)]
fn an_existing_reviewer_is_not_added_again() {
    let (base, log) = submitting(SUBMIT);
    provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap();
    assert_eq!(
        sequence(&log),
        [
            "GET /api/v4/version".to_string(),
            "GET /api/v4/user".into(),
            format!("GET {MR21}/reviewers"),
            format!("POST {MR21}/notes"),
            format!("GET {MR21}/reviewers"),
        ]
    );
}

/// GitLab lists the new reviewer a read or two later: Argus reads again,
/// a bounded number of times.
#[test]
#[cfg_attr(windows, ignore)]
fn the_add_is_read_back_until_gitlab_lists_the_user() {
    let (base, log) = submitting(Submit {
        reviewer: false,
        lag: 2,
        ..SUBMIT
    });
    provider(&base)
        .with_reviewer_recheck(&[Duration::ZERO; 4])
        .submit_review(&mr(&base), ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap();
    let seq = sequence(&log);
    let add = seq
        .iter()
        .position(|r| r == &format!("POST {GRAPHQL}"))
        .unwrap();
    let note = seq
        .iter()
        .position(|r| r == &format!("POST {MR21}/notes"))
        .unwrap();
    assert_eq!(seq[add + 1..note], vec![format!("GET {MR21}/reviewers"); 3]);

    // Never listed: an error after the last read, and no change request.
    let (base, log) = submitting(Submit {
        reviewer: false,
        lag: 10,
        ..SUBMIT
    });
    let err = provider(&base)
        .with_reviewer_recheck(&[Duration::ZERO; 3])
        .submit_review(&mr(&base), ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert!(
        err.message.contains("couldn't add you as a reviewer"),
        "{}",
        err.message
    );
    let seq = sequence(&log);
    let reads = seq
        .iter()
        .filter(|r| *r == &format!("GET {MR21}/reviewers"))
        .count();
    assert_eq!(reads, 1 + 3);
    assert_eq!(writes(&log).len(), 1);
}

/// An add GitLab refuses says why and what to do; nothing else is written.
#[test]
#[cfg_attr(windows, ignore)]
fn an_add_gitlab_refuses_is_a_clear_error() {
    for (add, says) in [
        (Add::Refused, "you don't have permission"),
        (Add::Fails, "Reviewers not able to be set: rev-fixture"),
        (Add::OneReviewer, "only one reviewer"),
        (Add::Ignored, "left the reviewers unchanged"),
    ] {
        let (base, log) = submitting(Submit {
            reviewer: false,
            add,
            ..SUBMIT
        });
        let err = provider(&base)
            .with_reviewer_recheck(&[Duration::ZERO])
            .submit_review(
                &mr(&base),
                ReviewVerdict::RequestChanges,
                HEAD_SHA,
                Some("Summary"),
            )
            .unwrap_err();
        assert_eq!(err.kind, PostErrorKind::PostRejected, "{add:?}");
        assert!(
            err.message
                .starts_with("Argus couldn't add you as a reviewer"),
            "{add:?}: {}",
            err.message
        );
        assert!(err.message.contains(says), "{add:?}: {}", err.message);
        assert!(!err.message.contains("ask to be added"));
        // Only the add was sent: no change request, no summary.
        let sent: Vec<String> = writes(&log).iter().map(|w| w.path.clone()).collect();
        assert_eq!(sent, [GRAPHQL], "{add:?}");
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_change_request_gitlab_did_not_take_is_an_error() {
    // Saved as a plain note: the command was not recognised.
    let (base, _) = submitting(Submit {
        recognised: false,
        ..SUBMIT
    });
    let err = provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::Unsupported);
    assert!(err.message.contains("delete it there"));

    // Run, but the reviewer state did not move: GitLab's words, no token.
    let (base, log) = submitting(Submit {
        takes: false,
        ..SUBMIT
    });
    let err = provider(&base)
        .submit_review(
            &mr(&base),
            ReviewVerdict::RequestChanges,
            HEAD_SHA,
            Some("Summary"),
        )
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert!(err.message.contains("Changes requested to the current"));
    assert!(!err.message.contains(TOKEN));
    // No summary after a failed verdict.
    assert_eq!(posts(&log).len(), 1);
}

#[test]
#[cfg_attr(windows, ignore)]
fn approves_with_a_summary_note_and_refuses_a_comment_review() {
    let (base, log) = submitting(SUBMIT);
    provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::Approve, HEAD_SHA, Some("LGTM"))
        .unwrap();
    let sent: Vec<(String, Value)> = posts(&log)
        .iter()
        .map(|p| (p.path.clone(), p.json()))
        .collect();
    assert_eq!(
        sent,
        [
            (format!("{MR21}/approve"), json!({ "sha": HEAD_SHA })),
            (format!("{MR21}/notes"), json!({ "body": "LGTM" })),
        ]
    );

    // Approving needs no reviewer status: nobody is added.
    let (base, log) = submitting(Submit {
        reviewer: false,
        ..SUBMIT
    });
    provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::Approve, HEAD_SHA, None)
        .unwrap();
    let sent: Vec<String> = writes(&log).iter().map(|w| w.path.clone()).collect();
    assert_eq!(sent, [format!("{MR21}/approve")]);

    let (base, log) = submitting(SUBMIT);
    let err = provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::Comment, HEAD_SHA, Some("x"))
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::Unsupported);
    assert!(posts(&log).is_empty());
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_failed_summary_says_the_verdict_went_through() {
    let (base, _) = submitting(Submit {
        note_status: "403 Forbidden",
        ..SUBMIT
    });
    let err = provider(&base)
        .submit_review(&mr(&base), ReviewVerdict::Approve, HEAD_SHA, Some("LGTM"))
        .unwrap_err();
    assert!(err
        .message
        .starts_with("Your review was submitted, but its summary was not posted"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn requesting_changes_through_the_core_waits_on_the_author() {
    let (base, log) = submitting(SUBMIT);
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    let before = core.review(id).unwrap();
    assert_eq!(before.status, ReviewStatus::NeedsReview);
    assert_eq!(before.my_review_state, MyReviewState::AwaitingReview);

    let after = core
        .submit_review(id, ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap();
    assert_eq!(after.my_review_state, MyReviewState::RequestedChanges);
    assert_eq!(after.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(after.last_reviewed_sha.as_deref(), Some(HEAD_SHA));
    assert_eq!(posts(&log).len(), 1);
}

#[test]
#[cfg_attr(windows, ignore)]
fn a_non_reviewer_requesting_changes_through_the_core_waits_on_the_author() {
    let (base, log) = submitting(Submit {
        reviewer: false,
        ..SUBMIT
    });
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    assert!(core.refresh().errors.is_empty());
    let id = core.reviews().unwrap()[0].id;
    assert_eq!(
        core.review(id).unwrap().my_review_state,
        MyReviewState::NotReviewer
    );
    assert!(writes(&log).is_empty());

    let after = core
        .submit_review(id, ReviewVerdict::RequestChanges, HEAD_SHA, None)
        .unwrap();
    assert_eq!(after.my_review_state, MyReviewState::RequestedChanges);
    assert_eq!(after.status, ReviewStatus::WaitingOnAuthor);
    let sent: Vec<String> = writes(&log).iter().map(|w| w.path.clone()).collect();
    assert_eq!(sent, [GRAPHQL.to_string(), format!("{MR21}/notes")]);
}
