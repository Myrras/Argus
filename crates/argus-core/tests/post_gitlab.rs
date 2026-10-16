//! GitLab writes against a local HTTP double (a plain `TcpListener`) that
//! records each request's method, path, token header and body, and answers
//! reads from `tests/fixtures/gitlab/`. No network, no real token.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use argus_core::post::{DiffLine, LineKind, PlacedComment, PostErrorKind};
use argus_core::provider::{
    GitlabProvider, ProviderId, ReviewProvider, ReviewRef, Side, StaticToken,
};
use argus_core::store::Store;
use argus_core::{Core, NewComment, PostResult, ReviewStatus};
use serde_json::{json, Value};

const TOKEN: &str = "glpat-fixture-not-a-real-token";
const BASE_SHA: &str = "a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0";
const HEAD_SHA: &str = "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2";
const MR21: &str = "/api/v4/projects/grp%2Fapp/merge_requests/21";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gitlab")
            .join(name),
    )
    .unwrap()
}

/// One request as the double saw it.
#[derive(Debug, Clone)]
struct Request {
    method: String,
    /// Path and query.
    path: String,
    token: bool,
    content_type: Option<String>,
    body: String,
}

impl Request {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

struct Reply {
    status: &'static str,
    headers: String,
    body: String,
}

fn reply(status: &'static str, body: impl Into<String>) -> Reply {
    Reply {
        status,
        headers: String::new(),
        body: body.into(),
    }
}

type Log = Arc<Mutex<Vec<Request>>>;

/// A local server answering every request through `route`; returns the base
/// URL and the log of requests.
fn serve(route: impl Fn(&Request) -> Reply + Send + 'static) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else { continue };
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            let head_end = loop {
                if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break raw.len(),
                    Ok(n) => raw.extend_from_slice(&buf[..n]),
                }
            };
            let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
            let header = |name: &str| {
                head.lines().find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case(name)
                        .then(|| v.trim().to_string())
                })
            };
            let length: usize = header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            while raw.len() < head_end + length {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => raw.extend_from_slice(&buf[..n]),
                }
            }
            let mut words = head.split_whitespace();
            let req = Request {
                method: words.next().unwrap_or("").to_string(),
                path: words.next().unwrap_or("").to_string(),
                token: header("private-token").as_deref() == Some(TOKEN),
                content_type: header("content-type"),
                body: String::from_utf8_lossy(&raw[head_end..]).into_owned(),
            };
            seen.lock().unwrap().push(req.clone());
            let r = route(&req);
            let out = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                r.status,
                r.headers,
                r.body.len(),
                r.body
            );
            let _ = sock.write_all(out.as_bytes());
        }
    });
    (base, log)
}

/// Reads from the fixtures; writes answered as GitLab does on success.
fn instance(req: &Request) -> Reply {
    let (route, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
    let ok = |name: &str| reply("200 OK", fixture(name));
    match (req.method.as_str(), route) {
        ("GET", "/api/v4/user") => ok("user.json"),
        ("GET", "/api/v4/merge_requests") => {
            reply("200 OK", format!("[{}]", fixture("mr_21.json")))
        }
        ("GET", "/api/v4/events") => reply("200 OK", "[]"),
        ("GET", r) if r == MR21 => ok("mr_21.json"),
        ("GET", r) if r == format!("{MR21}/commits") => ok("mr_21_commits.json"),
        ("GET", r) if r == format!("{MR21}/versions") => ok("mr_21_versions.json"),
        ("GET", r) if r == format!("{MR21}/reviewers") => ok("mr_21_reviewers.json"),
        ("GET", r) if r == format!("{MR21}/approvals") => ok("mr_21_approvals.json"),
        ("GET", r) if r == format!("{MR21}/diffs") => {
            let page2 = query.contains("page=2");
            Reply {
                headers: if page2 {
                    "X-Next-Page: \r\n"
                } else {
                    "X-Next-Page: 2\r\n"
                }
                .into(),
                ..ok(if page2 {
                    "mr_21_diffs_page2.json"
                } else {
                    "mr_21_diffs_page1.json"
                })
            }
        }
        ("GET", r) if r == format!("{MR21}/discussions") => {
            let page2 = query.contains("page=2");
            Reply {
                headers: if page2 {
                    "X-Next-Page: \r\n"
                } else {
                    "X-Next-Page: 2\r\n"
                }
                .into(),
                ..ok(if page2 {
                    "mr_21_discussions_page2.json"
                } else {
                    "mr_21_discussions_page1.json"
                })
            }
        }
        ("POST", r) if r == format!("{MR21}/discussions") => reply(
            "201 Created",
            r#"{"id":"d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9d9","individual_note":false,"notes":[{"id":9001,"body":"x","system":false}]}"#,
        ),
        ("POST", r) if r.starts_with(&format!("{MR21}/discussions/")) && r.ends_with("/notes") => {
            reply("201 Created", r#"{"id":9002,"body":"x"}"#)
        }
        ("POST", r) if r == format!("{MR21}/approve") => {
            reply("201 Created", r#"{"id":5,"iid":21,"approved_by":[]}"#)
        }
        _ => reply("404 Not Found", r#"{"message":"404 Not found"}"#),
    }
}

fn provider(base: &str) -> GitlabProvider {
    GitlabProvider::new(base, StaticToken(TOKEN.into())).unwrap()
}

fn mr(base: &str) -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Gitlab,
        host: base.trim_start_matches("http://").into(),
        repo: "grp/app".into(),
        number: 21,
        url: "https://gitlab.example.test/grp/app/-/merge_requests/21".into(),
        title: String::new(),
        author: String::new(),
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

fn placed(path: &str, side: Side, line: DiffLine, start: Option<DiffLine>) -> PlacedComment {
    PlacedComment {
        path: path.into(),
        old_path: path.into(),
        side,
        line,
        start,
        head_sha: HEAD_SHA.into(),
        base_sha: BASE_SHA.into(),
        body: "Why three?".into(),
    }
}

/// The POSTs the double saw.
fn posts(log: &Log) -> Vec<Request> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
        .cloned()
        .collect()
}

/// sha1("src/importer.rs") (from `shasum -a 1`), which GitLab's line codes
/// for that file start with.
const IMPORTER_SHA1: &str = "f98efccf8a5b8ca0e6bd59bffd9fd43cbc9b45b6";

#[test]
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
    assert_eq!(post.content_type.as_deref(), Some("application/json"));
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

#[test]
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
fn approves_and_explains_a_refusal() {
    let (base, log) = serve(instance);
    provider(&base).approve(&mr(&base)).unwrap();
    let p = &posts(&log)[0];
    assert_eq!(p.path, format!("{MR21}/approve"));
    assert!(p.body.is_empty() && p.token);

    let (base, _) = failing("401 Unauthorized", r#"{"message":"401 Unauthorized"}"#);
    let err = provider(&base).approve(&mr(&base)).unwrap_err();
    assert_eq!(err.kind, PostErrorKind::PostRejected);
    assert!(err.message.contains("already approved"));
}

#[test]
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
    let results = core
        .post_comments(
            id,
            vec![
                comment(2, Side::New),
                comment(9, Side::New),
                comment(2, Side::Old),
            ],
        )
        .unwrap();
    assert!(results[0].is_posted() && results[2].is_posted());
    let PostResult::Failed(e) = &results[1] else {
        panic!("{results:?}")
    };
    assert_eq!(e.kind, PostErrorKind::LineNotInDiff);

    let bodies: Vec<Value> = posts(&log).iter().map(|p| p.json()).collect();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["position"]["new_line"], 2);
    assert!(bodies[0]["position"].get("old_line").is_none());
    assert_eq!(bodies[1]["position"]["old_line"], 2);
    assert_eq!(bodies[0]["position"]["head_sha"], HEAD_SHA);

    let after = core.review(id).unwrap();
    assert_eq!(after.last_reviewed_sha.as_deref(), Some(HEAD_SHA));
    assert_eq!(after.status, ReviewStatus::WaitingOnAuthor);

    let approved = core.approve_review(id).unwrap();
    assert!(approved.approved_at.is_some());
    assert!(!approved.is_active());
    assert_eq!(posts(&log).last().unwrap().path, format!("{MR21}/approve"));
    // Every request carried the token, and no body did.
    let seen = log.lock().unwrap();
    assert!(seen.iter().all(|r| r.token && !r.body.contains(TOKEN)));
}
