//! The GitLab provider against a local HTTP double (a plain `TcpListener`,
//! as in Podium) replaying `tests/fixtures/gitlab/`. No network, no real
//! token.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use argus_core::provider::ReviewProvider as _;
use argus_core::store::Store;
use argus_core::{Core, ReviewStatus};

use argus_core::provider::{
    DiffRange, FileStatus, GitlabProvider, ProviderId, ReviewRef, ReviewState, ReviewerState, Side,
    StaticToken, TokenSource,
};
use argus_core::CoreError;

const TOKEN: &str = "glpat-fixture-not-a-real-token";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gitlab")
            .join(name),
    )
    .unwrap()
}

struct Reply {
    status: &'static str,
    headers: String,
    body: String,
}

fn ok(name: &str) -> Reply {
    Reply {
        status: "200 OK",
        headers: String::new(),
        body: fixture(name),
    }
}

fn status(status: &'static str) -> Reply {
    Reply {
        status,
        headers: String::new(),
        body: "secret-ish body".into(),
    }
}

/// The fixture instance: routes on the request path (query included).
fn fixture_routes(path: &str) -> Reply {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let mr21 = "/api/v4/projects/grp%2Fapp/merge_requests/21";
    let mr31 = "/api/v4/projects/grp%2Fapp/merge_requests/31";
    match route {
        "/api/v4/user" => ok("user.json"),
        "/api/v4/merge_requests" if query.contains("reviewer_id=42") => ok("mrs_reviewer.json"),
        "/api/v4/events" if query.contains("action=commented") => ok("events.json"),
        "/api/v4/projects/200/merge_requests/5" => ok("mr_200_5.json"),
        "/api/v4/projects/300/merge_requests/8" => ok("mr_300_8.json"),
        r if r == mr21 => ok("mr_21.json"),
        r if r == format!("{mr21}/commits") => ok("mr_21_commits.json"),
        r if r == format!("{mr21}/versions") => ok("mr_21_versions.json"),
        r if r == format!("{mr21}/reviewers") => ok("mr_21_reviewers.json"),
        r if r == format!("{mr21}/approvals") => ok("mr_21_approvals.json"),
        r if r == format!("{mr21}/diffs") => {
            if query.contains("page=2") {
                Reply {
                    headers: "X-Next-Page: \r\n".into(),
                    ..ok("mr_21_diffs_page2.json")
                }
            } else {
                Reply {
                    headers: "X-Next-Page: 2\r\n".into(),
                    ..ok("mr_21_diffs_page1.json")
                }
            }
        }
        "/api/v4/projects/grp%2Fapp/repository/commits/a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2/diff"
            if query.contains("page=1") =>
        {
            ok("commit_a2_diff.json")
        }
        "/api/v4/projects/grp%2Fapp/repository/commits/a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2" => {
            ok("commit_a2.json")
        }
        "/api/v4/projects/grp%2Fapp/repository/compare"
            if query
                == "from=a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1\
                    &to=a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2&straight=true" =>
        {
            ok("compare_a1_a2.json")
        }
        // An instance before 15.7: no `/diffs`, only `/changes`.
        r if r == format!("{mr31}/diffs") => status("404 Not Found"),
        r if r == format!("{mr31}/changes") => ok("mr_31_changes.json"),
        r if r == format!("{mr21}/discussions") => {
            if query.contains("page=2") {
                Reply {
                    headers: "X-Next-Page: \r\nX-Page: 2\r\n".into(),
                    ..ok("mr_21_discussions_page2.json")
                }
            } else {
                Reply {
                    headers: "X-Next-Page: 2\r\nX-Page: 1\r\n".into(),
                    ..ok("mr_21_discussions_page1.json")
                }
            }
        }
        "/api/v4/projects/grp%2Fsub%2Flib/merge_requests/5" => ok("mr_200_5.json"),
        r if r.starts_with("/api/v4/projects/grp%2Fsub%2Flib/merge_requests/5/") => Reply {
            body: "[]".into(),
            ..ok("user.json")
        },
        r if r == mr31 => ok("mr_31.json"),
        r if r == format!("{mr31}/reviewers") => ok("mr_31_reviewers.json"),
        r if r == format!("{mr31}/approvals") => ok("mr_31_approvals.json"),
        r if r.starts_with(mr31) => Reply {
            body: "[]".into(),
            ..ok("user.json")
        },
        "/api/v4/projects/grp%2Fapp/repository/files/src%2Fimporter.rs/raw"
            if query == "ref=a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2" =>
        {
            ok("raw_importer.rs")
        }
        _ => status("404 Not Found"),
    }
}

/// A local server answering every request through `route`; returns the base
/// URL and the log of `"<path> token=<yes|no>"` lines it saw.
fn serve(route: fn(&str) -> Reply) -> (String, Arc<Mutex<Vec<String>>>) {
    serve_raw(move |path, _| route(path))
}

/// Like [`serve`], with the whole request head (lowercased) for the route.
fn serve_raw(
    route: impl Fn(&str, &str) -> Reply + Send + 'static,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else { continue };
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let req = String::from_utf8_lossy(&req).into_owned();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
            let token = req
                .to_lowercase()
                .contains(&format!("private-token: {}", TOKEN.to_lowercase()));
            log.lock()
                .unwrap()
                .push(format!("{path} token={}", if token { "yes" } else { "no" }));
            let reply = route(&path, &req.to_lowercase());
            let out = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status,
                reply.headers,
                reply.body.len(),
                reply.body
            );
            let _ = sock.write_all(out.as_bytes());
        }
    });
    (base, seen)
}

fn provider(base: &str) -> GitlabProvider {
    GitlabProvider::new(base, StaticToken(TOKEN.into())).unwrap()
}

fn mr_ref(base: &str, number: u64) -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Gitlab,
        host: base.trim_start_matches("http://").into(),
        repo: "grp/app".into(),
        number,
        url: String::new(),
        title: String::new(),
        author: String::new(),
        updated_at: None,
    }
}

#[test]
fn lists_reviewer_mrs_plus_commented_ones() {
    let (base, seen) = serve(fixture_routes);
    let gl = provider(&base);
    assert_eq!(gl.current_user().unwrap(), "rev-fixture");
    let list = gl.list_reviews().unwrap();
    let keys: Vec<(&str, u64)> = list.iter().map(|r| (r.repo.as_str(), r.number)).collect();
    // !22 is the user's own MR, !8 is merged, the issue note is skipped, and
    // !21 / !5 appear once each although several events name them.
    assert_eq!(keys, [("grp/sub/lib", 5), ("grp/app", 21)]);
    assert_eq!(list[1].author, "alice-fixture");
    assert_eq!(list[1].host, base.trim_start_matches("http://"));
    assert!(list.iter().all(|r| r.provider == ProviderId::Gitlab));

    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|l| l.ends_with("token=yes")));
    assert_eq!(
        seen.iter()
            .filter(|l| l.starts_with("/api/v4/projects/200/merge_requests/5 "))
            .count(),
        1
    );
}

#[test]
fn fetches_detail_with_versions_reviewers_and_approvals() {
    let (base, _) = serve(fixture_routes);
    let gl = provider(&base);
    let d = gl.fetch_review(&mr_ref(&base, 21)).unwrap();
    assert_eq!(d.state, ReviewState::Open);
    assert_eq!(d.review.title, "Add retry to the importer");
    assert_eq!(d.head_sha, "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2");
    assert_eq!(
        d.start_sha.as_deref(),
        Some("a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0")
    );
    // Oldest first, for both.
    assert_eq!(d.commits[0].title, "Add retry");
    assert_eq!(
        d.commits[1].authored_at.unwrap().to_rfc3339(),
        "2026-10-10T08:55:00+00:00"
    );
    let heads: Vec<&str> = d.versions.iter().map(|v| &v.head_sha[..2]).collect();
    assert_eq!(heads, ["a1", "a2"]);

    assert!(d.requested_from_me);
    assert!(!d.approved_by_me);
    let state_of = |u: &str| {
        d.reviewers
            .iter()
            .find(|r| r.username == u)
            .map(|r| r.state)
    };
    assert_eq!(state_of("rev-fixture"), Some(ReviewerState::Requested));
    assert_eq!(state_of("dana-fixture"), Some(ReviewerState::Approved));
}

#[test]
fn approval_shows_on_a_merged_mr() {
    let (base, _) = serve(fixture_routes);
    let d = provider(&base).fetch_review(&mr_ref(&base, 31)).unwrap();
    assert_eq!(d.state, ReviewState::Merged);
    assert!(d.approved_by_me);
    assert!(!d.requested_from_me);
}

#[test]
fn discussions_become_threads_across_pages() {
    let (base, seen) = serve(fixture_routes);
    let threads = provider(&base).fetch_threads(&mr_ref(&base, 21)).unwrap();
    let ids: Vec<&str> = threads.iter().map(|t| &t.id[..2]).collect();
    // The system-only discussion is dropped; page 2 was followed.
    assert_eq!(ids, ["d1", "d2", "d4", "d5"]);
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.contains("/discussions?per_page=100&page=2 ")));

    let multi = &threads[0];
    assert_eq!(multi.notes.len(), 3);
    assert_eq!(multi.notes[1].author, "alice-fixture");
    assert!(!multi.outdated && !multi.resolved);
    let a = multi.anchor.as_ref().unwrap();
    assert_eq!(
        (a.side, a.line_new, a.line_old),
        (Side::New, Some(12), None)
    );

    // Position taken against an older head → outdated; an unchanged line
    // carries both line numbers.
    let outdated = &threads[1];
    assert!(outdated.outdated && outdated.resolved);
    let a = outdated.anchor.as_ref().unwrap();
    assert_eq!((a.line_old, a.line_new), (Some(30), Some(31)));
    assert_eq!(a.head_sha, "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1");

    // Removed lines in a renamed file, as a range.
    let renamed = threads[2].anchor.as_ref().unwrap();
    assert_eq!(renamed.side, Side::Old);
    assert_eq!(renamed.path_old.as_deref(), Some("src/old_name.rs"));
    assert_eq!(renamed.path_new.as_deref(), Some("src/new_name.rs"));
    assert_eq!((renamed.start_line, renamed.line_old), (Some(2), Some(4)));

    // An MR-level discussion has no anchor.
    assert!(threads[3].anchor.is_none());
}

#[test]
fn file_at_reads_raw_content() {
    let (base, _) = serve(fixture_routes);
    let gl = provider(&base);
    let r = mr_ref(&base, 21);
    let text = gl
        .file_at(
            &r,
            "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2",
            "src/importer.rs",
        )
        .unwrap();
    assert_eq!(text, "pub fn retries() -> u32 {\n    3\n}\n");
    assert!(matches!(
        gl.file_at(&r, "a2a2", "src/missing.rs"),
        Err(CoreError::NotFound)
    ));
}

const BASE_SHA: &str = "a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0";
const HEAD_SHA: &str = "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2";

fn review_range() -> DiffRange {
    DiffRange::Review {
        base: BASE_SHA.into(),
        head: HEAD_SHA.into(),
    }
}

#[test]
fn lists_mr_diffs_across_pages() {
    let (base, seen) = serve(fixture_routes);
    let diff = provider(&base)
        .fetch_files(&mr_ref(&base, 21), &review_range())
        .unwrap();
    // The left side is the version's base_sha, as GitLab diffs from it.
    assert_eq!(
        (diff.base_sha.as_str(), diff.head_sha.as_str()),
        (BASE_SHA, HEAD_SHA)
    );
    assert!(!diff.truncated);
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.contains("/diffs?per_page=100&page=2 ")));
    let f = |p: &str| diff.files.iter().find(|f| f.path == p).unwrap();
    assert_eq!(diff.files.len(), 6);

    let modified = f("src/importer.rs");
    assert_eq!(modified.status, FileStatus::Modified);
    // Counted from the diff: GitLab sends no numbers.
    assert_eq!((modified.additions, modified.deletions), (1, 1));
    assert!(modified.patch.is_some());
    assert_eq!(f("src/backoff.rs").status, FileStatus::Added);
    assert_eq!(f("src/backoff.rs").additions, 2);
    assert_eq!(f("src/old_import.rs").status, FileStatus::Removed);

    let renamed = f("src/new_name.rs");
    assert_eq!(renamed.status, FileStatus::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("src/old_name.rs"));

    let bin = f("docs/diagram.bin");
    assert!(bin.binary && !bin.too_large && bin.patch.is_none());
    let big = f("data/seed.sql");
    assert!(big.too_large && !big.binary && big.patch.is_none());

    // The content key follows the diff: the same diff gives the same key,
    // and every file has its own.
    let again = provider(&base)
        .fetch_files(&mr_ref(&base, 21), &review_range())
        .unwrap();
    assert_eq!(again.files, diff.files);
    let mut keys: Vec<&str> = diff.files.iter().map(|f| f.content_sha.as_str()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 6);
}

#[test]
fn falls_back_to_changes_on_an_older_instance() {
    let (base, _) = serve(fixture_routes);
    let diff = provider(&base)
        .fetch_files(&mr_ref(&base, 31), &review_range())
        .unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "README.md");
    assert_eq!(diff.files[0].additions, 1);
    // `overflow`: GitLab cut the list.
    assert!(diff.truncated);
}

#[test]
fn compares_two_commits_straight() {
    let (base, _) = serve(fixture_routes);
    let from = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
    let diff = provider(&base)
        .fetch_files(
            &mr_ref(&base, 21),
            &DiffRange::Compare {
                from: from.into(),
                to: HEAD_SHA.into(),
            },
        )
        .unwrap();
    assert_eq!(diff.base_sha, from);
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].patch.as_deref().unwrap().lines().count(), 5);
}

struct NoToken;

impl TokenSource for NoToken {
    fn token(&self) -> Option<String> {
        None
    }
}

#[test]
fn auth_and_setup_errors_are_argus_text() {
    let (base, seen) = serve(|_| status("401 Unauthorized"));
    let err = provider(&base).list_reviews().unwrap_err();
    assert!(matches!(err, CoreError::GitlabAuth));
    assert!(err.is_unavailable());
    assert!(!err.to_string().contains("secret-ish"));
    assert_eq!(seen.lock().unwrap().len(), 1);

    let gl = GitlabProvider::new(&base, NoToken).unwrap();
    assert!(matches!(
        gl.list_reviews(),
        Err(CoreError::GitlabNotConfigured)
    ));

    let (base, _) = serve(|_| status("500 Internal Server Error"));
    let err = provider(&base).current_user().unwrap_err();
    assert!(matches!(err, CoreError::Provider(_)));
    assert!(!err.to_string().contains(&base));
}

#[test]
fn never_follows_a_redirect_with_the_token() {
    let (thief, stolen) = serve(|_| ok("user.json"));
    let (base, _) = {
        // A redirect pointing at the thief; the route fn cannot capture, so
        // the thief's address goes through a static.
        static TARGET: Mutex<String> = Mutex::new(String::new());
        *TARGET.lock().unwrap() = thief.clone();
        serve(|_| Reply {
            status: "302 Found",
            headers: format!("Location: {}/api/v4/user\r\n", TARGET.lock().unwrap()),
            body: String::new(),
        })
    };
    let err = provider(&base).current_user().unwrap_err();
    assert!(err.to_string().contains("redirected"));
    assert!(stolen.lock().unwrap().is_empty());
}

#[test]
fn refresh_end_to_end_through_the_core() {
    let (base, _) = serve(fixture_routes);
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(provider(&base))]);
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.changed.len(), 2);

    let reviews = core.reviews().unwrap();
    let r21 = reviews.iter().find(|r| r.review.number == 21).unwrap();
    assert_eq!(r21.status, ReviewStatus::NeedsReview);
    assert!(r21.requested && r21.is_active());
    // d1: the user answered the author's reply last, so it is untouched
    // again; d2 is resolved.
    assert_eq!(r21.untouched_threads, 1);
    let threads = core.threads(r21.id).unwrap();
    assert_eq!(threads.len(), 4);
    assert!(threads[0].mine && !threads[2].mine);

    let marked = core.mark_reviewed(r21.id).unwrap();
    assert_eq!(marked.status, ReviewStatus::WaitingOnAuthor);
    assert_eq!(
        marked.last_reviewed_sha.as_deref(),
        Some(r21.head_sha.as_str())
    );

    // Nothing moved on the instance: the next refresh changes nothing.
    assert!(core.refresh().changed.is_empty());
    assert_eq!(
        core.file_at(r21.id, &r21.head_sha, "src/importer.rs")
            .unwrap(),
        "pub fn retries() -> u32 {\n    3\n}\n"
    );

    // The code view: the files, a viewed mark that persists in the store.
    let files = core.list_files(r21.id, false).unwrap();
    assert_eq!(files.files.len(), 6);
    assert_eq!(files.base_sha, BASE_SHA);
    assert!(files.files.iter().all(|f| !f.viewed));
    assert_eq!(
        core.set_file_viewed(r21.id, "src/importer.rs", true)
            .unwrap(),
        ["src/importer.rs"]
    );
    let files = core.list_files(r21.id, false).unwrap();
    let viewed: Vec<&str> = files
        .files
        .iter()
        .filter(|f| f.viewed)
        .map(|f| f.file.path.as_str())
        .collect();
    assert_eq!(viewed, ["src/importer.rs"]);
    // Marked reviewed at the head: nothing to diff since.
    assert!(matches!(
        core.list_files(r21.id, true),
        Err(CoreError::InvalidInput(_))
    ));
}

#[test]
fn commit_diff_and_resolve_events() {
    let (base, seen) = serve(fixture_routes);
    let gl = provider(&base);
    let r = mr_ref(&base, 21);
    let diff = gl.commit_diff(&r, HEAD_SHA).unwrap();
    // The diff endpoint names no parent; the commit does.
    assert_eq!(
        (diff.base_sha.as_str(), diff.head_sha.as_str()),
        ("a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1", HEAD_SHA)
    );
    let files: Vec<(&str, FileStatus)> = diff
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status))
        .collect();
    assert_eq!(
        files,
        [
            ("src/importer.rs", FileStatus::Modified),
            ("src/new_name.rs", FileStatus::Renamed),
            ("src/old_import.rs", FileStatus::Removed),
        ]
    );
    assert_eq!(diff.files[1].old_path.as_deref(), Some("src/old_name.rs"));
    assert!(diff.files[1].patch.is_none());
    assert!(seen.lock().unwrap().iter().any(|l| l.contains(
        "/repository/commits/a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2/diff?per_page=100&page=1 "
    )));

    // d2 is the one resolved discussion; only it has an event.
    let events = gl.thread_resolutions(&r).unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert!(e.thread_id.starts_with("d2") && e.resolved);
    assert_eq!(e.by, "alice-fixture");
    assert_eq!(e.at.to_rfc3339(), "2026-10-09T15:00:00+00:00");
}

// --- M6: conditional requests, rate limits, timeouts ---

/// The fixture instance with an ETag per path, answering 304 to a request
/// that names it; `not_modified` logs each 304's path.
fn conditional_routes(not_modified: Arc<Mutex<Vec<String>>>) -> impl Fn(&str, &str) -> Reply {
    move |path, req| {
        let tag = format!("w/\"fx-{}\"", path.len());
        let reply = fixture_routes(path);
        if !reply.status.starts_with("200") {
            return reply;
        }
        if req.contains(&format!("if-none-match: {tag}")) {
            not_modified.lock().unwrap().push(path.to_string());
            return Reply {
                status: "304 Not Modified",
                // GitLab repeats the pagination headers on a 304; Argus
                // must not need them.
                headers: format!("ETag: W/\"fx-{}\"\r\n", path.len()),
                body: String::new(),
            };
        }
        Reply {
            headers: format!("{}ETag: W/\"fx-{}\"\r\n", reply.headers, path.len()),
            ..reply
        }
    }
}

#[test]
fn a_second_poll_sends_if_none_match_and_replays_the_304s() {
    let not_modified = Arc::new(Mutex::new(Vec::new()));
    let (base, _) = serve_raw(conditional_routes(not_modified.clone()));
    let gl = provider(&base);
    let r = mr_ref(&base, 21);
    let poll = || {
        (
            gl.fetch_review(&r).unwrap(),
            gl.fetch_threads(&r).unwrap(),
            gl.fetch_files(&r, &review_range()).unwrap(),
        )
    };
    let first = poll();
    // Within one poll the MR itself is read twice (detail and threads): the
    // second read is already a 304.
    let after_first = not_modified.lock().unwrap().len();
    let second = poll();
    assert_eq!(first, second);
    let seen = not_modified.lock().unwrap();
    let seen = &seen[after_first..];
    // Page 2 was still followed (its number came from the cached page 1).
    assert!(seen
        .iter()
        .any(|p| p.ends_with("/discussions?per_page=100&page=2")));
    assert!(seen
        .iter()
        .any(|p| p.ends_with("/diffs?per_page=100&page=2")));
    assert!(seen
        .iter()
        .any(|p| p == "/api/v4/projects/grp%2Fapp/merge_requests/21"));
}

#[test]
fn a_429_backs_off_until_retry_after() {
    let (base, seen) = serve(|_| Reply {
        status: "429 Too Many Requests",
        headers: "Retry-After: 120\r\nRateLimit-Remaining: 0\r\n".into(),
        body: "Retry later".into(),
    });
    let gl = provider(&base);
    let start = chrono::Utc::now();
    let err = gl.current_user().unwrap_err();
    let end = chrono::Utc::now();
    let until = gl.rate_limited_until().unwrap();
    // 120 s from the reply, which came somewhere in the call.
    assert!((until - start).num_seconds() >= 119 && (until - end).num_seconds() <= 121);
    let hhmm = until
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string();
    assert_eq!(
        err.to_string(),
        format!(
            "Could not fetch reviews: GitLab is rate limited until {hhmm}; Argus tries again then"
        )
    );
    assert!(!err.is_unavailable());
    // Nothing more is sent until then.
    assert!(gl.list_reviews().is_err());
    assert!(gl.fetch_threads(&mr_ref(&base, 21)).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn a_429_without_retry_after_reads_the_reset_epoch() {
    static RESET: Mutex<i64> = Mutex::new(0);
    let reset = chrono::Utc::now().timestamp() + 300;
    *RESET.lock().unwrap() = reset;
    let (base, _) = serve(|_| Reply {
        status: "429 Too Many Requests",
        headers: format!("RateLimit-Reset: {}\r\n", *RESET.lock().unwrap()),
        body: String::new(),
    });
    let gl = provider(&base);
    assert!(gl.current_user().is_err());
    assert_eq!(gl.rate_limited_until().unwrap().timestamp(), reset);
}

#[test]
fn a_hung_instance_times_out_without_retrying() {
    // Accepts, reads, never answers.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(Mutex::new(0));
    let count = accepted.clone();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for sock in listener.incoming().flatten() {
            *count.lock().unwrap() += 1;
            held.push(sock);
        }
    });
    let gl = provider(&base).with_timeout(std::time::Duration::from_millis(300));
    let start = std::time::Instant::now();
    let err = gl.current_user().unwrap_err();
    assert!(err
        .to_string()
        .contains("could not reach GitLab (timed out)"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(*accepted.lock().unwrap(), 1);
}

/// `n` discussions with one note each, ids `d<from>`…, as JSON.
fn discussions(from: usize, n: usize) -> String {
    let items: Vec<String> = (from..from + n)
        .map(|i| {
            format!(
                r#"{{"id":"d{i}","notes":[{{"id":{i},"body":"note {i}","author":{{"username":"alice-fixture"}},"created_at":"2026-10-10T08:00:00Z"}}]}}"#
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

#[test]
fn a_full_page_replayed_from_the_cache_still_reads_the_next_one() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // Exactly 100 discussions, then a 101st. Page 1 does not change, so the
    // second poll gets a 304 for it, with page 1's old "no next page".
    let grown = Arc::new(AtomicBool::new(false));
    let flag = grown.clone();
    let (base, _) = serve_raw(move |path, req| {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        let mr21 = "/api/v4/projects/grp%2Fapp/merge_requests/21";
        if route != format!("{mr21}/discussions") {
            return fixture_routes(path);
        }
        let (tag, body) = if query.ends_with("page=1") {
            ("p1", discussions(0, 100))
        } else if query.ends_with("page=2") && flag.load(Ordering::SeqCst) {
            ("p2b", discussions(100, 1))
        } else {
            ("p2a", "[]".to_string())
        };
        if req.contains(&format!("if-none-match: \"{tag}\"")) {
            return Reply {
                status: "304 Not Modified",
                headers: format!("ETag: \"{tag}\"\r\n"),
                body: String::new(),
            };
        }
        Reply {
            status: "200 OK",
            headers: format!("ETag: \"{tag}\"\r\nX-Next-Page: \r\n"),
            body,
        }
    });
    let gl = provider(&base);
    let r = mr_ref(&base, 21);
    assert_eq!(gl.fetch_threads(&r).unwrap().len(), 100);
    grown.store(true, Ordering::SeqCst);
    let threads = gl.fetch_threads(&r).unwrap();
    assert_eq!(threads.len(), 101);
    assert_eq!(threads[100].id, "d100");
}

/// MR 21 with `/reviewers` and `/approvals` answered by `reviewers` and
/// `approvals` (status, body); everything else from the fixtures.
fn mr21_with(
    reviewers: (&'static str, &'static str),
    approvals: (&'static str, &'static str),
) -> impl Fn(&str, &str) -> Reply {
    move |path, _| {
        let mr21 = "/api/v4/projects/grp%2Fapp/merge_requests/21";
        let reply = |(status, body): (&'static str, &'static str)| Reply {
            status,
            headers: String::new(),
            body: body.into(),
        };
        match path.split_once('?').map_or(path, |(r, _)| r) {
            r if r == format!("{mr21}/reviewers") => reply(reviewers),
            r if r == format!("{mr21}/approvals") => reply(approvals),
            _ => fixture_routes(path),
        }
    }
}

const NO_APPROVALS: (&str, &str) = ("200 OK", r#"{"approved_by":[]}"#);

#[test]
fn a_failing_reviewers_call_fails_the_fetch_instead_of_guessing() {
    // A 500 from /reviewers must not read as "requested" (the MR's plain
    // reviewer list): that would start a false re-request round.
    let (base, _) = serve_raw(mr21_with(("500 Internal Server Error", "{}"), NO_APPROVALS));
    let err = provider(&base)
        .fetch_review(&mr_ref(&base, 21))
        .unwrap_err();
    assert!(matches!(err, CoreError::Provider(_)), "{err:?}");

    // An instance without the endpoint (404): the plain list stands in.
    let (base, _) = serve_raw(mr21_with(("404 Not Found", "{}"), NO_APPROVALS));
    let d = provider(&base).fetch_review(&mr_ref(&base, 21)).unwrap();
    assert!(d.reviewers.iter().any(|r| r.is_me));
}

#[test]
fn a_failing_approvals_call_fails_the_fetch_or_falls_back_to_reviewer_state() {
    const I_APPROVED: (&str, &str) = (
        "200 OK",
        r#"[{"user":{"id":42,"username":"rev-fixture"},"state":"approved"}]"#,
    );
    // A 500 must not read as "approved by nobody" (clearing the approval).
    let (base, _) = serve_raw(mr21_with(I_APPROVED, ("500 Internal Server Error", "{}")));
    let err = provider(&base)
        .fetch_review(&mr_ref(&base, 21))
        .unwrap_err();
    assert!(matches!(err, CoreError::Provider(_)), "{err:?}");

    // No approvals API here (404, or 403 on a tier without it): the user's
    // own reviewer state says it.
    for status in ["404 Not Found", "403 Forbidden"] {
        let (base, _) = serve_raw(mr21_with(I_APPROVED, (status, "{}")));
        let d = provider(&base).fetch_review(&mr_ref(&base, 21)).unwrap();
        assert!(d.approved_by_me, "{status}");
        assert!(!d.requested_from_me);
    }
}

#[test]
fn the_review_diff_names_the_version_it_was_taken_at() {
    // Asked for an older head (a1): `/diffs` is the latest version's (a2),
    // and the diff is labelled with that version's shas.
    let (base, _) = serve(fixture_routes);
    let stale = DiffRange::Review {
        base: "a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9".into(),
        head: "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1".into(),
    };
    let diff = provider(&base)
        .fetch_files(&mr_ref(&base, 21), &stale)
        .unwrap();
    assert_eq!(
        (diff.base_sha.as_str(), diff.head_sha.as_str()),
        (BASE_SHA, HEAD_SHA)
    );
    assert_eq!(diff.files.len(), 6);
}

#[test]
fn a_version_pushed_while_the_diff_loads_is_read_again() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // The first versions read says a1 is the latest, the next ones a2: the
    // diff read in between may be either, so it is read again.
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    let (base, seen) = serve_raw(move |path, _| {
        let versions = "/api/v4/projects/grp%2Fapp/merge_requests/21/versions";
        if path.starts_with(versions) && count.fetch_add(1, Ordering::SeqCst) == 0 {
            let all: Vec<serde_json::Value> =
                serde_json::from_str(&fixture("mr_21_versions.json")).unwrap();
            return Reply {
                status: "200 OK",
                headers: String::new(),
                body: serde_json::to_string(&all[1..]).unwrap(),
            };
        }
        fixture_routes(path)
    });
    let diff = provider(&base)
        .fetch_files(&mr_ref(&base, 21), &review_range())
        .unwrap();
    assert_eq!(diff.head_sha, HEAD_SHA);
    let diffs_reads = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.contains("/diffs?per_page=100&page=1 "))
        .count();
    assert_eq!(diffs_reads, 2);
}

#[test]
fn discussions_are_read_past_twenty_pages() {
    // 30 pages of one discussion each (the old cap was 20 pages).
    let (base, _) = serve_raw(|path, _| {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        if route != "/api/v4/projects/grp%2Fapp/merge_requests/21/discussions" {
            return fixture_routes(path);
        }
        let page: usize = query
            .rsplit_once("page=")
            .and_then(|(_, n)| n.parse().ok())
            .unwrap_or(1);
        let next = if page < 30 {
            (page + 1).to_string()
        } else {
            String::new()
        };
        Reply {
            status: "200 OK",
            headers: format!("X-Next-Page: {next}\r\n"),
            body: discussions(page, 1),
        }
    });
    let threads = provider(&base).fetch_threads(&mr_ref(&base, 21)).unwrap();
    assert_eq!(threads.len(), 30);
}

#[test]
fn a_failing_commented_mr_is_skipped_not_the_whole_list() {
    let (base, _) = serve(|path| match path {
        "/api/v4/projects/200/merge_requests/5" => status("500 Internal Server Error"),
        _ => fixture_routes(path),
    });
    let list = provider(&base).list_reviews().unwrap();
    let keys: Vec<(&str, u64)> = list.iter().map(|r| (r.repo.as_str(), r.number)).collect();
    assert_eq!(keys, [("grp/app", 21)]);

    // A rate limit still stops the listing.
    let (base, _) = serve(|path| match path {
        "/api/v4/projects/200/merge_requests/5" => Reply {
            status: "429 Too Many Requests",
            headers: "Retry-After: 60\r\n".into(),
            body: String::new(),
        },
        _ => fixture_routes(path),
    });
    let err = provider(&base).list_reviews().unwrap_err();
    assert!(err.to_string().contains("rate limited"), "{err}");
}

#[test]
fn a_merged_commented_mr_is_not_fetched_again() {
    let (base, seen) = serve(fixture_routes);
    let gl = provider(&base);
    gl.list_reviews().unwrap();
    gl.list_reviews().unwrap();
    let count = |p: &str| {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with(&format!("{p} ")))
            .count()
    };
    // !8 is merged: final, read once. !5 is open: read each time.
    assert_eq!(count("/api/v4/projects/300/merge_requests/8"), 1);
    assert_eq!(count("/api/v4/projects/200/merge_requests/5"), 2);
}

/// Exactly 100 discussions on page 1 and no next page; page 2 answers
/// `page2`.
fn full_last_page_then(page2: fn() -> Reply) -> impl Fn(&str, &str) -> Reply {
    move |path, _| {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        if route != "/api/v4/projects/grp%2Fapp/merge_requests/21/discussions" {
            return fixture_routes(path);
        }
        if query.ends_with("page=1") {
            Reply {
                status: "200 OK",
                headers: "X-Next-Page: \r\n".into(),
                body: discussions(0, 100),
            }
        } else {
            page2()
        }
    }
}

#[test]
fn a_page_past_the_end_after_a_full_last_page_ends_the_list() {
    // Page 2 is asked for in case more came in; a 404 or 422 there (no
    // such page) does not fail the complete list.
    for page2 in [(|| status("404 Not Found")) as fn() -> Reply, || {
        status("422 Unprocessable Entity")
    }] {
        let (base, seen) = serve_raw(full_last_page_then(page2));
        let threads = provider(&base).fetch_threads(&mr_ref(&base, 21)).unwrap();
        assert_eq!(threads.len(), 100);
        assert!(seen.lock().unwrap().iter().any(|l| l.contains("page=2")));
    }
}

#[test]
fn a_failing_page_after_a_full_last_page_fails_the_list() {
    // A server error, an auth failure: whether more came is unknown, so the
    // list is not cut there silently.
    for page2 in [
        (|| status("500 Internal Server Error")) as fn() -> Reply,
        || status("401 Unauthorized"),
    ] {
        let (base, _) = serve_raw(full_last_page_then(page2));
        assert!(provider(&base).fetch_threads(&mr_ref(&base, 21)).is_err());
    }

    // A page the list names as next failing is a real gap.
    let (base, _) = serve(|path| {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        if route != "/api/v4/projects/grp%2Fapp/merge_requests/21/discussions" {
            return fixture_routes(path);
        }
        if query.ends_with("page=1") {
            Reply {
                status: "200 OK",
                headers: "X-Next-Page: 2\r\n".into(),
                body: discussions(0, 100),
            }
        } else {
            status("500 Internal Server Error")
        }
    });
    assert!(provider(&base).fetch_threads(&mr_ref(&base, 21)).is_err());
}

#[test]
fn the_head_and_commits_do_not_need_the_reviewer_state() {
    // /reviewers and /approvals failing fail `fetch_review` (see above),
    // not the head check before approving or the history's commit list.
    let (base, seen) = serve_raw(mr21_with(
        ("500 Internal Server Error", "{}"),
        ("500 Internal Server Error", "{}"),
    ));
    let gl = provider(&base);
    let r = mr_ref(&base, 21);
    assert!(gl.fetch_review(&r).is_err());
    let full = {
        let (b, _) = serve(fixture_routes);
        provider(&b).fetch_review(&mr_ref(&b, 21)).unwrap()
    };
    seen.lock().unwrap().clear();
    let head = gl.fetch_head(&r).unwrap();
    assert_eq!(head.head_sha, full.head_sha);
    assert_eq!(head.base_sha, full.base_sha);
    assert_eq!(head.commits, full.commits);
    // Nor are they asked.
    let calls = seen.lock().unwrap().join("\n");
    assert!(
        !calls.contains("/reviewers") && !calls.contains("/approvals"),
        "{calls}"
    );
}

#[test]
fn a_commented_mr_seen_closed_is_checked_again_later() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // !8 was merged... then a closed-and-reopened MR of the same kind: the
    // list skips a final MR only for a while, not until Argus restarts.
    let reopened = Arc::new(AtomicBool::new(false));
    let flag = reopened.clone();
    let (base, _) = serve_raw(move |path, _| {
        if path == "/api/v4/projects/300/merge_requests/8" && flag.load(Ordering::SeqCst) {
            let mr = fixture("mr_300_8.json").replace("\"merged\"", "\"opened\"");
            return Reply {
                status: "200 OK",
                headers: String::new(),
                body: mr,
            };
        }
        fixture_routes(path)
    });
    let gl = provider(&base).with_final_recheck(chrono::Duration::zero());
    let numbers = |list: Vec<ReviewRef>| list.iter().map(|r| r.number).collect::<Vec<_>>();
    assert!(!numbers(gl.list_reviews().unwrap()).contains(&8));
    reopened.store(true, Ordering::SeqCst);
    assert!(numbers(gl.list_reviews().unwrap()).contains(&8));
}

#[test]
fn a_commented_mr_failing_once_stays_listed() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // Listed once; then its read fails for a poll. Dropping it from the list
    // would mark the stored review unlisted (gone from the active tabs)
    // until the next poll: it stays listed as last seen instead.
    let failing = Arc::new(AtomicBool::new(false));
    let flag = failing.clone();
    let (base, _) = serve_raw(move |path, _| {
        if path == "/api/v4/projects/200/merge_requests/5" && flag.load(Ordering::SeqCst) {
            return status("500 Internal Server Error");
        }
        fixture_routes(path)
    });
    let gl = provider(&base);
    let first = gl.list_reviews().unwrap();
    let five = first.iter().find(|r| r.number == 5).unwrap().clone();
    failing.store(true, Ordering::SeqCst);
    let second = gl.list_reviews().unwrap();
    assert_eq!(second.iter().find(|r| r.number == 5), Some(&five));
}

#[test]
fn a_commented_mr_that_keeps_failing_drops_off_the_list() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // A read that fails every poll (a permanent 500 on that MR) keeps it
    // listed as last seen for a few polls only, not for the whole 30 days
    // its comment event stays in the feed.
    let failing = Arc::new(AtomicBool::new(false));
    let flag = failing.clone();
    let (base, _) = serve_raw(move |path, _| {
        if path == "/api/v4/projects/200/merge_requests/5" && flag.load(Ordering::SeqCst) {
            return status("500 Internal Server Error");
        }
        fixture_routes(path)
    });
    let gl = provider(&base);
    let listed = |gl: &GitlabProvider| gl.list_reviews().unwrap().iter().any(|r| r.number == 5);
    assert!(listed(&gl));
    failing.store(true, Ordering::SeqCst);
    for poll in 1..=3 {
        assert!(listed(&gl), "failing poll {poll}");
    }
    assert!(!listed(&gl), "still listed after 4 failing polls");
    // Readable again: listed again, and the count starts over.
    failing.store(false, Ordering::SeqCst);
    assert!(listed(&gl));
    failing.store(true, Ordering::SeqCst);
    assert!(listed(&gl));
}
