//! No error ever carries a provider token: GitHub (`gh`) and GitLab error
//! bodies that echo the token, in every place a platform puts text, for
//! every read and write. The token may be redacted, the message shortened
//! or replaced by Argus' own text; it is never forwarded.
//!
//! GitLab's token is glab's: Argus never knows it, so it cannot redact it
//! by value. The fake `glab` (`support::glab`) holds a fictitious one (a
//! `glpat-` and a 64-hex OAuth-shaped one), sends it to the double, and
//! echoes every failed response body on stderr as glab does; the errors
//! must come out clean all the same.
//!
//! Fictitious tokens only; no network (a fake `gh`, a fake `glab`, a local
//! HTTP double).

#![cfg_attr(windows, allow(dead_code))]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use argus_core::post::{DiffLine, LineKind, PlacedComment};
#[cfg(unix)]
use argus_core::provider::GitlabProvider;
use argus_core::provider::{
    DiffRange, GithubProvider, ProviderId, ReviewProvider, ReviewRef, ReviewVerdict, Side,
};
use argus_core::store::Store;
use argus_core::Core;

mod support;

use support::glab::FakeGlab;
// The fake CLIs are shell scripts: only the Unix tests write them.
#[cfg(unix)]
use support::script;

const GH_TOKEN: &str = "ghp_fixtureTOKENnotreal0123456789";
const GL_TOKEN: &str = "glpat-fixtureTOKENnotreal";
/// The shape of glab's OAuth login (64 hex digits, no prefix).
const GL_OAUTH_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const HEAD: &str = "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2";
const BASE: &str = "a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0";

/// Error bodies with `{T}` where the platform echoes the token.
fn bodies() -> Vec<String> {
    [
        r#"{"message":"Bad credentials {T}"}"#,
        r#"{"message":"Validation Failed","errors":["token {T} is not allowed"]}"#,
        r#"{"message":"Validation Failed","errors":[{"message":"({T}) must be part of the diff"}]}"#,
        r#"{"message":"Validation Failed","errors":[{"field":"{T}","code":"invalid"}]}"#,
        r#"{"message":"line one\nline two {T}\r\n\tend"}"#,
        r#"{"message":"{T}{T} twice, and once more {T}"}"#,
        r#"{"message":"see https://example.test/?access_token={T}&x=1"}"#,
        r#"{"message":{"note":["{T} is bad"],"base":["fine"]}}"#,
        r#"{"message":["{T}","list"]}"#,
        r#"{"error":"invalid_token","error_description":"Token {T} was revoked"}"#,
        // Long enough to be cut, with the token right at the cut.
        &format!(r#"{{"message":"{}{{T}} tail"}}"#, "x".repeat(290)),
        r#"<html>proxy error {T}</html>"#,
        "{T}",
        "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn assert_clean(token: &str, what: &str, text: &str) {
    assert!(!text.contains(token), "{what} leaked the token: {text}");
    // Not even a long piece of it.
    assert!(
        !text.contains(&token[..token.len() - 4]),
        "{what} leaked most of the token: {text}"
    );
}

fn row(kind: LineKind, old_pos: u32, new_pos: u32) -> DiffLine {
    DiffLine {
        kind,
        old_pos,
        new_pos,
    }
}

fn placed(path: &str) -> PlacedComment {
    PlacedComment {
        path: path.into(),
        old_path: path.into(),
        side: Side::New,
        line: row(LineKind::Added, 3, 3),
        start: None,
        in_diff: true,
        excerpt: None,
        head_sha: HEAD.into(),
        base_sha: BASE.into(),
        body: "Why three?".into(),
    }
}

// ------------------------------------------------------------------- GitHub

/// A `gh` that hands out the token (for the `work` account and for the
/// active account, as `gh auth token` does without `--user`) and fails every
/// other call with `stdout` / `stderr` from files next to it. REST reads
/// (`gh api -i`) get the `status` file as their status line first, the way
/// `gh api -i` prints a failed response.
const FAKE_GH: &str = r#"
case "$*" in
  "auth token --hostname github.com --user work") echo "$TOKEN"; exit 0 ;;
  "auth token --hostname github.com") echo "$TOKEN"; exit 0 ;;
esac
cat > /dev/null
if [ "$1 $2" = "api -i" ]; then
  printf 'HTTP/2.0 %s\r\nContent-Type: application/json; charset=utf-8\r\n\r\n' "$(cat "$DIR/status")"
fi
cat "$DIR/stdout"
cat "$DIR/stderr" >&2
exit 1
"#;

/// A fake `gh`; `account` is the configured account (`None`: gh's active
/// one, which Argus then never names).
#[cfg(unix)]
fn fake_gh(account: Option<&str>) -> (tempfile::TempDir, GithubProvider) {
    let dir = tempfile::tempdir().unwrap();
    let gh = dir.path().join("gh");
    let script = format!(
        "#!/bin/sh\nDIR='{}'\nTOKEN='{GH_TOKEN}'\n{FAKE_GH}",
        dir.path().display()
    );
    script::write_executable(&gh, &script);
    std::fs::write(dir.path().join("status"), "500 Internal Server Error").unwrap();
    let provider = GithubProvider::with_binary(
        gh.to_string_lossy().into_owned(),
        "github.com",
        account.map(str::to_string),
    );
    (dir, provider)
}

#[cfg(not(unix))]
fn fake_gh(_: Option<&str>) -> (tempfile::TempDir, GithubProvider) {
    unreachable!("fake gh scripts are Unix-only; these tests are ignored on Windows")
}

fn github_pr() -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: "acme-fixture/widgets".into(),
        number: 12,
        url: "https://github.com/acme-fixture/widgets/pull/12".into(),
        title: "Add widget caching".into(),
        author: "alice-fixture".into(),
        updated_at: None,
    }
}

/// Every provider call's error text, Display and Debug.
fn every_error(p: &dyn ReviewProvider, r: &ReviewRef) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut core = |what, e: argus_core::CoreError| out.push((what, format!("{e} / {e:?}")));
    if let Err(e) = p.current_user() {
        core("current_user", e);
    }
    if let Err(e) = p.list_reviews() {
        core("list_reviews", e);
    }
    if let Err(e) = p.fetch_review(r) {
        core("fetch_review", e);
    }
    if let Err(e) = p.fetch_threads(r) {
        core("fetch_threads", e);
    }
    if let Err(e) = p.file_at(r, HEAD, "src/cache.rs") {
        core("file_at", e);
    }
    let range = DiffRange::Review {
        base: BASE.into(),
        head: HEAD.into(),
    };
    if let Err(e) = p.fetch_files(r, &range) {
        core("fetch_files", e);
    }
    if let Err(e) = p.commit_diff(r, HEAD) {
        core("commit_diff", e);
    }
    if let Err(e) = p.thread_resolutions(r) {
        core("thread_resolutions", e);
    }
    let mut post = |what, e: argus_core::PostError| out.push((what, format!("{e} / {e:?}")));
    if let Err(e) = p.post_comment(r, &placed("src/cache.rs")) {
        post("post_comment", e);
    }
    if let Err(e) = p.reply(r, "4001", "Thanks") {
        post("reply", e);
    }
    if let Err(e) = p.approve(r, HEAD) {
        post("approve", e);
    }
    for resolved in [true, false] {
        if let Err(e) = p.set_thread_resolved(r, "4001", resolved) {
            post("set_thread_resolved", e);
        }
    }
    for verdict in [
        ReviewVerdict::Approve,
        ReviewVerdict::RequestChanges,
        ReviewVerdict::Comment,
    ] {
        if let Err(e) = p.submit_review(r, verdict, HEAD, Some("Summary")) {
            post("submit_review", e);
        }
    }
    out
}

/// Every body through every call, `gh` failing with `stderr` and REST
/// reads answering `status` (one test per shape so they run in parallel),
/// with the `work` account and without one.
fn github_errors_with(status: &str, stderr: &str) {
    for account in [Some("work"), None] {
        let (dir, gh) = fake_gh(account);
        std::fs::write(dir.path().join("status"), status).unwrap();
        let mut checked = 0;
        for body in bodies() {
            std::fs::write(dir.path().join("stdout"), body.replace("{T}", GH_TOKEN)).unwrap();
            std::fs::write(dir.path().join("stderr"), stderr.replace("{T}", GH_TOKEN)).unwrap();
            let errors = every_error(&gh, &github_pr());
            // All but `thread_resolutions` (no such call on GitHub) fail,
            // the three verdicts of `submit_review` included.
            assert_eq!(
                errors.len(),
                15,
                "{account:?} {body} / {stderr}: {errors:?}"
            );
            for (what, text) in errors {
                assert_clean(GH_TOKEN, what, &text);
                checked += 1;
            }
            // A reply in the PR conversation (its own read, then a PR
            // comment) fails clean too.
            for thread in ["issuecomment-4001", "pullrequestreview-4001"] {
                let e = gh
                    .reply(&github_pr(), thread, "Thanks")
                    .expect_err("every gh call fails");
                assert_clean(GH_TOKEN, "conversation reply", &format!("{e} / {e:?}"));
                checked += 1;
            }
        }
        assert!(checked >= 196);
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn github_rejections_never_carry_the_token() {
    github_errors_with(
        "422 Unprocessable Entity",
        "gh: Validation Failed (HTTP 422)",
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn github_not_found_never_carries_the_token() {
    github_errors_with("404 Not Found", "gh: Not Found (HTTP 404)");
}

#[test]
#[cfg_attr(windows, ignore)]
fn github_auth_failures_never_carry_the_token() {
    // gh itself echoing the token on stderr.
    github_errors_with("401 Unauthorized", "gh: Bad credentials (HTTP 401) {T}");
}

#[test]
#[cfg_attr(windows, ignore)]
fn github_server_errors_never_carry_the_token() {
    github_errors_with("502 Bad Gateway", "gh: HTTP 502 {T}");
}

#[test]
#[cfg_attr(windows, ignore)]
fn the_platforms_reason_still_comes_through_redacted() {
    // Without an account too: Argus learns the active account's token from
    // `gh auth token` and hands gh exactly that one, so it can redact it.
    for account in [Some("work"), None] {
        let (dir, gh) = fake_gh(account);
        std::fs::write(
            dir.path().join("stdout"),
            format!(
                r#"{{"message":"Validation Failed","errors":[{{"message":"line must be part of the diff ({GH_TOKEN})"}}]}}"#
            ),
        )
        .unwrap();
        std::fs::write(dir.path().join("stderr"), "gh: (HTTP 422)").unwrap();
        let err = gh
            .post_comment(&github_pr(), &placed("src/cache.rs"))
            .unwrap_err();
        assert_eq!(
            err.message, "GitHub: Validation Failed: line must be part of the diff ([redacted])",
            "{account:?}"
        );
    }
}

/// A `gh` whose GraphQL read of the PR's threads works (thread 4001 has a
/// node id) and whose resolve mutation fails with `stdout` / `stderr`: the
/// write itself is fuzzed, not only the read before it.
const FAKE_GH_RESOLVE: &str = r#"
case "$*" in
  "auth token"*) echo "$TOKEN"; exit 0 ;;
  "api graphql -f query="*)
    echo '{"data":{"repository":{"pullRequest":{"reviewThreads":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"id":"PRRT_fixture_4001","isResolved":false,"isOutdated":false,"comments":{"nodes":[{"url":"https://github.com/acme-fixture/widgets/pull/12#discussion_r4001"}]}}]}}}}}'
    exit 0 ;;
esac
cat > /dev/null
cat "$DIR/stdout"
cat "$DIR/stderr" >&2
exit 1
"#;

#[test]
#[cfg(unix)]
fn github_resolve_refusals_never_carry_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let gh = dir.path().join("gh");
    let script = format!(
        "#!/bin/sh\nDIR='{}'\nTOKEN='{GH_TOKEN}'\n{FAKE_GH_RESOLVE}",
        dir.path().display()
    );
    script::write_executable(&gh, &script);
    let gh = GithubProvider::with_binary(gh.to_string_lossy().into_owned(), "github.com", None);
    // GraphQL's own error shape on top of the REST ones.
    let mut all = bodies();
    all.extend([
        r#"{"data":{"resolveReviewThread":null},"errors":[{"type":"FORBIDDEN","message":"{T} may not resolve"}]}"#.to_string(),
        r#"{"errors":[{"message":"a"},{"message":"{T}"}]}"#.to_string(),
    ]);
    let stderrs = [
        "gh: {T} may not resolve",
        "gh: Bad credentials (HTTP 401) {T}",
        "gh: HTTP 502 {T}",
    ];
    let mut checked = 0;
    for body in &all {
        for stderr in stderrs {
            std::fs::write(dir.path().join("stdout"), body.replace("{T}", GH_TOKEN)).unwrap();
            std::fs::write(dir.path().join("stderr"), stderr.replace("{T}", GH_TOKEN)).unwrap();
            for resolved in [true, false] {
                let e = gh
                    .set_thread_resolved(&github_pr(), "4001", resolved)
                    .expect_err("the mutation fails");
                assert_clean(GH_TOKEN, "set_thread_resolved", &format!("{e} / {e:?}"));
                checked += 1;
            }
        }
    }
    assert_eq!(checked, all.len() * 6);
}

// ------------------------------------------------------------------- GitLab

type Route = Arc<Mutex<(String, String)>>;

/// A local GitLab answering every request with the status and body in
/// `route` — except the versions read a comment needs first.
fn serve(route: Route) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
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
            let length: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(0);
            while raw.len() < head_end + length {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => raw.extend_from_slice(&buf[..n]),
                }
            }
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let (status, body) = if path.contains("/versions") {
                ("200 OK".to_string(), versions())
            } else {
                route.lock().unwrap().clone()
            };
            let out = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            let _ = sock.write_all(out.as_bytes());
        }
    });
    base
}

fn versions() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gitlab/mr_21_versions.json"),
    )
    .unwrap()
}

fn gitlab_mr(base: &str) -> ReviewRef {
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

#[test]
#[cfg_attr(windows, ignore)]
fn gitlab_errors_never_carry_the_token() {
    gitlab_errors_never_carry(
        GL_TOKEN,
        &[
            "400 Bad Request",
            "401 Unauthorized",
            "403 Forbidden",
            "404 Not Found",
            "409 Conflict",
            "422 Unprocessable Entity",
            "429 Too Many Requests",
            "500 Internal Server Error",
            "502 Bad Gateway",
        ],
    );
}

#[test]
#[cfg_attr(windows, ignore)]
fn gitlab_errors_never_carry_an_oauth_token() {
    // The same shapes, fewer statuses: what matters is the token's shape.
    gitlab_errors_never_carry(
        GL_OAUTH_TOKEN,
        &[
            "401 Unauthorized",
            "409 Conflict",
            "500 Internal Server Error",
        ],
    );
}

/// Every body under each of `statuses` through every call, the fake glab
/// logged in with `token`.
fn gitlab_errors_never_carry(token: &str, statuses: &[&str]) {
    let route: Route = Arc::new(Mutex::new((String::new(), String::new())));
    let base = serve(route.clone());
    let fake = FakeGlab::with_token(token);
    let gl = fake.provider(&base);
    let mut checked = 0;
    for body in bodies() {
        for status in statuses {
            *route.lock().unwrap() = (status.to_string(), body.replace("{T}", token));
            let errors = every_error(&gl, &gitlab_mr(&base));
            assert!(errors.len() >= 9, "{status} {body}: {errors:?}");
            for (what, text) in errors {
                assert_clean(token, what, &text);
                checked += 1;
            }
        }
    }
    assert!(checked > statuses.len() * bodies().len() * 9);
    // The token went out on every request, from glab, never from Argus.
    assert!(fake.log().contains("env-token=0"));
    assert!(!fake.calls().iter().any(|c| c.contains(token)));
}

/// glab failing before any response, with the token in what it prints (a
/// debug line, a proxy's echo): stdout and stderr are never forwarded.
#[test]
#[cfg_attr(windows, ignore)]
fn glabs_own_output_never_reaches_an_error() {
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let glab = dir.path().join("glab");
        for (out, err) in [
            ("", "glab: 401 Unauthorized {T} (HTTP 401)"),
            (
                "PRIVATE-TOKEN: {T}",
                "Post \"http://x/api/v4\": dial tcp: lookup x: no such host {T}",
            ),
            ("{T}", "Bearer {T}"),
            ("not http {T}", "panic: {T}"),
        ] {
            let script = format!(
                "#!/bin/sh\ncat > /dev/null\nprintf '%s' '{}'\nprintf '%s' '{}' >&2\nexit 1\n",
                out.replace("{T}", GL_TOKEN),
                err.replace("{T}", GL_TOKEN)
            );
            script::write_executable(&glab, &script);
            let gl =
                GitlabProvider::with_binary(glab.to_string_lossy(), "https://gitlab.example.test")
                    .unwrap()
                    .with_connect_retries(&[std::time::Duration::ZERO]);
            let errors = every_error(&gl, &gitlab_mr("http://gitlab.example.test"));
            assert!(errors.len() >= 9, "{errors:?}");
            for (what, text) in errors {
                assert_clean(GL_TOKEN, what, &text);
            }
            // Its `auth status` answer, the same text: no account message.
            let accounts = argus_core::Core::with_cli(
                argus_core::store::Store::in_memory().unwrap(),
                argus_core::Settings::default(),
                "/nonexistent/argus-fixture/gh",
                glab.to_string_lossy(),
            )
            .list_accounts(true);
            assert_clean(GL_TOKEN, "list_accounts", &format!("{accounts:?}"));
        }
    }
}

/// Submit review's own GitLab path past its reads: the instance and the
/// reviewer check answer, then the reviewer add (when the user is not one
/// yet), the quick action, its check and the summary note fail with the
/// token in the body (or GitLab's 202 summary, or a GraphQL error, echoes
/// it).
#[test]
#[cfg_attr(windows, ignore)]
fn gitlab_submit_errors_never_carry_the_token() {
    let route: Route = Arc::new(Mutex::new((String::new(), String::new())));
    let fail = route.clone();
    let reviewer = Arc::new(AtomicBool::new(true));
    let listed = reviewer.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else { continue };
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => raw.extend_from_slice(&buf[..n]),
                }
            }
            let head = String::from_utf8_lossy(&raw).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let (status, body) = if path.starts_with("/api/v4/version") {
                ("200 OK".to_string(), r#"{"version":"17.5.0"}"#.to_string())
            } else if path.starts_with("/api/v4/user") {
                (
                    "200 OK".to_string(),
                    r#"{"id":42,"username":"rev-fixture"}"#.to_string(),
                )
            } else if path.contains("/reviewers") {
                (
                    "200 OK".to_string(),
                    if listed.load(Ordering::SeqCst) {
                        r#"[{"user":{"id":42,"username":"rev-fixture"},"state":"unreviewed"}]"#
                    } else {
                        "[]"
                    }
                    .to_string(),
                )
            } else {
                fail.lock().unwrap().clone()
            };
            let out = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            let _ = sock.write_all(out.as_bytes());
        }
    });
    let fake = FakeGlab::with_token(GL_TOKEN);
    let gl = fake
        .provider(&base)
        .with_reviewer_recheck(&[std::time::Duration::ZERO]);
    let mut checked = 0;
    let mut cases: Vec<(String, String)> = Vec::new();
    for body in bodies() {
        for status in [
            "400 Bad Request",
            "403 Forbidden",
            "409 Conflict",
            "500 Internal Server Error",
        ] {
            cases.push((status.into(), body.replace("{T}", GL_TOKEN)));
        }
    }
    // Run, but the state did not move: GitLab's summary is forwarded.
    cases.push((
        "202 Accepted".into(),
        format!(r#"{{"commands_changes":{{}},"summary":["Could not: {GL_TOKEN}"]}}"#),
    ));
    for (status, body) in &cases {
        *route.lock().unwrap() = (status.clone(), body.clone());
        for verdict in [ReviewVerdict::Approve, ReviewVerdict::RequestChanges] {
            let err = gl
                .submit_review(&gitlab_mr(&base), verdict, HEAD, Some("Summary"))
                .unwrap_err();
            assert_clean(GL_TOKEN, "submit_review", &format!("{err} / {err:?}"));
            checked += 1;
        }
    }

    // Not a reviewer: the add (GraphQL) fails the same ways, or answers
    // 200 with the token in a GraphQL error or the mutation's own errors.
    reviewer.store(false, Ordering::SeqCst);
    for body in bodies() {
        cases.push((
            "200 OK".into(),
            format!(
                r#"{{"data":{{"mergeRequestSetReviewers":null}},"errors":[{{"message":{}}}]}}"#,
                serde_json::Value::String(body.replace("{T}", GL_TOKEN))
            ),
        ));
        cases.push((
            "200 OK".into(),
            format!(
                r#"{{"data":{{"mergeRequestSetReviewers":{{"errors":[{}],"mergeRequest":null}}}}}}"#,
                serde_json::Value::String(body.replace("{T}", GL_TOKEN))
            ),
        ));
    }
    for (status, body) in cases {
        *route.lock().unwrap() = (status, body);
        let err = gl
            .submit_review(
                &gitlab_mr(&base),
                ReviewVerdict::RequestChanges,
                HEAD,
                Some("Summary"),
            )
            .unwrap_err();
        assert_clean(GL_TOKEN, "submit_review add", &format!("{err} / {err:?}"));
        checked += 1;
    }
    // Both verdicts as a reviewer, then each case once more as none.
    let n = bodies().len();
    assert_eq!(checked, (n * 4 + 1) * 2 + (n * 4 + 1) + n * 2);
}

#[test]
#[cfg_attr(windows, ignore)]
fn refresh_reports_never_carry_the_token() {
    let route: Route = Arc::new(Mutex::new((
        "500 Internal Server Error".into(),
        format!(r#"{{"message":"boom {GL_TOKEN}"}}"#),
    )));
    let base = serve(route.clone());
    let fake = FakeGlab::with_token(GL_TOKEN);
    let gl = fake.provider(&base);
    let core = Core::with_providers(Store::in_memory().unwrap(), vec![Arc::new(gl)]);
    for status in [
        "401 Unauthorized",
        "404 Not Found",
        "500 Internal Server Error",
    ] {
        route.lock().unwrap().0 = status.into();
        let report = core.refresh();
        assert_eq!(report.errors.len(), 1, "{status}");
        let json = serde_json::to_string(&report).unwrap();
        assert_clean(GL_TOKEN, status, &json);
    }
}

/// `r` as stored after an earlier refresh: what a one-review refresh reads.
fn stored(r: ReviewRef) -> argus_core::ReviewRecord {
    argus_core::ReviewRecord {
        id: r.id(),
        review: r,
        state: argus_core::provider::ReviewState::Open,
        draft: false,
        head_sha: HEAD.into(),
        base_sha: BASE.into(),
        last_reviewed_sha: None,
        last_reviewed_at: None,
        approved_at: None,
        round: 1,
        round_reasons: Vec::new(),
        status: argus_core::ReviewStatus::NeedsReview,
        untouched_threads: 0,
        requested: true,
        rerequested_at: None,
        my_review_state: argus_core::MyReviewState::AwaitingReview,
        my_reviewed_at: None,
        my_reviewed_sha: None,
        listed: true,
        viewer: "rev-fixture".into(),
        fingerprint: String::new(),
        fetched_at: chrono::Utc::now(),
    }
}

/// A core holding `r`, over `provider`.
fn core_holding(r: ReviewRef, provider: Arc<dyn ReviewProvider>) -> Core {
    let store = Store::in_memory().unwrap();
    store.upsert_review(&stored(r)).unwrap();
    Core::with_providers(store, vec![provider])
}

#[test]
#[cfg_attr(windows, ignore)]
fn refreshing_one_github_review_never_carries_the_token() {
    for status in [
        "401 Unauthorized",
        "404 Not Found",
        "500 Internal Server Error",
    ] {
        let (dir, gh) = fake_gh(Some("work"));
        std::fs::write(dir.path().join("status"), status).unwrap();
        let core = core_holding(github_pr(), Arc::new(gh));
        let id = github_pr().id();
        for body in bodies() {
            std::fs::write(dir.path().join("stdout"), body.replace("{T}", GH_TOKEN)).unwrap();
            std::fs::write(
                dir.path().join("stderr"),
                format!("gh: {} {GH_TOKEN}", status),
            )
            .unwrap();
            let e = core.refresh_review(id).unwrap_err();
            assert_clean(GH_TOKEN, status, &format!("{e} / {e:?}"));
        }
    }
}

#[test]
#[cfg_attr(windows, ignore)]
fn refreshing_one_gitlab_review_never_carries_the_token() {
    let route: Route = Arc::new(Mutex::new((String::new(), String::new())));
    let base = serve(route.clone());
    let fake = FakeGlab::with_token(GL_TOKEN);
    let gl = fake.provider(&base);
    let core = core_holding(gitlab_mr(&base), Arc::new(gl));
    let id = gitlab_mr(&base).id();
    let mut checked = 0;
    for body in bodies() {
        // No 429: its pause would stop every call after it.
        for status in [
            "400 Bad Request",
            "401 Unauthorized",
            "403 Forbidden",
            "404 Not Found",
            "500 Internal Server Error",
        ] {
            *route.lock().unwrap() = (status.to_string(), body.replace("{T}", GL_TOKEN));
            let e = core.refresh_review(id).unwrap_err();
            assert_clean(GL_TOKEN, status, &format!("{e} / {e:?}"));
            checked += 1;
        }
    }
    // A rate limit's message is clean too.
    *route.lock().unwrap() = (
        "429 Too Many Requests".into(),
        format!(r#"{{"message":"slow down {GL_TOKEN}"}}"#),
    );
    let e = core.refresh_review(id).unwrap_err();
    assert!(e.is_rate_limited(), "{e}");
    assert_clean(GL_TOKEN, "429", &format!("{e} / {e:?}"));
    assert!(checked > 50);
}

/// `gh auth status` and `glab auth status` answers with tokens in them (a
/// `token` field, an error echoing one, a text answer printing one): no
/// account row, message or hint ever carries it.
#[test]
#[cfg_attr(windows, ignore)]
fn account_status_output_never_carries_a_token() {
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.path().join(name);
            script::write_executable(&path, &format!("#!/bin/sh\n{body}\n"));
            path.to_string_lossy().into_owned()
        };
        let gh_answers = [
            // A `token` field and the token in gh's error.
            format!(
                r#"echo '{{"hosts":{{"github.com":[{{"state":"error","error":"Bad credentials {GH_TOKEN}","active":true,"host":"github.com","login":"rev-fixture","token":"{GH_TOKEN}"}},{{"state":"timeout","error":"{GH_TOKEN}","active":false,"host":"github.com","login":"work-fixture"}}]}}}}'"#
            ),
            // A gh without --json whose text prints the token.
            format!(
                r#"case "$*" in *--json*) echo 'unknown flag: --json' >&2; exit 1 ;; esac
printf 'github.com\n  X Failed to log in to github.com account rev-fixture (keyring)\n  - Token: {GH_TOKEN}\n  - {GH_TOKEN}\n' >&2; exit 1"#
            ),
            // Garbage with the token.
            format!("echo '{GH_TOKEN}'"),
        ];
        let glab_answers = [
            format!(
                r#"echo '{{"hosts":[{{"host":"gitlab.example.test","state":"error","error":"PRIVATE-TOKEN: {GL_TOKEN} rejected by proxy","token_source":"{GL_TOKEN}"}},{{"host":"gitlab.two.test","state":"error","error":"Bearer {GL_OAUTH_TOKEN}"}}]}}'; exit 1"#
            ),
            format!(
                r#"case "$*" in *--all*) echo 'unknown flag: --all' >&2; exit 1 ;; esac
case "$*" in *--output*) echo 'unknown flag: --output' >&2; exit 1 ;; esac
printf 'gitlab.example.test\n  x gitlab.example.test: API call failed: {GL_TOKEN}\n  Token: {GL_TOKEN}\n'"#
            ),
            format!("echo '{GL_TOKEN}' >&2; exit 1"),
        ];
        for (i, (gh, glab)) in gh_answers.iter().zip(glab_answers.iter()).enumerate() {
            let gh = write(&format!("gh{i}"), gh);
            let glab = write(&format!("glab{i}"), glab);
            let core = Core::with_cli(
                Store::in_memory().unwrap(),
                argus_core::Settings::default(),
                gh,
                glab,
            );
            let accounts = core.list_accounts(true);
            assert!(!accounts.is_empty(), "{i}");
            let text = format!("{accounts:?} {}", serde_json::to_string(&accounts).unwrap());
            assert_clean(GH_TOKEN, "list_accounts (gh)", &text);
            assert_clean(GL_TOKEN, "list_accounts (glab)", &text);
            assert_clean(GL_OAUTH_TOKEN, "list_accounts (glab oauth)", &text);
        }
    }
}
