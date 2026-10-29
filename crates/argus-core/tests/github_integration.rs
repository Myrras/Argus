//! The GitHub provider against the real GitHub API, read-only.
//!
//! Opt-in: every test is `#[ignore]`d, so `cargo test` skips them; run them
//! with `just test-integration` (nightly in CI). They need the `gh` CLI and a
//! token in `INTEGRATION_TOKEN`; without the token each test says so and
//! returns. The target is the public repo `Myrras/rust-base`
//! (merged PRs #1 and #2), so a token with read access to public repos is
//! enough. Nothing here writes to GitHub: only the provider's read methods
//! are called.
//!
//! GitLab has no real-API test: its instance is internal and unreachable from
//! GitHub runners; `gitlab_provider.rs` covers it against a local double.
//!
//! The token never reaches an assertion message or a log line.

use std::sync::Once;

use argus_core::provider::{
    DiffRange, FileDiff, FileStatus, GithubProvider, ProviderId, ReviewProvider, ReviewRef,
    ReviewState,
};
use argus_core::store::Store;
use argus_core::{AccountState, Core, Settings};

const REPO: &str = "Myrras/rust-base";
/// PR #2 "Lean CI defaults": one commit on top of `PR2_BASE`, merged.
const PR2_TITLE: &str = "Lean CI defaults: ubuntu rust job, Windows on main or by label";
const PR2_HEAD: &str = "ef8723eb59a8fc4d92ef3903fc06e415c61ec764";
const PR2_BASE: &str = "b11405c28608a2557ec9e075e1f5bebe5810f2dd";
const PR1_TITLE: &str =
    "Bump actions/upload-artifact from 7.0.1 to 7.0.2 in the github-actions group";

/// The provider on the integration token, or `None` (with a message) when
/// `INTEGRATION_TOKEN` is unset. The provider asks `gh auth token` for its
/// token, and `gh` answers that from `GH_TOKEN`, so the integration token is
/// handed over once through that variable. It is set once for the whole test
/// binary, before any provider runs.
fn provider() -> Option<GithubProvider> {
    static INIT: Once = Once::new();
    let token = std::env::var("INTEGRATION_TOKEN").unwrap_or_default();
    if token.trim().is_empty() {
        eprintln!(
            "skipped: INTEGRATION_TOKEN is not set (a GitHub token that can read public repos)"
        );
        return None;
    }
    INIT.call_once(|| std::env::set_var("GH_TOKEN", token.trim()));
    Some(GithubProvider::new("github.com", None))
}

fn pr(number: u64, title: &str) -> ReviewRef {
    ReviewRef {
        provider: ProviderId::Github,
        host: "github.com".into(),
        repo: REPO.into(),
        number,
        url: format!("https://github.com/{REPO}/pull/{number}"),
        title: title.into(),
        author: String::new(),
        updated_at: None,
    }
}

fn paths(diff: &FileDiff) -> Vec<&str> {
    diff.files.iter().map(|f| f.path.as_str()).collect()
}

#[test]
#[ignore = "integration: real GitHub API, needs INTEGRATION_TOKEN; run with just test-integration"]
fn gh_is_installed_and_logged_in() {
    if provider().is_none() {
        return;
    }
    // Settings → Accounts as the app sees it: what `gh auth status` says
    // about github.com (`glab` may be missing; only GitHub is checked).
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        "gh",
        "glab",
    );
    let accounts = core.list_accounts(true);
    let github = accounts
        .iter()
        .find(|a| a.provider == ProviderId::Github && a.host == "github.com");
    let state = github.map(|a| &a.state);
    assert!(
        !matches!(state, Some(AccountState::CliMissing { .. })),
        "the gh CLI is not installed"
    );
    assert_eq!(
        state,
        Some(&AccountState::LoggedIn),
        "gh has no login for github.com (is INTEGRATION_TOKEN valid?)"
    );
}

#[test]
#[ignore = "integration: real GitHub API, needs INTEGRATION_TOKEN; run with just test-integration"]
fn current_user_and_review_list_read() {
    let Some(gh) = provider() else { return };
    assert!(!gh.current_user().expect("current_user").is_empty());
    // Whose PRs these are depends on the token; only that the three searches
    // succeed and every item is a GitHub one.
    let list = gh.list_reviews().expect("list_reviews");
    assert!(list.iter().all(|r| r.provider == ProviderId::Github));
}

#[test]
#[ignore = "integration: real GitHub API, needs INTEGRATION_TOKEN; run with just test-integration"]
fn fetches_a_merged_pull_request() {
    let Some(gh) = provider() else { return };
    let detail = gh.fetch_review(&pr(2, "")).expect("fetch_review #2");
    assert_eq!(detail.review.title, PR2_TITLE);
    assert_eq!(detail.review.author, "Myrras");
    assert_eq!(detail.review.repo, REPO);
    assert_eq!(detail.state, ReviewState::Merged);
    assert!(!detail.draft);
    assert_eq!(detail.head_sha, PR2_HEAD);
    assert_eq!(detail.base_sha, PR2_BASE);
    assert_eq!(detail.commits.len(), 1);
    assert_eq!(detail.commits[0].sha, PR2_HEAD);
    assert_eq!(detail.commits[0].title, PR2_TITLE);
    // A merged PR is nobody's to review any more.
    assert!(!detail.requested_from_me);

    let head = gh.fetch_head(&pr(2, "")).expect("fetch_head #2");
    assert_eq!(
        (head.head_sha.as_str(), head.base_sha.as_str()),
        (PR2_HEAD, PR2_BASE)
    );
    assert_eq!(head.commits, detail.commits);

    let first = gh.fetch_review(&pr(1, "")).expect("fetch_review #1");
    assert_eq!(first.review.title, PR1_TITLE);
    assert_eq!(first.state, ReviewState::Merged);
}

#[test]
#[ignore = "integration: real GitHub API, needs INTEGRATION_TOKEN; run with just test-integration"]
fn reads_the_files_of_a_pull_request() {
    let Some(gh) = provider() else { return };
    let diff = gh
        .fetch_files(
            &pr(2, PR2_TITLE),
            &DiffRange::Review {
                base: PR2_BASE.into(),
                head: PR2_HEAD.into(),
            },
        )
        .expect("fetch_files #2");
    assert_eq!(diff.head_sha, PR2_HEAD);
    assert_eq!(diff.base_sha, PR2_BASE, "merge base of the PR");
    assert!(!diff.truncated && !diff.base_approximate);
    let files = paths(&diff);
    for expected in [".github/workflows/tauri-ci.yml", "README.md", "common.just"] {
        assert!(
            files.contains(&expected),
            "{expected} missing from {files:?}"
        );
    }
    let ci = diff
        .files
        .iter()
        .find(|f| f.path == ".github/workflows/tauri-ci.yml")
        .unwrap();
    assert_eq!(ci.status, FileStatus::Modified);
    assert!(ci.additions > 0 && ci.deletions > 0);
    assert!(!ci.binary && !ci.too_large);
    assert!(ci.patch.as_deref().is_some_and(|p| p.starts_with("@@")));
    assert!(!ci.content_sha.is_empty());

    // The dependabot PR touched one workflow file; its shas come from the
    // provider, as the app's own poll does.
    let head = gh.fetch_head(&pr(1, PR1_TITLE)).expect("fetch_head #1");
    let first = gh
        .fetch_files(
            &pr(1, PR1_TITLE),
            &DiffRange::Review {
                base: head.base_sha,
                head: head.head_sha,
            },
        )
        .expect("fetch_files #1");
    assert_eq!(paths(&first), [".github/workflows/build.yml"]);
}

#[test]
#[ignore = "integration: real GitHub API, needs INTEGRATION_TOKEN; run with just test-integration"]
fn compares_two_commits_and_reads_a_file() {
    let Some(gh) = provider() else { return };
    let r = pr(2, PR2_TITLE);
    let diff = gh
        .fetch_files(
            &r,
            &DiffRange::Compare {
                from: PR2_BASE.into(),
                to: PR2_HEAD.into(),
            },
        )
        .expect("compare");
    assert_eq!(
        (diff.base_sha.as_str(), diff.head_sha.as_str()),
        (PR2_BASE, PR2_HEAD)
    );
    assert!(paths(&diff).contains(&".github/workflows/tauri-ci.yml"));

    let common = gh.file_at(&r, PR2_HEAD, "common.just").expect("file_at");
    assert!(common.contains("TEST_SCOPE"), "common.just lost TEST_SCOPE");
    // The path is encoded for the API, including its directories.
    let wf = gh
        .file_at(&r, PR2_HEAD, ".github/workflows/tauri-ci.yml")
        .expect("file_at in a directory");
    assert!(wf.contains("name: Tauri CI"));
    // A missing path is an error, not an empty file.
    assert!(gh.file_at(&r, PR2_HEAD, "no/such/file.txt").is_err());
}

#[test]
#[ignore = "integration: real GitHub API, needs INTEGRATION_TOKEN; run with just test-integration"]
fn reads_the_threads_of_a_pull_request() {
    let Some(gh) = provider() else { return };
    // PR #2 has no comments or reviews today, so the list is empty; the
    // assertion holds if some are added later (every thread has a note).
    let threads = gh.fetch_threads(&pr(2, PR2_TITLE)).expect("fetch_threads");
    assert!(threads.iter().all(|t| !t.notes.is_empty()));
}
