//! A local GitLab double (a plain `TcpListener`) that records each
//! request's method, path, token header and body, and answers reads from
//! `tests/fixtures/gitlab/`; plus the Submit review routes on top. The
//! provider reaches it through the fake `glab` ([`super::glab`]), which adds
//! glab's own login. Shared by `post_gitlab.rs` and `guarantees.rs`. No
//! network, no real token.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use argus_core::provider::{GitlabProvider, ProviderId, ReviewRef};
use serde_json::{json, Value};

pub use super::glab::{FakeGlab, TOKEN};
pub const BASE_SHA: &str = "a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0";
pub const HEAD_SHA: &str = "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2";
pub const MR21: &str = "/api/v4/projects/grp%2Fapp/merge_requests/21";

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gitlab")
            .join(name),
    )
    .unwrap()
}

/// One request as the double saw it.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Path and query.
    pub path: String,
    /// glab's token came as `PRIVATE-TOKEN` (REST and GraphQL alike).
    pub token: bool,
    pub content_type: Option<String>,
    pub body: String,
}

impl Request {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

pub struct Reply {
    pub status: &'static str,
    pub headers: String,
    pub body: String,
}

pub fn reply(status: &'static str, body: impl Into<String>) -> Reply {
    Reply {
        status,
        headers: String::new(),
        body: body.into(),
    }
}

pub type Log = Arc<Mutex<Vec<Request>>>;

/// A local server answering every request through `route`; returns the base
/// URL and the log of requests.
pub fn serve(route: impl Fn(&Request) -> Reply + Send + 'static) -> (String, Log) {
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
pub fn instance(req: &Request) -> Reply {
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
        ("PUT", r) if r.starts_with(&format!("{MR21}/discussions/")) => reply(
            "200 OK",
            r#"{"id":"d1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","individual_note":false,"notes":[]}"#,
        ),
        ("POST", r) if r == format!("{MR21}/approve") => {
            reply("201 Created", r#"{"id":5,"iid":21,"approved_by":[]}"#)
        }
        _ => reply("404 Not Found", r#"{"message":"404 Not found"}"#),
    }
}

/// A provider for the double at `base`, through the shared fake glab.
pub fn provider(base: &str) -> GitlabProvider {
    FakeGlab::shared().provider(base)
}

pub fn mr(base: &str) -> ReviewRef {
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

/// The POSTs the double saw.
pub fn posts(log: &Log) -> Vec<Request> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
        .cloned()
        .collect()
}

/// Every request but a GET: the writes.
pub fn writes(log: &Log) -> Vec<Request> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|r| r.method != "GET")
        .cloned()
        .collect()
}

/// How the double answers the reviewer add (GraphQL
/// `mergeRequestSetReviewers`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Add {
    /// Appends the user to the reviewers.
    Appends,
    /// Refused as GitLab does without the permission (a top-level error).
    Refused,
    /// The mutation's own `errors`.
    Fails,
    /// Free tier: one reviewer per MR, so the other one stays and the user
    /// is not added.
    OneReviewer,
    /// Answers as done but nothing changed.
    Ignored,
}

/// How the double answers a submit: the instance's version, whether the
/// user is a reviewer, the reviewer add, and how `/request_changes` goes.
#[derive(Clone, Copy)]
pub struct Submit {
    pub version: &'static str,
    pub reviewer: bool,
    pub add: Add,
    /// `/reviewers` reads after the add that do not list the user yet.
    pub lag: u32,
    /// GitLab runs the quick action (202) rather than saving it as a note.
    pub recognised: bool,
    /// The quick action sets the user's reviewer state.
    pub takes: bool,
    /// What a plain note (the summary) gets.
    pub note_status: &'static str,
}

pub const SUBMIT: Submit = Submit {
    version: "17.5.1-ee",
    reviewer: true,
    add: Add::Appends,
    lag: 0,
    recognised: true,
    takes: true,
    note_status: "201 Created",
};

/// The GraphQL endpoint.
pub const GRAPHQL: &str = "/api/graphql";

/// The double's state over one submit.
#[derive(Default)]
struct SubmitState {
    changed: bool,
    added: bool,
    reads_since_add: u32,
}

/// The fixture instance with the submit routes on top.
pub fn submitting(how: Submit) -> (String, Log) {
    let state = Arc::new(Mutex::new(SubmitState::default()));
    serve(move |req: &Request| {
        let route = req
            .path
            .split_once('?')
            .map_or(req.path.as_str(), |(r, _)| r);
        let mut st = state.lock().unwrap();
        let me_state = if st.changed {
            "requested_changes"
        } else {
            "unreviewed"
        };
        match (req.method.as_str(), route) {
            ("GET", "/api/v4/version") => reply(
                "200 OK",
                format!(r#"{{"version":"{}","revision":"fixture"}}"#, how.version),
            ),
            ("GET", r) if r == format!("{MR21}/reviewers") => {
                let mut list = vec![json!({
                    "user": { "id": 45, "username": "dana-fixture" }, "state": "approved"
                })];
                let listed = how.reviewer || {
                    if st.added {
                        st.reads_since_add += 1;
                    }
                    st.added && st.reads_since_add > how.lag
                };
                if listed {
                    list.push(json!({
                        "user": { "id": 42, "username": "rev-fixture" }, "state": me_state
                    }));
                }
                reply("200 OK", Value::Array(list).to_string())
            }
            ("POST", GRAPHQL) => {
                let nodes = |me: bool| {
                    let mut n = vec![json!({ "username": "dana-fixture" })];
                    if me {
                        n.push(json!({ "username": "rev-fixture" }));
                    }
                    n
                };
                let payload = |errors: Vec<&str>, multiple: bool, me: bool| {
                    json!({ "data": { "mergeRequestSetReviewers": {
                        "errors": errors,
                        "mergeRequest": {
                            "allowsMultipleReviewers": multiple,
                            "reviewers": { "nodes": nodes(me) },
                        },
                    } } })
                    .to_string()
                };
                match how.add {
                    Add::Appends => {
                        st.added = true;
                        reply("200 OK", payload(vec![], true, true))
                    }
                    Add::Refused => reply(
                        "200 OK",
                        json!({
                            "data": { "mergeRequestSetReviewers": null },
                            "errors": [{
                                "message": "The resource that you are attempting to access does not exist or you don't have permission to perform this action",
                                "locations": [{ "line": 1, "column": 80 }],
                                "path": ["mergeRequestSetReviewers"],
                            }],
                        })
                        .to_string(),
                    ),
                    Add::Fails => reply(
                        "200 OK",
                        payload(vec!["Reviewers not able to be set: rev-fixture"], true, false),
                    ),
                    Add::OneReviewer => reply("200 OK", payload(vec![], false, false)),
                    Add::Ignored => reply("200 OK", payload(vec![], true, false)),
                }
            }
            ("POST", r) if r == format!("{MR21}/notes") => {
                if req.json()["body"] == "/request_changes" {
                    if !how.recognised {
                        return reply("201 Created", r#"{"id":9010,"body":"/request_changes"}"#);
                    }
                    if how.takes {
                        st.changed = true;
                    }
                    return reply(
                        "202 Accepted",
                        r#"{"commands_changes":{},"summary":["Changes requested to the current merge request."]}"#,
                    );
                }
                if how.note_status.starts_with('2') {
                    reply(how.note_status, r#"{"id":9011,"body":"x"}"#)
                } else {
                    reply(how.note_status, r#"{"message":"403 Forbidden"}"#)
                }
            }
            _ => {
                drop(st);
                instance(req)
            }
        }
    })
}

/// Each request as `METHOD path` without the query: the sequence a test
/// asserts on.
pub fn sequence(log: &Log) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .map(|r| {
            let path = r.path.split_once('?').map_or(r.path.as_str(), |(p, _)| p);
            format!("{} {path}", r.method)
        })
        .collect()
}
