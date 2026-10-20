//! GitHub writes against a fake `gh` that logs its arguments and the JSON
//! it reads on stdin. No network, no real token.
//!
//! The fake is a POSIX shell script, so these run on Unix only; they still
//! compile on Windows.

#![cfg_attr(windows, allow(dead_code))]

use std::path::PathBuf;

use argus_core::post::{DiffLine, LineKind, PlacedComment, PostErrorKind};
use argus_core::provider::{
    GithubProvider, ProviderId, ReviewProvider, ReviewRef, ReviewVerdict, Side,
};
use serde_json::{json, Value};

const HEAD: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";

/// Routes `gh` calls; every call's arguments and token go to `$LOG`, every
/// stdin body to `$BODIES` (one JSON document per line). The reads a reply
/// to a conversation thread makes first (`api -i`) answer from `$F`.
const FAKE: &str = r#"
echo "$* token=${GH_TOKEN:-none}" >> "$LOG"
case "$*" in
  "auth token --hostname github.com --user work") echo fake-gh-token; exit 0 ;;
  "api -i -H If-None-Match: \"fx-8002\" repos/acme-fixture/widgets/issues/comments/8002")
    printf 'HTTP/2.0 304 Not Modified\r\nEtag: "fx-8002"\r\n\r\n'
    echo "gh: HTTP 304" >&2; exit 1 ;;
  "api -i repos/acme-fixture/widgets/issues/comments/8002")
    printf 'HTTP/2.0 200 OK\r\nEtag: "fx-8002"\r\n\r\n'; cat "$F/issue_comment_8002.json"; exit 0 ;;
  "api -i repos/acme-fixture/widgets/pulls/12/reviews/7004")
    printf 'HTTP/2.0 200 OK\r\nEtag: "fx-7004"\r\n\r\n'; cat "$F/pull_12_review_7004.json"; exit 0 ;;
  "api -i "*)
    printf 'HTTP/2.0 404 Not Found\r\n\r\n{"message":"Not Found"}'
    echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
esac
body=$(cat)
printf '%s\n' "$body" >> "$BODIES"
case "$*" in
  "api --method POST repos/acme-fixture/widgets/issues/12/comments --input -")
    case "$body" in
      *reject-me*)
        echo '{"message":"Validation Failed","errors":[{"resource":"IssueComment","code":"custom","message":"body is locked (fake-gh-token)"}],"status":"422"}'
        echo "gh: Validation Failed (HTTP 422)" >&2; exit 1 ;;
      *)
        echo '{"id":8010,"html_url":"https://github.com/acme-fixture/widgets/pull/12#issuecomment-8010","body":"x"}' ;;
    esac ;;
  "api --method POST repos/acme-fixture/widgets/pulls/12/comments --input -")
    case "$body" in
      *reject-me*)
        echo '{"message":"Validation Failed","errors":[{"resource":"PullRequestReviewComment","code":"custom","field":"pull_request_review_thread.line","message":"pull_request_review_thread.line must be part of the diff (fake-gh-token)"}],"documentation_url":"https://docs.github.com/rest","status":"422"}'
        echo "gh: Validation Failed (HTTP 422)" >&2; exit 1 ;;
      *not-json*)
        echo "<html>proxy error fake-gh-token</html>"
        echo "gh: HTTP 502" >&2; exit 1 ;;
      *)
        echo '{"id":5001,"html_url":"https://github.com/acme-fixture/widgets/pull/12#discussion_r5001","body":"x"}' ;;
    esac ;;
  "api --method POST repos/acme-fixture/widgets/pulls/12/comments/4001/replies --input -")
    echo '{"id":5002,"html_url":"https://github.com/acme-fixture/widgets/pull/12#discussion_r5002","in_reply_to_id":4001}' ;;
  "api --method POST repos/acme-fixture/widgets/pulls/12/reviews --input -")
    echo '{"id":6001,"state":"APPROVED"}' ;;
  "api --method POST repos/acme-fixture/widgets/pulls/13/reviews --input -")
    echo '{"message":"Unprocessable Entity","errors":["Can not approve your own pull request"],"status":"422"}'
    echo "gh: Unprocessable Entity (HTTP 422)" >&2; exit 1 ;;
  *) echo '{"message":"Not Found","status":"404"}'; echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
esac
"#;

struct Fake {
    _dir: tempfile::TempDir,
    gh: String,
    log: PathBuf,
    bodies: PathBuf,
}

impl Fake {
    fn calls(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Every JSON body `gh` read, in order.
    fn bodies(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.bodies)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn provider(&self) -> GithubProvider {
        GithubProvider::with_binary(&self.gh, "github.com", Some("work".into()))
    }
}

#[cfg(unix)]
fn fake() -> Fake {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.log");
    let bodies = dir.path().join("bodies.log");
    let gh = dir.path().join("gh");
    let script = format!(
        "#!/bin/sh\nF='{}'\nLOG='{}'\nBODIES='{}'\n{FAKE}\n",
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/github")
            .display(),
        log.display(),
        bodies.display()
    );
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fake {
        gh: gh.to_string_lossy().into_owned(),
        log,
        bodies,
        _dir: dir,
    }
}

#[cfg(not(unix))]
fn fake() -> Fake {
    unreachable!("fake gh scripts are Unix-only; these tests are ignored on Windows")
}

fn pr(number: u64) -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: "acme-fixture/widgets".into(),
        number,
        url: format!("https://github.com/acme-fixture/widgets/pull/{number}"),
        title: "Add widget caching".into(),
        author: "alice-fixture".into(),
        updated_at: None,
    }
}

fn row(kind: LineKind, old_pos: u32, new_pos: u32) -> DiffLine {
    DiffLine {
        kind,
        old_pos,
        new_pos,
    }
}

fn placed(side: Side, line: DiffLine, start: Option<DiffLine>, body: &str) -> PlacedComment {
    PlacedComment {
        path: "src/cache.rs".into(),
        old_path: "src/cache.rs".into(),
        side,
        line,
        start,
        head_sha: HEAD.into(),
        base_sha: "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0".into(),
        body: body.into(),
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn posts_a_line_comment_on_the_head() {
    let fake = fake();
    let gh = fake.provider();
    let c = placed(Side::New, row(LineKind::Added, 3, 2), None, "Why 60?");
    let posted = gh.post_comment(&pr(12), &c).unwrap();
    assert_eq!(posted.provider_thread_id, "5001");
    assert_eq!(posted.provider_note_id, "5001");
    assert_eq!(
        posted.url,
        "https://github.com/acme-fixture/widgets/pull/12#discussion_r5001"
    );
    assert_eq!((posted.line, posted.side), (2, Side::New));

    assert_eq!(
        fake.bodies(),
        [json!({
            "body": "Why 60?",
            "commit_id": HEAD,
            "path": "src/cache.rs",
            "line": 2,
            "side": "RIGHT",
        })]
    );
    let calls = fake.calls();
    assert!(calls.contains(
        "api --method POST repos/acme-fixture/widgets/pulls/12/comments --input - token=fake-gh-token"
    ));
    // The token goes in the environment only.
    assert!(!std::fs::read_to_string(&fake.bodies)
        .unwrap()
        .contains("fake-gh-token"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn posts_ranges_and_old_side_lines() {
    let fake = fake();
    let gh = fake.provider();
    let range = placed(
        Side::New,
        row(LineKind::Context, 3, 4),
        Some(row(LineKind::Added, 3, 2)),
        "This block",
    );
    gh.post_comment(&pr(12), &range).unwrap();
    let removed = placed(Side::Old, row(LineKind::Removed, 2, 2), None, "Gone?");
    let posted = gh.post_comment(&pr(12), &removed).unwrap();
    assert_eq!((posted.line, posted.side), (2, Side::Old));
    let bodies = fake.bodies();
    assert_eq!(
        bodies[0],
        json!({
            "body": "This block",
            "commit_id": HEAD,
            "path": "src/cache.rs",
            "line": 4,
            "side": "RIGHT",
            "start_line": 2,
            "start_side": "RIGHT",
        })
    );
    assert_eq!(bodies[1]["side"], "LEFT");
    assert_eq!(bodies[1]["line"], 2);
    assert!(bodies[1].get("start_line").is_none());
}

#[test]
#[cfg_attr(windows, ignore)]
fn surfaces_githubs_reason_without_the_token() {
    let fake = fake();
    let gh = fake.provider();
    let c = placed(Side::New, row(LineKind::Context, 9, 9), None, "reject-me");
    let err = gh.post_comment(&pr(12), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(
        err.message,
        "GitHub: Validation Failed: pull_request_review_thread.line must be part of the diff ([redacted])"
    );

    // A body that is not GitHub's JSON is never forwarded.
    let c = placed(Side::New, row(LineKind::Context, 9, 9), None, "not-json");
    let err = gh.post_comment(&pr(12), &c).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::Provider);
    assert!(!err.message.contains("proxy"));
    assert!(!err.message.contains("fake-gh-token"));
}

#[test]
#[cfg_attr(windows, ignore)]
fn replies_to_the_root_comment() {
    let fake = fake();
    let gh = fake.provider();
    let note = gh.reply(&pr(12), "4001", "Done, thanks").unwrap();
    assert_eq!(
        (
            note.provider_thread_id.as_str(),
            note.provider_note_id.as_str()
        ),
        ("4001", "5002")
    );
    assert_eq!(fake.bodies(), [json!({ "body": "Done, thanks" })]);
    assert!(fake.calls().contains(
        "api --method POST repos/acme-fixture/widgets/pulls/12/comments/4001/replies --input -"
    ));
    // Not a GitHub comment id: refused before running gh.
    let err = gh.reply(&pr(12), "abc/../x", "x").unwrap_err();
    assert_eq!(err.kind, PostErrorKind::InvalidInput);
    assert_eq!(fake.bodies().len(), 1);
}

#[test]
#[cfg_attr(windows, ignore)]
fn replies_in_the_conversation_with_a_quoting_pr_comment() {
    let fake = fake();
    let gh = fake.provider();
    let note = gh
        .reply(&pr(12), "issuecomment-8002", "Settings it is.")
        .unwrap();
    assert_eq!(
        (
            note.provider_thread_id.as_str(),
            note.provider_note_id.as_str(),
            note.url.as_str()
        ),
        (
            "issuecomment-8002",
            "issuecomment-8010",
            "https://github.com/acme-fixture/widgets/pull/12#issuecomment-8010"
        )
    );
    // Exactly one write: a PR comment quoting the thread's first comment,
    // read just before, and ending with the marker that files it under the
    // thread (invisible on GitHub).
    assert_eq!(
        fake.bodies(),
        [json!({
            "body": "> Should the TTL come from settings instead of a constant?\n\nSettings it is.\n\n<!-- argus:reply-to issuecomment-8002 -->"
        })]
    );
    let calls = fake.calls();
    let lines: Vec<&str> = calls.lines().filter(|l| !l.starts_with("auth ")).collect();
    assert_eq!(
        lines,
        [
            "api -i repos/acme-fixture/widgets/issues/comments/8002 token=fake-gh-token",
            "api --method POST repos/acme-fixture/widgets/issues/12/comments --input - token=fake-gh-token",
        ]
    );

    // A review summary's thread: the review's own body is quoted, not its
    // verdict heading.
    let note = gh
        .reply(&pr(12), "pullrequestreview-7004", "Fixed.")
        .unwrap();
    assert_eq!(note.provider_thread_id, "pullrequestreview-7004");
    assert_eq!(
        fake.bodies()[1],
        json!({ "body": "> One more nit.\n\nFixed.\n\n<!-- argus:reply-to pullrequestreview-7004 -->" })
    );
    assert!(fake
        .calls()
        .contains("api -i repos/acme-fixture/widgets/pulls/12/reviews/7004 token="));

    // GitHub's refusal comes through redacted; a thread that is gone is
    // not found, and nothing is posted for it.
    let err = gh
        .reply(&pr(12), "issuecomment-8002", "reject-me")
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(
        err.message,
        "GitHub: Validation Failed: body is locked ([redacted])"
    );
    let posts = fake.bodies().len();
    let err = gh.reply(&pr(12), "issuecomment-9999", "x").unwrap_err();
    assert_eq!(err.kind, PostErrorKind::NotFound);
    assert_eq!(fake.bodies().len(), posts);
}

#[test]
#[cfg_attr(windows, ignore)]
fn approves_with_an_approve_review() {
    let fake = fake();
    let gh = fake.provider();
    gh.approve(&pr(12), HEAD).unwrap();
    assert_eq!(
        fake.bodies(),
        [json!({ "event": "APPROVE", "commit_id": HEAD })]
    );
    assert!(fake
        .calls()
        .contains("api --method POST repos/acme-fixture/widgets/pulls/12/reviews --input -"));

    let err = gh.approve(&pr(13), HEAD).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert_eq!(
        err.message,
        "GitHub: Unprocessable Entity: Can not approve your own pull request"
    );
    // Missing PR: not found.
    assert_eq!(
        gh.reply(&pr(99), "1", "x").unwrap_err().kind,
        PostErrorKind::NotFound
    );
}

// ---------------------------------------------------------------------------
// Submit review: one GitHub review per submit.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(windows, ignore)]
fn submits_each_verdict_as_one_review_pinned_to_the_head() {
    let f = fake();
    let gh = f.provider();
    gh.submit_review(&pr(12), ReviewVerdict::Approve, HEAD, None)
        .unwrap();
    gh.submit_review(
        &pr(12),
        ReviewVerdict::RequestChanges,
        HEAD,
        Some("Please add a test for the TTL."),
    )
    .unwrap();
    gh.submit_review(&pr(12), ReviewVerdict::Comment, HEAD, Some("Looks close."))
        .unwrap();
    gh.submit_review(&pr(12), ReviewVerdict::Approve, HEAD, Some("LGTM"))
        .unwrap();
    assert_eq!(
        f.bodies(),
        [
            json!({ "event": "APPROVE", "commit_id": HEAD }),
            json!({
                "event": "REQUEST_CHANGES",
                "commit_id": HEAD,
                "body": "Please add a test for the TTL."
            }),
            json!({ "event": "COMMENT", "commit_id": HEAD, "body": "Looks close." }),
            json!({ "event": "APPROVE", "commit_id": HEAD, "body": "LGTM" }),
        ]
    );
    let calls = f.calls();
    assert_eq!(
        calls
            .lines()
            .filter(|l| l.starts_with(
                "api --method POST repos/acme-fixture/widgets/pulls/12/reviews --input -"
            ))
            .count(),
        4
    );
    // GitHub's refusal comes back with its reason.
    let err = gh
        .submit_review(&pr(13), ReviewVerdict::RequestChanges, HEAD, Some("x"))
        .unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert!(!err.message.contains("fake-gh-token"));
}
