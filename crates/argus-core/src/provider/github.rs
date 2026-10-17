//! GitHub, via the `gh` CLI — exactly Podium's mechanism, so Argus never
//! stores a GitHub token.
//!
//! `gh auth token --hostname <host> [--user <account>]` picks the configured
//! account's token, or the active account's without one. It goes to every
//! later `gh` call through `GH_TOKEN` (`GH_ENTERPRISE_TOKEN` off github.com)
//! and nowhere else, so Argus knows exactly the token gh sends and can redact
//! it from any GitHub text it shows. A host other than
//! github.com is selected with `GH_HOST`, which both `gh search` and `gh api`
//! honour.
//!
//! `gh`'s stderr is only inspected to classify a failure, never forwarded.
//!
//! REST reads go through `gh api -i` (status line and headers on stdout) one
//! page at a time, so each page is a conditional request: its last `ETag` is
//! sent as `If-None-Match` and a 304 replays the cached body (and next-page
//! link). Searches (`gh search prs`) stay unconditional. Every `gh` call is
//! killed after a timeout. Thread resolved state comes from one GraphQL query
//! per PR (REST has none).

use std::collections::{BTreeMap, HashMap};
use std::process::{Output, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::conditional::{Cached, ConditionalCache};
use super::model::{
    Anchor, ChangedFile, CommitInfo, DiffRange, FileDiff, FileStatus, Note, ProviderId,
    ReviewDetail, ReviewHead, ReviewRef, ReviewState, Reviewer, ReviewerState, Side, Thread,
};
use super::rate_limit::{default_reset, reset_at, Backoff};
use super::{encode_path, first_line, looks_binary, PostResultOf, ReviewProvider};
use crate::error::{CoreError, CoreResult};
use crate::platform::{run_with_timeout, ExecError};
use crate::post::{sanitize, PlacedComment, PostError, PostErrorKind, PostedComment, PostedNote};

/// Upper bound per search; a reviewer with more than this many open PRs in
/// one bucket is not the user Argus is built for.
const SEARCH_LIMIT: &str = "100";
const SEARCH_FIELDS: &str = "number,title,url,repository,author,updatedAt";

/// GitHub's caps: `pulls/{n}/files` lists at most 3000 files, a compare at
/// most 300. Reaching one means the list is cut off.
const PULL_FILES_LIMIT: usize = 3000;
const COMPARE_FILES_LIMIT: usize = 300;

/// Page size of every list read.
const PER_PAGE: usize = 100;
/// Pages followed at most per list ([`PER_PAGE`] each: past the 3000-file
/// cap).
const MAX_PAGES: usize = 40;

/// How long one `gh` call may run before it is killed. Generous: a large
/// PR's file page can take a while on a slow link, but a hung call must not
/// hold the poll loop.
const GH_TIMEOUT: Duration = Duration::from_secs(60);
/// `gh auth token` is a local lookup; it only hangs on a stuck keychain.
const AUTH_TIMEOUT: Duration = Duration::from_secs(15);

/// The resolved and outdated state of every review thread of one PR, a page
/// at a time. Threads are matched to the REST ones by their first comment's
/// URL (`…#discussion_r<id>`, the REST root comment id): `databaseId` is a
/// 32-bit `Int` that review comment ids have outgrown.
const THREADS_QUERY: &str =
    "query($owner: String!, $name: String!, $number: Int!, $after: String) { \
repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
reviewThreads(first: 100, after: $after) { pageInfo { hasNextPage endCursor } \
nodes { isResolved isOutdated comments(first: 1) { nodes { url } } } } } } }";

/// What [`GithubProvider::auth_status`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GhAuth {
    /// The `gh` program could be started.
    pub installed: bool,
    /// `gh` has a token for the host (and account).
    pub logged_in: bool,
}

/// GitHub through the `gh` binary.
pub struct GithubProvider {
    /// Program to run; `gh` normally, a fake script in tests (the same
    /// injection Podium uses instead of mutating `PATH`).
    gh: String,
    host: String,
    account: Option<String>,
    /// The token handed to gh (configured or active account), held in
    /// memory only, for `GH_TOKEN` and for redaction.
    token: Mutex<Option<String>>,
    me: Mutex<Option<String>>,
    timeout: Duration,
    /// Conditional-request cache of REST reads, keyed by `gh api` path.
    cache: ConditionalCache,
    limit: Backoff,
}

impl GithubProvider {
    /// `host` is `github.com` or a GHE host; `account` picks a logged-in `gh`
    /// account, `None` uses the active one.
    pub fn new(host: impl Into<String>, account: Option<String>) -> Self {
        // Not a bare "gh": an app started from Finder has no Homebrew on its
        // PATH. When gh is not installed the bare name makes the spawn fail,
        // which reports as "not installed" rather than panicking.
        Self::with_binary(crate::platform::gh_binary().unwrap_or("gh"), host, account)
    }

    /// Like [`new`](Self::new) with an explicit `gh` program (tests).
    pub fn with_binary(
        gh: impl Into<String>,
        host: impl Into<String>,
        account: Option<String>,
    ) -> Self {
        let host = host.into();
        let host = if host.trim().is_empty() {
            "github.com".to_string()
        } else {
            host.trim().to_string()
        };
        Self {
            gh: gh.into(),
            host,
            account: account.filter(|a| !a.trim().is_empty()),
            token: Mutex::new(None),
            me: Mutex::new(None),
            timeout: GH_TIMEOUT,
            cache: ConditionalCache::default(),
            limit: Backoff::default(),
        }
    }

    /// Kill a `gh` call after `timeout` instead of the default minute.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.gh);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1");
        if self.host != "github.com" {
            cmd.env("GH_HOST", &self.host);
        }
        #[cfg(windows)]
        {
            // No console window flashing up for every poll.
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd
    }

    /// The token gh is to use, fetched once and kept in memory: the
    /// configured account's, else the active account's. Knowing it even
    /// without an account matters: GitHub can echo the token in an error
    /// body, and only a known token can be redacted.
    ///
    /// With an account, no token is an error (that account is not logged
    /// in). Without one, a failed lookup is not: `None` lets gh authenticate
    /// on its own (an old gh without `auth token`), and fail on its own if it
    /// cannot.
    fn token(&self) -> CoreResult<Option<String>> {
        let mut cached = self.token.lock().expect("token lock poisoned");
        if let Some(t) = cached.as_ref() {
            return Ok(Some(t.clone()));
        }
        let mut cmd = self.command();
        cmd.args(["auth", "token", "--hostname", &self.host]);
        if let Some(account) = &self.account {
            cmd.args(["--user", account]);
        }
        cmd.stderr(Stdio::null());
        let token = match run_with_timeout(&mut cmd, None, self.timeout.min(AUTH_TIMEOUT)) {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => String::new(),
        };
        if token.is_empty() {
            return match self.account {
                Some(_) => Err(CoreError::GithubUnavailable),
                None => Ok(None),
            };
        }
        *cached = Some(token.clone());
        Ok(Some(token))
    }

    /// Whether `gh` runs at all, and whether it holds a token for the host
    /// (and the configured account). `gh auth token` is a local lookup, so
    /// this never touches the network; its output (the token) is discarded
    /// unread.
    pub fn auth_status(&self) -> GhAuth {
        let mut cmd = self.command();
        cmd.args(["auth", "token", "--hostname", &self.host]);
        if let Some(account) = &self.account {
            cmd.args(["--user", account]);
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        match run_with_timeout(&mut cmd, None, self.timeout.min(AUTH_TIMEOUT)) {
            Ok(output) => GhAuth {
                installed: true,
                logged_in: output.status.success(),
            },
            // It ran but never answered: there, but not usable.
            Err(ExecError::TimedOut) => GhAuth {
                installed: true,
                logged_in: false,
            },
            Err(_) => GhAuth {
                installed: false,
                logged_in: false,
            },
        }
    }

    /// Run `gh` with `args` (and `input` on stdin), the account's token in
    /// its env, killed after the timeout.
    fn exec(
        &self,
        token: Option<&str>,
        args: &[&str],
        input: Option<Vec<u8>>,
    ) -> Result<Output, ExecError> {
        let mut cmd = self.command();
        cmd.args(args);
        if let Some(token) = token {
            let var = if self.host == "github.com" {
                "GH_TOKEN"
            } else {
                "GH_ENTERPRISE_TOKEN"
            };
            cmd.env(var, token);
        }
        run_with_timeout(&mut cmd, input, self.timeout)
    }

    /// Classify a failed `gh` call from its stderr (never forwarded).
    fn failure(&self, stderr: &[u8]) -> CoreError {
        let stderr = String::from_utf8_lossy(stderr).to_lowercase();
        if stderr.contains("auth login")
            || stderr.contains("not logged")
            || stderr.contains("http 401")
        {
            // A stale picked-account token: fetch it afresh next time.
            *self.token.lock().expect("token lock poisoned") = None;
            CoreError::GithubUnavailable
        } else if stderr.contains("http 404") || stderr.contains("not found") {
            CoreError::NotFound
        } else if stderr.contains("rate limit") {
            // No headers to read the reset from (`gh search`): the default.
            self.limit
                .hit(ProviderId::Github, default_reset(Utc::now()))
        } else {
            CoreError::Provider("gh could not fetch from GitHub".into())
        }
    }

    /// Run one unconditional `gh` call (searches) and return its stdout.
    fn run(&self, args: &[&str]) -> CoreResult<String> {
        self.limit.check(ProviderId::Github)?;
        let token = self.token()?;
        let output = self.exec(token.as_deref(), args, None)?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        tracing::debug!("gh {} failed", args.first().unwrap_or(&""));
        Err(self.failure(&output.stderr))
    }

    /// One conditional REST GET of `path` (optionally through `--jq`): the
    /// body, from the cache on a 304, and the next page's query.
    fn get(&self, path: &str, jq: Option<&str>) -> CoreResult<Page> {
        self.limit.check(ProviderId::Github)?;
        let token = self.token()?;
        let key = match jq {
            Some(filter) => format!("{path} --jq {filter}"),
            None => path.to_string(),
        };
        let cached = self.cache.get(&key);
        let header = cached
            .as_deref()
            .and_then(Cached::validator)
            .map(|(name, value)| format!("{name}: {value}"));
        let mut args = vec!["api", "-i"];
        if let Some(h) = &header {
            args.extend(["-H", h]);
        }
        args.push(path);
        if let Some(filter) = jq {
            args.extend(["--jq", filter]);
        }
        let output = self.exec(token.as_deref(), &args, None)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some(reply) = HttpReply::parse(&stdout) else {
            // gh failed before any response (logged out, offline), or a gh
            // that prints no headers: classify as before.
            if output.status.success() {
                return Ok(Page {
                    body: stdout.into_owned(),
                    next: None,
                });
            }
            tracing::debug!("gh api failed");
            return Err(self.failure(&output.stderr));
        };
        match reply.status {
            200..=299 => {
                let next = reply.header("link").and_then(|l| next_query(&l));
                self.cache.put(
                    &key,
                    Cached {
                        etag: reply.header("etag"),
                        last_modified: reply.header("last-modified"),
                        body: reply.body.clone(),
                        next: next.clone(),
                    },
                );
                Ok(Page {
                    body: reply.body,
                    next,
                })
            }
            // Unchanged: the cached response stands (and cost no rate limit).
            304 => cached
                .map(|c| Page {
                    body: c.body.clone(),
                    next: c.next.clone(),
                })
                .ok_or_else(unexpected),
            401 => {
                *self.token.lock().expect("token lock poisoned") = None;
                Err(CoreError::GithubUnavailable)
            }
            403 | 429 if reply.is_rate_limited() => Err(self.limit.hit(
                ProviderId::Github,
                reset_at(|h| reply.header(h), Utc::now()),
            )),
            404 => Err(CoreError::NotFound),
            status => {
                tracing::debug!("gh api answered HTTP {status}");
                Err(CoreError::Provider("gh could not fetch from GitHub".into()))
            }
        }
    }

    fn api<T: DeserializeOwned>(&self, path: &str) -> CoreResult<T> {
        let page = self.get(path, None)?;
        serde_json::from_str(&page.body).map_err(|_| unexpected())
    }

    /// Every page of a list endpoint, one conditional request each, following
    /// the `Link` header's `rel="next"` (its query only: the host stays the
    /// one `gh` is set up for).
    ///
    /// A full page (an array of [`PER_PAGE`]) without a next link is
    /// followed anyway: a 304 replays the link as it was when the page was
    /// cached, and a last page of exactly [`PER_PAGE`] stays unchanged (a
    /// 304) when more items arrive after it. An empty next page ends it, and
    /// so does that unasked-for page failing (any error but a rate limit):
    /// the list read so far is complete as far as GitHub said. Not at
    /// [`PULL_FILES_LIMIT`] items: GitHub serves no page past its cap (a
    /// 422), and the cap is where a cut list ends anyway.
    fn pages(&self, path: &str) -> CoreResult<Vec<String>> {
        let (base, query) = path.split_once('?').unwrap_or((path, ""));
        let first = if query.is_empty() {
            format!("{base}?per_page={PER_PAGE}")
        } else {
            format!("{base}?{query}&per_page={PER_PAGE}")
        };
        let mut url = first.clone();
        let mut number = 1;
        let mut items = 0;
        let mut guessed = false;
        let mut bodies = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = match self.get(&url, None) {
                Ok(p) => p,
                Err(e) if guessed && !super::is_rate_limited(&e) => {
                    tracing::debug!("a GitHub list ends at a page it did not link: {e}");
                    break;
                }
                Err(e) => return Err(e),
            };
            let count = serde_json::from_str::<Vec<serde::de::IgnoredAny>>(&page.body)
                .map_or(0, |items| items.len());
            items += count;
            bodies.push(page.body);
            match page.next {
                Some(q) if format!("{base}?{q}") != url => {
                    number = page_number(&q).unwrap_or(number + 1);
                    url = format!("{base}?{q}");
                    guessed = false;
                }
                None if count >= PER_PAGE && items < PULL_FILES_LIMIT => {
                    number += 1;
                    url = format!("{first}&page={number}");
                    guessed = true;
                }
                _ => break,
            }
        }
        Ok(bodies)
    }

    /// A paginated list endpoint, flattened.
    fn api_list<T: DeserializeOwned>(&self, path: &str) -> CoreResult<Vec<T>> {
        let mut all = Vec::new();
        for body in self.pages(path)? {
            all.extend(parse_pages::<T>(&body)?);
        }
        Ok(all)
    }

    /// The resolved / outdated state of the PR's review threads by REST root
    /// comment id. `None` when the query fails (GraphQL off on a GHE, a
    /// token without the scope, a GraphQL rate limit): the threads then stay
    /// unresolved, as before, and the refresh goes on.
    fn thread_states(&self, r: &ReviewRef) -> Option<HashMap<String, ThreadState>> {
        // GraphQL has its own budget, but when REST is limited the poll is
        // backing off anyway.
        if self.limit.active().is_some() {
            return None;
        }
        let (owner, name) = r.repo.split_once('/')?;
        let token = self.token().ok()?;
        let query = format!("query={THREADS_QUERY}");
        let owner = format!("owner={owner}");
        let name = format!("name={name}");
        let number = format!("number={}", r.number);
        let mut states = HashMap::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let after_field = after.as_ref().map(|c| format!("after={c}"));
            // `-f` passes owner and name as raw strings; `-F` types the
            // number as an Int.
            let mut args = vec![
                "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-F", &number,
            ];
            if let Some(a) = &after_field {
                args.extend(["-f", a]);
            }
            let output = match self.exec(token.as_deref(), &args, None) {
                Ok(o) if o.status.success() => o,
                _ => {
                    tracing::debug!("GitHub thread states unavailable");
                    return None;
                }
            };
            let page: GqlResponse = serde_json::from_slice(&output.stdout).ok()?;
            let threads = page.data?.repository?.pull_request?.review_threads;
            for t in threads.nodes {
                let Some(id) = t
                    .comments
                    .nodes
                    .first()
                    .and_then(|c| root_comment_id(&c.url))
                else {
                    continue;
                };
                states.insert(
                    id,
                    ThreadState {
                        resolved: t.is_resolved,
                        outdated: t.is_outdated,
                    },
                );
            }
            match threads.page_info {
                GqlPageInfo {
                    has_next_page: true,
                    end_cursor: Some(c),
                } => after = Some(c),
                _ => break,
            }
        }
        Some(states)
    }

    fn search(&self, qualifier: &str) -> CoreResult<Vec<GhSearchItem>> {
        let out = self.run(&[
            "search",
            "prs",
            qualifier,
            "--state=open",
            "--limit",
            SEARCH_LIMIT,
            "--json",
            SEARCH_FIELDS,
        ])?;
        serde_json::from_str(&out).map_err(|_| unexpected())
    }

    fn me(&self) -> CoreResult<String> {
        let mut me = self.me.lock().expect("me lock poisoned");
        if let Some(m) = me.as_ref() {
            return Ok(m.clone());
        }
        let user: GhUser = self.api("user")?;
        *me = Some(user.login.clone());
        Ok(user.login)
    }
}

impl GithubProvider {
    /// The PR's own diff, labelled with the head it is of. `pulls/{n}/files`
    /// is always the PR as it is now, whatever head was asked for, so the
    /// PR is read before and after the files (conditional: normally two
    /// 304s); the same head both times is the files' head. A push in
    /// between reads it all once more.
    ///
    /// The files are diffed from the merge base (three-dot), not from the
    /// base branch tip the PR reports: the compare API of the PR's own base
    /// and head names that commit. When GitHub cannot compare (a PR too
    /// large for it), the PR's base commit stands in.
    fn review_diff(&self, r: &ReviewRef) -> CoreResult<FileDiff> {
        let pull_path = format!("repos/{}/pulls/{}", r.repo, r.number);
        for _ in 0..2 {
            let pull: GhPull = self.api(&pull_path)?;
            let (base, head) = (pull.base.sha, pull.head.sha);
            check_sha(&base).map_err(|_| unexpected())?;
            check_sha(&head).map_err(|_| unexpected())?;
            let (files, merge_base) = std::thread::scope(|s| {
                let a = s.spawn(|| self.api_list::<GhFile>(&format!("{pull_path}/files")));
                let b = s.spawn(|| {
                    self.get(
                        &format!("repos/{}/compare/{base}...{head}", r.repo),
                        Some(".merge_base_commit.sha"),
                    )
                });
                (join(a), join(b))
            });
            let files = files?;
            let (merge_base, base_approximate) = match merge_base {
                Ok(page) => {
                    let sha = page.body.trim().to_string();
                    check_sha(&sha).map_err(|_| unexpected())?;
                    (sha, false)
                }
                // GitHub gives up comparing a very large PR (a server
                // error or a timeout) while its files still list. The
                // base branch commit the PR reports is the closest left
                // side then (the merge base itself while the branch is up
                // to date with its base), flagged: once the base branch
                // moved on, its old side does not line up with the
                // patches. The core asks a local clone for the real one.
                Err(e @ CoreError::Provider(_)) if !super::is_rate_limited(&e) => {
                    tracing::debug!("no merge base for a large PR; using its base commit");
                    (base.clone(), true)
                }
                Err(e) => return Err(e),
            };
            let after: GhPull = self.api(&pull_path)?;
            if after.head.sha != head {
                continue;
            }
            return Ok(FileDiff {
                base_sha: merge_base,
                head_sha: head,
                truncated: files.len() >= PULL_FILES_LIMIT,
                files: files.into_iter().map(changed_file).collect(),
                base_approximate,
            });
        }
        Err(CoreError::Provider(
            "the pull request got new commits while loading its diff; try again".into(),
        ))
    }
}

fn unexpected() -> CoreError {
    CoreError::Provider("unexpected response from gh".into())
}

impl From<ExecError> for CoreError {
    fn from(e: ExecError) -> Self {
        match e {
            ExecError::Missing => CoreError::GithubUnavailable,
            ExecError::Failed => CoreError::Provider("could not run gh".into()),
            ExecError::TimedOut => CoreError::Provider("gh did not answer in time".into()),
        }
    }
}

/// One REST page: its body and the query of the next page, if any.
struct Page {
    body: String,
    next: Option<String>,
}

/// A response as `gh api -i` prints it: `HTTP/2.0 200 OK`, the headers,
/// a blank line, the body.
struct HttpReply {
    status: u16,
    /// Lowercased names.
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpReply {
    fn parse(out: &str) -> Option<Self> {
        let (first, mut rest) = out.split_once('\n')?;
        let first = first.trim_end_matches('\r');
        if !first.starts_with("HTTP/") {
            return None;
        }
        let status = first.split_whitespace().nth(1)?.parse().ok()?;
        let mut headers = Vec::new();
        while !rest.is_empty() {
            let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
            rest = after;
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
            }
        }
        Some(Self {
            status,
            headers,
            body: rest.to_string(),
        })
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    /// GitHub's primary limit (403/429 with no requests left), or a
    /// secondary one (a `Retry-After`, or the message says so).
    fn is_rate_limited(&self) -> bool {
        self.status == 429
            || self.header("x-ratelimit-remaining").as_deref() == Some("0")
            || self.header("retry-after").is_some()
            || self.body.to_lowercase().contains("rate limit")
    }
}

/// The query of the `rel="next"` URL in a `Link` header, when it is plain
/// enough to pass on to `gh` (`per_page=100&page=2`).
fn next_query(link: &str) -> Option<String> {
    let part = link.split(',').find(|p| p.contains("rel=\"next\""))?;
    let url = part.split_once('<')?.1.split_once('>')?.0;
    let query = url.split_once('?')?.1;
    query
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"=&_-.%+,:".contains(&b))
        .then(|| query.to_string())
}

/// The `page=` number of a next-page query.
fn page_number(query: &str) -> Option<usize> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("page="))
        .and_then(|n| n.parse().ok())
}

/// The REST id in a review comment URL (`…/pull/12#discussion_r501`).
fn root_comment_id(url: &str) -> Option<String> {
    let id = url.rsplit_once("#discussion_r")?.1;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// A review thread's state from GraphQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadState {
    resolved: bool,
    outdated: bool,
}

/// Lay GraphQL's thread states over the REST threads. Outdated only ever
/// turns on (REST's `position == null` already marks most), and then the
/// thread has no current line.
fn apply_thread_states(threads: &mut [Thread], states: &HashMap<String, ThreadState>) {
    for t in threads {
        let Some(s) = states.get(&t.id) else { continue };
        t.resolved = s.resolved;
        if s.outdated && !t.outdated {
            t.outdated = true;
            if let Some(a) = t.anchor.as_mut() {
                a.current_line = None;
            }
        }
    }
}

pub(crate) fn parse_pages<T: DeserializeOwned>(out: &str) -> CoreResult<Vec<T>> {
    let mut all = Vec::new();
    for page in serde_json::Deserializer::from_str(out).into_iter::<Vec<T>>() {
        all.extend(page.map_err(|_| unexpected())?);
    }
    Ok(all)
}

fn same_user(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl ReviewProvider for GithubProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn current_user(&self) -> CoreResult<String> {
        self.me()
    }

    /// Three searches, merged and deduped: review requested, reviewed by me,
    /// commented by me. `--reviewed-by` is on top of the plan's two: GitHub
    /// drops the user from `review-requested` once they submit a review, and
    /// `commenter` does not cover a review without a conversation comment.
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        let me = self.me()?;
        let (requested, reviewed, commented) = std::thread::scope(|s| {
            let a = s.spawn(|| self.search("--review-requested=@me"));
            let b = s.spawn(|| self.search("--reviewed-by=@me"));
            let c = s.spawn(|| self.search("--commenter=@me"));
            (join(a), join(b), join(c))
        });
        let mut seen: BTreeMap<(String, u64), ReviewRef> = BTreeMap::new();
        for item in requested?.into_iter().chain(reviewed?).chain(commented?) {
            let author = item.author.map(|a| a.login).unwrap_or_default();
            if same_user(&author, &me) {
                continue; // own PRs are out of scope
            }
            let repo = item.repository.name_with_owner;
            seen.entry((repo.clone(), item.number))
                .or_insert_with(|| ReviewRef {
                    provider: ProviderId::Github,
                    host: self.host.clone(),
                    repo,
                    number: item.number,
                    url: item.url,
                    title: item.title,
                    author,
                    updated_at: item.updated_at,
                });
        }
        let mut list: Vec<ReviewRef> = seen.into_values().collect();
        list.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(list)
    }

    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
        let me = self.me()?;
        let base = format!("repos/{}/pulls/{}", r.repo, r.number);
        let (pull, commits, reviews) = std::thread::scope(|s| {
            let a = s.spawn(|| self.api::<GhPull>(&base));
            let b = s.spawn(|| self.api_list::<GhCommit>(&format!("{base}/commits")));
            let c = s.spawn(|| self.api_list::<GhReview>(&format!("{base}/reviews")));
            (join(a), join(b), join(c))
        });
        Ok(assemble_detail(r, &me, pull?, commits?, reviews?))
    }

    /// The PR and its commits: not its reviews.
    fn fetch_head(&self, r: &ReviewRef) -> CoreResult<ReviewHead> {
        let base = format!("repos/{}/pulls/{}", r.repo, r.number);
        let (pull, commits) = std::thread::scope(|s| {
            let a = s.spawn(|| self.api::<GhPull>(&base));
            let b = s.spawn(|| self.api_list::<GhCommit>(&format!("{base}/commits")));
            (join(a), join(b))
        });
        let pull = pull?;
        Ok(ReviewHead {
            head_sha: pull.head.sha,
            base_sha: pull.base.sha,
            commits: commit_infos(commits?),
        })
    }

    /// The REST review comments, regrouped into threads, with GraphQL's
    /// resolved state laid over them (fetched in parallel; on failure the
    /// threads just stay unresolved).
    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
        let (comments, states) = std::thread::scope(|s| {
            let a = s.spawn(|| {
                self.api_list::<GhComment>(&format!("repos/{}/pulls/{}/comments", r.repo, r.number))
            });
            let b = s.spawn(|| self.thread_states(r));
            (join(a), b.join().ok().flatten())
        });
        let mut threads = group_threads(comments?);
        if let Some(states) = states {
            apply_thread_states(&mut threads, &states);
        }
        Ok(threads)
    }

    fn file_at(&self, r: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        check_sha(sha)?;
        let file: GhContent = self.api(&format!(
            "repos/{}/contents/{}?ref={sha}",
            r.repo,
            encode_path(path)
        ))?;
        decode_content(file)
    }

    fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        match range {
            DiffRange::Review { base, head } => {
                check_sha(base)?;
                check_sha(head)?;
                self.review_diff(r)
            }
            DiffRange::Compare { from, to } => {
                check_sha(from)?;
                check_sha(to)?;
                let cmp: GhCompare =
                    self.api(&format!("repos/{}/compare/{from}...{to}", r.repo))?;
                Ok(FileDiff {
                    base_sha: cmp.merge_base_commit.sha,
                    head_sha: to.clone(),
                    truncated: cmp.files.len() >= COMPARE_FILES_LIMIT,
                    files: cmp.files.into_iter().map(changed_file).collect(),
                    base_approximate: false,
                })
            }
        }
    }
    // --- M4 writes (helpers in the "M4: writes" section at the end) ---

    fn post_comment(&self, r: &ReviewRef, c: &PlacedComment) -> PostResultOf<PostedComment> {
        let posted: GhPosted = self.post_json(
            &format!("repos/{}/pulls/{}/comments", r.repo, r.number),
            &comment_body(c),
        )?;
        Ok(PostedComment {
            url: posted.html_url.unwrap_or_else(|| r.url.clone()),
            provider_thread_id: posted.id.to_string(),
            provider_note_id: posted.id.to_string(),
            path: c.path.clone(),
            side: c.side,
            line: c.line_number(),
            start_line: c.start_number(),
            head_sha: c.head_sha.clone(),
        })
    }

    fn reply(&self, r: &ReviewRef, thread: &str, body: &str) -> PostResultOf<PostedNote> {
        let root: u64 = thread
            .parse()
            .map_err(|_| PostError::new(PostErrorKind::InvalidInput, "not a GitHub thread"))?;
        let posted: GhPosted = self.post_json(
            &format!(
                "repos/{}/pulls/{}/comments/{root}/replies",
                r.repo, r.number
            ),
            &serde_json::json!({ "body": body }),
        )?;
        Ok(PostedNote {
            url: posted.html_url.unwrap_or_else(|| r.url.clone()),
            provider_thread_id: root.to_string(),
            provider_note_id: posted.id.to_string(),
        })
    }

    /// GitHub accepts an approval of an older commit (it shows as approving
    /// an outdated head), so the core checks the head before calling this;
    /// `commit_id` records which commit was approved.
    fn approve(&self, r: &ReviewRef, head_sha: &str) -> PostResultOf<()> {
        let _: serde_json::Value = self.post_json(
            &format!("repos/{}/pulls/{}/reviews", r.repo, r.number),
            &serde_json::json!({ "event": "APPROVE", "commit_id": head_sha }),
        )?;
        Ok(())
    }
    // --- M3: thread history ---

    /// `commits/{sha}` pages its `files` (one object per page); the first
    /// parent comes with the first page.
    fn commit_diff(&self, r: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        check_sha(sha)?;
        let pages = self.pages(&format!("repos/{}/commits/{sha}", r.repo))?;
        let mut parent: Option<String> = None;
        let mut files = Vec::new();
        for page in &pages {
            let page: GhCommitDiff = serde_json::from_str(page).map_err(|_| unexpected())?;
            if parent.is_none() {
                parent = Some(
                    page.parents
                        .into_iter()
                        .next()
                        .map(|p| p.sha)
                        .unwrap_or_default(),
                );
            }
            files.extend(page.files);
        }
        Ok(FileDiff {
            base_sha: parent.unwrap_or_default(),
            head_sha: sha.to_string(),
            truncated: files.len() >= PULL_FILES_LIMIT,
            files: files.into_iter().map(changed_file).collect(),
            base_approximate: false,
        })
    }

    fn rate_limited_until(&self) -> Option<DateTime<Utc>> {
        self.limit.active()
    }
}

/// One entry of `pulls/{n}/files` or a compare's `files`. GitHub leaves the
/// patch out of binary files and of diffs too large to send; `changes` tells
/// the two apart (a binary file counts no lines).
fn changed_file(f: GhFile) -> ChangedFile {
    let status = match f.status.as_str() {
        "added" => FileStatus::Added,
        "removed" => FileStatus::Removed,
        "renamed" => FileStatus::Renamed,
        // modified, changed (mode only), copied, unchanged
        _ => FileStatus::Modified,
    };
    let no_patch = f.patch.is_none();
    // Only a "modified" file with no patch and no lines is a binary by
    // that alone: "changed" (a mode change), "copied" and "unchanged" also
    // map to Modified and send no patch, but have no content change.
    let binary =
        no_patch && f.changes == 0 && (f.status == "modified" || looks_binary(&f.filename));
    ChangedFile {
        old_path: f
            .previous_filename
            .filter(|_| status == FileStatus::Renamed),
        status,
        additions: f.additions,
        deletions: f.deletions,
        too_large: no_patch && f.changes > 0,
        binary,
        patch: f.patch,
        // A missing sha (submodules) still needs a key; the path does.
        content_sha: f.sha.unwrap_or_else(|| f.filename.clone()),
        path: f.filename,
    }
}

fn join<T>(h: std::thread::ScopedJoinHandle<'_, CoreResult<T>>) -> CoreResult<T> {
    h.join()
        .unwrap_or_else(|_| Err(CoreError::Provider("a GitHub fetch panicked".into())))
}

pub(crate) fn check_sha(sha: &str) -> CoreResult<()> {
    if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(CoreError::InvalidInput("not a commit sha".into()));
    }
    Ok(())
}

fn decode_content(file: GhContent) -> CoreResult<String> {
    if file.encoding.as_deref() != Some("base64") {
        // GitHub sends `encoding: none` and no content above 1 MB.
        return Err(CoreError::Provider("the file is too large to fetch".into()));
    }
    let raw: String = file
        .content
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| unexpected())?;
    String::from_utf8(bytes).map_err(|_| CoreError::Provider("the file is not text".into()))
}

/// GitHub's commits (oldest first, as listed) as the model's.
fn commit_infos(commits: Vec<GhCommit>) -> Vec<CommitInfo> {
    commits
        .into_iter()
        .map(|c| CommitInfo {
            title: first_line(&c.commit.message),
            author: c
                .author
                .map(|a| a.login)
                .or_else(|| c.commit.author.as_ref().map(|a| a.name.clone()))
                .unwrap_or_default(),
            authored_at: c.commit.author.and_then(|a| a.date),
            sha: c.sha,
        })
        .collect()
}

fn assemble_detail(
    r: &ReviewRef,
    me: &str,
    pull: GhPull,
    commits: Vec<GhCommit>,
    reviews: Vec<GhReview>,
) -> ReviewDetail {
    let author = pull.user.map(|u| u.login).unwrap_or_default();
    let requested: Vec<String> = pull
        .requested_reviewers
        .into_iter()
        .map(|u| u.login)
        .collect();

    // Latest standing state per reviewer, in submission order. A plain
    // comment does not override an approval or a change request (GitHub's
    // own rule); the author's replies to review comments are reviews too and
    // are skipped.
    let mut standing: Vec<(String, ReviewerState)> = Vec::new();
    for review in reviews {
        let Some(user) = review.user.map(|u| u.login) else {
            continue;
        };
        if same_user(&user, &author) {
            continue;
        }
        let state = match review.state.as_str() {
            "APPROVED" => ReviewerState::Approved,
            "CHANGES_REQUESTED" => ReviewerState::ChangesRequested,
            "DISMISSED" => ReviewerState::Dismissed,
            "COMMENTED" => ReviewerState::Commented,
            _ => continue, // PENDING: not submitted
        };
        match standing.iter_mut().find(|(u, _)| same_user(u, &user)) {
            Some((_, current)) => {
                if state != ReviewerState::Commented || *current == ReviewerState::Dismissed {
                    *current = state;
                }
            }
            None => standing.push((user, state)),
        }
    }
    let my_state = standing
        .iter()
        .find(|(u, _)| same_user(u, me))
        .map(|(_, s)| *s);
    let requested_from_me = requested.iter().any(|u| same_user(u, me));

    let mut reviewers: Vec<Reviewer> = requested
        .iter()
        .map(|u| Reviewer {
            username: u.clone(),
            state: ReviewerState::Requested,
            is_me: same_user(u, me),
        })
        .collect();
    for (user, state) in standing {
        if !requested.iter().any(|u| same_user(u, &user)) {
            reviewers.push(Reviewer {
                is_me: same_user(&user, me),
                username: user,
                state,
            });
        }
    }

    let state = if pull.merged || pull.merged_at.is_some() {
        ReviewState::Merged
    } else if pull.state == "closed" {
        ReviewState::Closed
    } else {
        ReviewState::Open
    };

    ReviewDetail {
        review: ReviewRef {
            title: pull.title,
            author,
            url: pull.html_url.unwrap_or_else(|| r.url.clone()),
            updated_at: pull.updated_at.or(r.updated_at),
            ..r.clone()
        },
        description: pull.body.unwrap_or_default(),
        state,
        draft: pull.draft,
        head_sha: pull.head.sha,
        base_sha: pull.base.sha,
        start_sha: None,
        commits: commit_infos(commits),
        versions: Vec::new(),
        reviewers,
        requested_from_me,
        // Approved at the current or any earlier head counts; a re-request
        // since means the author wants another look.
        approved_by_me: my_state == Some(ReviewerState::Approved) && !requested_from_me,
    }
}

/// Rebuild threads from flat review comments: a reply's `in_reply_to_id`
/// names the root (followed transitively, in case a reply points at a
/// reply). A root with `position == null` is what GitHub calls outdated.
fn group_threads(mut comments: Vec<GhComment>) -> Vec<Thread> {
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let parent: HashMap<u64, Option<u64>> =
        comments.iter().map(|c| (c.id, c.in_reply_to_id)).collect();
    let root_of = |mut id: u64| {
        for _ in 0..64 {
            match parent.get(&id) {
                Some(Some(p)) if parent.contains_key(p) => id = *p,
                _ => break,
            }
        }
        id
    };

    let mut threads: Vec<Thread> = Vec::new();
    let mut index: HashMap<u64, usize> = HashMap::new();
    // Roots first so a reply always finds its thread.
    let (roots, replies): (Vec<_>, Vec<_>) = comments
        .into_iter()
        .partition(|c| c.in_reply_to_id.is_none_or(|p| !parent.contains_key(&p)));
    for c in roots {
        index.insert(c.id, threads.len());
        threads.push(root_thread(c));
    }
    for c in replies {
        if let Some(&i) = index.get(&root_of(c.id)) {
            threads[i].notes.push(note(&c));
        }
    }
    for t in &mut threads {
        t.notes.sort_by_key(|n| n.created_at);
    }
    threads
}

fn note(c: &GhComment) -> Note {
    Note {
        id: c.id.to_string(),
        author: login(&c.user),
        body: c.body.clone(),
        created_at: c.created_at,
        system: false,
    }
}

fn login(u: &Option<GhUser>) -> String {
    u.as_ref()
        .map(|u| u.login.clone())
        // GitHub's placeholder for a deleted account.
        .unwrap_or_else(|| "ghost".to_string())
}

fn root_thread(c: GhComment) -> Thread {
    let side = match c.side.as_deref() {
        Some("LEFT") => Side::Old,
        _ => Side::New,
    };
    let line = c.original_line.or(c.line);
    let (line_old, line_new) = match side {
        Side::Old => (line, None),
        Side::New => (None, line),
    };
    let anchor = Anchor {
        path_old: Some(c.path.clone()),
        path_new: Some(c.path.clone()),
        line_old,
        line_new,
        start_line: c.original_start_line.or(c.start_line),
        side,
        base_sha: None,
        start_sha: None,
        head_sha: c
            .original_commit_id
            .clone()
            .or_else(|| c.commit_id.clone())
            .unwrap_or_default(),
        diff_hunk: c.diff_hunk.clone(),
        // Outdated comments have no current line.
        current_line: c.position.and(c.line),
    };
    Thread {
        id: c.id.to_string(),
        author: login(&c.user),
        created_at: c.created_at,
        resolved: false,
        outdated: c.position.is_none(),
        anchor: Some(anchor),
        notes: vec![note(&c)],
    }
}

#[derive(Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhSearchItem {
    number: u64,
    title: String,
    url: String,
    repository: GhRepository,
    author: Option<GhUser>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepository {
    name_with_owner: String,
}

#[derive(Deserialize)]
struct GhRef {
    sha: String,
}

#[derive(Deserialize)]
struct GhPull {
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    #[serde(default)]
    merged: bool,
    #[serde(default)]
    merged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    draft: bool,
    user: Option<GhUser>,
    html_url: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    head: GhRef,
    base: GhRef,
    #[serde(default)]
    requested_reviewers: Vec<GhUser>,
}

#[derive(Deserialize)]
struct GhCommit {
    sha: String,
    commit: GhCommitBody,
    author: Option<GhUser>,
}

#[derive(Deserialize)]
struct GhCommitBody {
    message: String,
    author: Option<GhCommitAuthor>,
}

#[derive(Deserialize)]
struct GhCommitAuthor {
    name: String,
    date: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct GhReview {
    user: Option<GhUser>,
    state: String,
}

#[derive(Deserialize)]
struct GhComment {
    id: u64,
    in_reply_to_id: Option<u64>,
    user: Option<GhUser>,
    body: String,
    created_at: DateTime<Utc>,
    path: String,
    position: Option<u64>,
    line: Option<u32>,
    original_line: Option<u32>,
    start_line: Option<u32>,
    original_start_line: Option<u32>,
    side: Option<String>,
    commit_id: Option<String>,
    original_commit_id: Option<String>,
    diff_hunk: Option<String>,
}

#[derive(Deserialize)]
struct GhFile {
    filename: String,
    status: String,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    additions: u32,
    #[serde(default)]
    deletions: u32,
    #[serde(default)]
    changes: u32,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    previous_filename: Option<String>,
}

#[derive(Deserialize)]
struct GhCompare {
    merge_base_commit: GhRef,
    #[serde(default)]
    files: Vec<GhFile>,
}

#[derive(Deserialize)]
struct GhContent {
    content: Option<String>,
    encoding: Option<String>,
}

// ---------------------------------------------------------------------------
// M4: writes. Only `Core`'s post API reaches these, on an explicit user
// action. The JSON body goes to `gh api --input -` on stdin, never into an
// argument or a shell string.
// ---------------------------------------------------------------------------

impl GithubProvider {
    /// `gh api --method POST <path> --input -` with `body` on stdin; the
    /// response parsed as `T`.
    fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> PostResultOf<T> {
        let token = self.token()?;
        let output = match self.exec(
            token.as_deref(),
            &["api", "--method", "POST", path, "--input", "-"],
            Some(body.to_string().into_bytes()),
        ) {
            Ok(o) => o,
            // It may have reached GitHub before it hung.
            Err(ExecError::TimedOut) => {
                return Err(PostError::new(
                    PostErrorKind::Provider,
                    "GitHub did not answer in time; refresh to see whether it went through",
                ))
            }
            Err(e) => return Err(CoreError::from(e).into()),
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() {
            return serde_json::from_str(&stdout).map_err(|_| unexpected().into());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        tracing::debug!("gh api POST failed");
        if stderr.contains("auth login")
            || stderr.contains("not logged")
            || stderr.contains("http 401")
        {
            *self.token.lock().expect("token lock poisoned") = None;
            return Err(CoreError::GithubUnavailable.into());
        }
        if stderr.contains("rate limit") {
            return Err(
                CoreError::Provider("GitHub rate limit reached, try again later".into()).into(),
            );
        }
        // gh prints GitHub's JSON error body on stdout: its message is what
        // the user needs ("line must be part of the diff").
        let message = github_error_message(&stdout).map(|m| {
            format!(
                "GitHub: {}",
                sanitize(&m, &[token.as_deref().unwrap_or("")])
            )
        });
        Err(match message {
            Some(m) if stderr.contains("http 404") => PostError::new(PostErrorKind::NotFound, m),
            Some(m) => PostError::new(PostErrorKind::PostRejected, m),
            None if stderr.contains("http 404") => CoreError::NotFound.into(),
            None => CoreError::Provider("gh could not post to GitHub".into()).into(),
        })
    }
}

/// The JSON body of a new review comment.
fn comment_body(c: &PlacedComment) -> serde_json::Value {
    let side = match c.side {
        Side::Old => "LEFT",
        Side::New => "RIGHT",
    };
    let mut body = serde_json::json!({
        "body": c.body,
        "commit_id": c.head_sha,
        "path": c.path,
        "line": c.line_number(),
        "side": side,
    });
    if let Some(start) = c.start_number() {
        body["start_line"] = start.into();
        body["start_side"] = side.into();
    }
    body
}

/// GitHub's error text: `message`, plus each of `errors` (an object's
/// `message`, else its `field` and `code`; or a plain string).
fn github_error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
        parts.push(m.to_string());
    }
    for e in v
        .get("errors")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
    {
        let text = match e {
            serde_json::Value::String(s) => Some(s.clone()),
            _ => e
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .or_else(|| {
                    let field = e.get("field").and_then(|f| f.as_str())?;
                    let code = e.get("code").and_then(|c| c.as_str()).unwrap_or("invalid");
                    Some(format!("{field} {code}"))
                }),
        };
        parts.extend(text);
    }
    (!parts.is_empty()).then(|| parts.join(": "))
}

#[derive(Deserialize)]
struct GhPosted {
    id: u64,
    html_url: Option<String>,
}

// GraphQL review thread states (M6).

#[derive(Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
}

#[derive(Deserialize)]
struct GqlData {
    repository: Option<GqlRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRepository {
    pull_request: Option<GqlPull>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPull {
    review_threads: GqlThreads,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlThreads {
    page_info: GqlPageInfo,
    #[serde(default)]
    nodes: Vec<GqlThread>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPageInfo {
    #[serde(default)]
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlThread {
    #[serde(default)]
    is_resolved: bool,
    #[serde(default)]
    is_outdated: bool,
    comments: GqlComments,
}

#[derive(Deserialize)]
struct GqlComments {
    #[serde(default)]
    nodes: Vec<GqlComment>,
}

#[derive(Deserialize)]
struct GqlComment {
    url: String,
}

/// One page of `commits/{sha}` (M3).
#[derive(Deserialize)]
struct GhCommitDiff {
    #[serde(default)]
    parents: Vec<GhRef>,
    #[serde(default)]
    files: Vec<GhFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mode_only_change_is_not_a_binary() {
        let file = |status: &str, name: &str| GhFile {
            filename: name.into(),
            status: status.into(),
            sha: Some("f1".repeat(20)),
            additions: 0,
            deletions: 0,
            changes: 0,
            patch: None,
            previous_filename: None,
        };
        // chmod +x: status "changed", no patch, no lines.
        let chmod = changed_file(file("changed", "scripts/run.sh"));
        assert!(!chmod.binary && !chmod.too_large);
        assert_eq!(chmod.status, FileStatus::Modified);
        // A pure rename or copy is no binary either.
        assert!(!changed_file(file("copied", "src/a.rs")).binary);
        // A modified file without a patch or lines is.
        assert!(changed_file(file("modified", "assets/blob")).binary);
        // A binary type whatever the status.
        assert!(changed_file(file("changed", "assets/logo.png")).binary);
    }

    #[test]
    fn parses_concatenated_pages() {
        let v: Vec<u32> = parse_pages("[1,2]\n[3]").unwrap();
        assert_eq!(v, [1, 2, 3]);
        let v: Vec<u32> = parse_pages("[]").unwrap();
        assert!(v.is_empty());
        assert!(parse_pages::<u32>("{oops").is_err());
    }

    #[test]
    fn parses_gh_included_output() {
        let out = "HTTP/2.0 200 OK\nContent-Type: application/json\r\nEtag: W/\"abc\"\r\n\
                   Link: <https://api.github.com/repositories/1/pulls/12/comments?per_page=100&page=2>; rel=\"next\", \
                   <https://api.github.com/repositories/1/pulls/12/comments?per_page=100&page=3>; rel=\"last\"\r\n\r\n[1,2]";
        let r = HttpReply::parse(out).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.header("ETag").as_deref(), Some("W/\"abc\""));
        assert_eq!(r.body, "[1,2]");
        assert_eq!(
            next_query(&r.header("link").unwrap()).as_deref(),
            Some("per_page=100&page=2")
        );
        let not_modified = HttpReply::parse("HTTP/2.0 304 Not Modified\nEtag: x\r\n\r\n").unwrap();
        assert_eq!((not_modified.status, not_modified.body.as_str()), (304, ""));
        assert!(HttpReply::parse("[1,2]").is_none());
        assert!(HttpReply::parse("").is_none());
        assert_eq!(next_query("<https://x/y?a=1>; rel=\"last\""), None);
        assert_eq!(next_query("<https://x/y?a=$(rm)>; rel=\"next\""), None);
    }

    #[test]
    fn spots_rate_limits() {
        let reply = |status, headers: &[(&str, &str)], body: &str| HttpReply {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.into(),
        };
        assert!(reply(403, &[("x-ratelimit-remaining", "0")], "").is_rate_limited());
        assert!(reply(403, &[("retry-after", "30")], "").is_rate_limited());
        assert!(reply(429, &[], "").is_rate_limited());
        assert!(reply(
            403,
            &[],
            "{\"message\":\"You have exceeded a secondary rate limit\"}"
        )
        .is_rate_limited());
        assert!(!reply(
            403,
            &[("x-ratelimit-remaining", "4999")],
            "{\"message\":\"Forbidden\"}"
        )
        .is_rate_limited());
    }

    #[test]
    fn maps_review_comment_urls_to_root_ids() {
        assert_eq!(
            root_comment_id("https://github.com/o/r/pull/12#discussion_r2200000001").as_deref(),
            Some("2200000001")
        );
        assert_eq!(root_comment_id("https://github.com/o/r/pull/12"), None);
        assert_eq!(
            root_comment_id("https://github.com/o/r/pull/12#discussion_rx"),
            None
        );
    }

    #[test]
    fn rejects_non_sha() {
        assert!(check_sha("abc123").is_ok());
        assert!(check_sha("--help").is_err());
        assert!(check_sha("").is_err());
    }
}
