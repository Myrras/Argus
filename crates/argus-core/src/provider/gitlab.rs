//! GitLab, through the user's `glab` CLI, the way GitHub goes through `gh`:
//! Argus keeps no GitLab token. glab authenticates every call with its own
//! login for the host (`glab auth login --hostname <host>`); Argus never
//! reads, stores or logs that token, and never puts one in glab's
//! environment (the user's environment passes through, as for `gh`).
//!
//! Every REST call is `glab api --hostname <host> --include --method <M>
//! <path>` (a body as JSON on stdin through `--input -`); GraphQL is the
//! same with `graphql` as the path. No shell: the arguments go to glab as
//! they are. glab prints the status line, the headers, a blank line and the
//! body for any status (it exits 1 for a non-2xx, a 304 included, and for a
//! GraphQL answer with `errors`), so the status line decides, as with
//! `gh api -i`. Without one glab never got a response (not installed,
//! offline): its stderr then only classifies the failure.
//!
//! What glab prints is GitLab's and glab's text. Only GitLab's own error
//! message is ever forwarded, and only after [`redact_tokens`]: Argus does
//! not know glab's token, so it removes anything shaped like one.
//!
//! Every GET is conditional: the last `ETag` (or `Last-Modified`) of the
//! path goes out as `If-None-Match` (`If-Modified-Since`) through
//! `--header`, and a 304 replays the cached body and `X-Next-Page`. A 429
//! stops all calls until GitLab's `Retry-After` / `RateLimit-Reset`. Every
//! glab call is killed after a timeout. Redirects are glab's to follow.
//!
//! glab's logins (`glab auth status`) are the accounts module's to ask
//! ([`crate::accounts`]); one provider talks to one host.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::conditional::{Cached, ConditionalCache};
use super::github::check_sha;
use super::http::HttpReply;
use super::model::{
    Anchor, ChangedFile, CommitInfo, DiffRange, DiffVersion, FileDiff, FileStatus, MyReviewState,
    Note, ProviderId, ReviewDetail, ReviewHead, ReviewRef, ReviewState, ReviewVerdict, Reviewer,
    ReviewerState, Side, Thread,
};
use super::rate_limit::{reset_at, Backoff};
use super::{
    count_patch, digest, encode_segment, first_line, looks_binary, PostResultOf, ReviewProvider,
};
use crate::error::{CoreError, CoreResult};
use crate::history::ThreadResolution;
use crate::platform::{run_with_timeout, ExecError};
use crate::post::{
    sanitize, DiffLine, LineKind, OutsideDiff, PlacedComment, PostError, PostErrorKind,
    PostedComment, PostedNote,
};

/// Page size for every list call, and how many pages are followed at most.
const PER_PAGE: u32 = 100;
const MAX_PAGES: u32 = 20;
/// Pages of discussions followed at most: every thread counts for rounds
/// and the untouched count, so far past any real MR (10 000) rather than a
/// silent cut at 2000.
const DISCUSSION_PAGES: u32 = 100;

/// How many commented-on MRs the list fetches at the same time.
const LIST_PARALLELISM: usize = 4;

/// How long a commented-on MR seen merged or closed is left out of the
/// list before it is read again (it may have been reopened).
const FINAL_RECHECK: chrono::Duration = chrono::Duration::days(1);

/// How many polls in a row a commented-on MR whose read fails stays listed
/// as last seen: a blip costs it nothing, a read that keeps failing (a
/// permanent error on that MR) drops it instead of keeping it for as long
/// as its comment event stays in the feed.
const COMMENTED_FAILURES_KEPT: u32 = 3;

/// How far back the user's own comment events are read to find MRs they
/// commented on.
const COMMENTED_DAYS: i64 = 30;

/// How long one glab call may run before it is killed: one request, start
/// to end of body (glab itself has no overall timeout).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The waits before each try of a request whose connection could not be
/// made (VPN or Wi-Fi coming back after sleep), as Podium retries.
const CONNECT_RETRIES: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(500),
    Duration::from_millis(1500),
];

/// `https://host[:port]/path` → `host[:port]`.
pub fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or("")
}

/// Trim, parse and drop a trailing slash; `None` when not an http(s) URL
/// of an instance. A bare host (`gitlab.example.com`, as glab names hosts)
/// is taken as `https://` that host. The URL only names the instance: its
/// host is what glab is asked for (`--hostname`), its scheme and path are
/// for web links. How glab talks to the host (https or http, its API host)
/// is glab's own login setting.
///
/// The host is the parsed one: in `http://127.0.0.1:80@gitlab.example.test`
/// the part before `@` is userinfo, not the host. Userinfo, a query or a
/// fragment have no place in an instance URL and are refused; the URL comes
/// back in its parsed form, so [`url_host`] reads the same host.
pub fn normalize_base_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let with_scheme = if url.contains("://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    let parsed = url::Url::parse(&with_scheme).ok()?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    parsed.host_str().filter(|h| !h.is_empty())?;
    matches!(parsed.scheme(), "https" | "http")
        .then(|| parsed.as_str().trim_end_matches('/').to_string())
}

/// Token prefixes GitLab gives its tokens (personal, OAuth app, deploy,
/// runner, CI build, project/group, feed, incoming mail, agent, SCIM,
/// feature flag, workspace, test). glab's OAuth login holds a plain
/// 64-hex token instead, which [`redact_tokens`] catches by its shape.
const TOKEN_PREFIXES: &[&str] = &[
    "glpat-",
    "gloas-",
    "gldt-",
    "glrt-",
    "glrtr-",
    "glcbt-",
    "glptt-",
    "glft-",
    "glimt-",
    "glagent-",
    "glsoat-",
    "glffct-",
    "glwt-",
    "gltst-",
    "glsa-",
    "GR1348941",
];

/// Words after which the next run of token characters (past any spaces) is
/// a credential.
const TOKEN_CONTEXTS: &[&str] = &[
    "bearer ",
    "private-token:",
    "job-token:",
    "private_token=",
    "access_token=",
    "token=",
];

/// A character a token is made of (GitLab's tokens are URL-safe base64 or
/// hex, the routable ones with dots).
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

/// `text` with everything that looks like a GitLab credential replaced by
/// `[redacted]`: a run of token characters that starts with one of
/// [`TOKEN_PREFIXES`], follows one of [`TOKEN_CONTEXTS`] (any case), or is
/// 64 or more hex digits (an OAuth token; a commit sha is 40). Argus never
/// knows glab's token, so it cannot redact it by value as it does gh's: any
/// text of GitLab's or glab's goes through this before it can become a
/// message.
pub(crate) fn redact_tokens(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_ascii_lowercase()).collect();
    let starts_with = |i: usize, word: &str, folded: bool| {
        let src = if folded { &lower } else { &chars };
        let w: Vec<char> = word.chars().collect();
        i + w.len() <= src.len() && src[i..i + w.len()] == w[..]
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // A credential after a context word: keep the word, drop the run.
        if let Some(word) = TOKEN_CONTEXTS.iter().find(|w| starts_with(i, w, true)) {
            let n = word.chars().count();
            out.extend(&chars[i..i + n]);
            i += n;
            while i < chars.len() && chars[i] == ' ' {
                out.push(' ');
                i += 1;
            }
            let end = (i..chars.len())
                .find(|&j| !token_char(chars[j]))
                .unwrap_or(chars.len());
            if end > i {
                out.push_str("[redacted]");
                i = end;
            }
            continue;
        }
        if !token_char(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = (i..chars.len())
            .find(|&j| !token_char(chars[j]))
            .unwrap_or(chars.len());
        let run: String = chars[i..end].iter().collect();
        let hex = run.len() >= 64 && run.chars().all(|c| c.is_ascii_hexdigit());
        // A prefix anywhere in the run (`x=glpat-…` splits at `=`, but
        // `ab-glpat-…` does not): from there on it is the token.
        let at = (i..end).find(|&j| TOKEN_PREFIXES.iter().any(|p| starts_with(j, p, false)));
        match at {
            _ if hex => out.push_str("[redacted]"),
            Some(j) => {
                out.extend(&chars[i..j]);
                out.push_str("[redacted]");
            }
            None => out.push_str(&run),
        }
        i = end;
    }
    out
}

/// GitLab's or glab's text made fit for a message: tokens redacted, then
/// one line, capped ([`sanitize`]).
fn clean(text: &str) -> String {
    sanitize(&redact_tokens(text), &[])
}

/// Why a glab call printed no response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// glab is not installed (or not where Argus looked).
    Missing,
    /// glab could not be started.
    Spawn,
    /// Killed after the timeout: it may have reached GitLab.
    TimedOut,
    /// No connection could be made (`dial tcp …`): nothing was sent.
    NotConnected,
    /// The connection broke after it was made (TLS, a reset): a write may
    /// have gone through.
    Broken,
    /// glab has no usable login for the host.
    Auth,
    /// Anything else glab said.
    Other,
}

impl Failure {
    /// Classify glab's stderr (never forwarded).
    fn from_stderr(stderr: &[u8]) -> Self {
        let text = String::from_utf8_lossy(stderr).to_lowercase();
        if text.contains("has not been authenticated")
            || text.contains("auth login")
            || text.contains("http 401")
            || text.contains("401 unauthorized")
        {
            Failure::Auth
        } else if text.contains("dial tcp") || text.contains("no such host") {
            // Go's dialer: the request never left this machine.
            Failure::NotConnected
        } else if text.contains("connection reset")
            || text.contains("eof")
            || text.contains("tls")
            || text.contains("timeout")
            || text.contains("x509")
        {
            Failure::Broken
        } else {
            Failure::Other
        }
    }

    fn into_error(self, host: &str) -> CoreError {
        match self {
            Failure::Missing => CoreError::GlabMissing,
            Failure::Spawn => CoreError::Provider("could not run glab".into()),
            Failure::TimedOut => CoreError::Provider("could not reach GitLab (timed out)".into()),
            Failure::NotConnected | Failure::Broken => {
                CoreError::Provider("could not reach GitLab (connection failed)".into())
            }
            Failure::Auth => CoreError::GitlabAuth {
                host: host.to_string(),
            },
            Failure::Other => CoreError::Provider("glab could not reach GitLab".into()),
        }
    }
}

impl From<ExecError> for Failure {
    fn from(e: ExecError) -> Self {
        match e {
            ExecError::Missing => Failure::Missing,
            ExecError::Failed => Failure::Spawn,
            ExecError::TimedOut => Failure::TimedOut,
        }
    }
}

/// GitLab for glab's login on one instance.
pub struct GitlabProvider {
    /// Program to run; `glab` normally, a fake script in tests.
    glab: String,
    base: String,
    /// What glab is asked for (`--hostname`).
    host: String,
    timeout: Duration,
    me: Mutex<Option<GlUser>>,
    /// Conditional-request cache of GETs, keyed by path under `/api/v4`.
    cache: ConditionalCache,
    limit: Backoff,
    /// Commented-on MRs (project id, iid) seen merged or closed, and when:
    /// the list skips them for [`FINAL_RECHECK`] (`final_recheck`), then
    /// reads them once more in case one was reopened. Memory only.
    done: Mutex<HashMap<(u64, u64), DateTime<Utc>>>,
    final_recheck: chrono::Duration,
    /// The last successful read of each commented-on open MR and how many
    /// reads failed since: what the list shows while a read fails (see
    /// `list_reviews`). Memory only.
    commented: Mutex<HashMap<(u64, u64), (ReviewRef, u32)>>,
    /// The waits before each `/reviewers` read after adding the user as a
    /// reviewer ([`REVIEWER_RECHECK`]).
    reviewer_recheck: Vec<Duration>,
    /// The waits before each try of a request that could not connect
    /// ([`CONNECT_RETRIES`]).
    connect_retries: Vec<Duration>,
}

impl GitlabProvider {
    /// `base_url` names the instance (`https://gitlab.example.com`, or just
    /// its host). An empty or unusable URL is
    /// [`CoreError::GitlabNotConfigured`].
    pub fn new(base_url: &str) -> CoreResult<Self> {
        // Checked first: an unusable URL needs no login shell to find glab.
        normalize_base_url(base_url).ok_or(CoreError::GitlabNotConfigured)?;
        // Not a bare "glab": an app started from Finder has no Homebrew on
        // its PATH. Not installed, the bare name fails to spawn, which
        // reports as "glab not found".
        Self::with_binary(crate::platform::glab_binary().unwrap_or("glab"), base_url)
    }

    /// Like [`new`](Self::new) with an explicit `glab` program (tests).
    pub fn with_binary(glab: impl Into<String>, base_url: &str) -> CoreResult<Self> {
        let base = normalize_base_url(base_url).ok_or(CoreError::GitlabNotConfigured)?;
        Ok(Self {
            glab: glab.into(),
            host: url_host(&base).to_string(),
            base,
            timeout: REQUEST_TIMEOUT,
            me: Mutex::new(None),
            cache: ConditionalCache::default(),
            limit: Backoff::default(),
            done: Mutex::new(HashMap::new()),
            final_recheck: FINAL_RECHECK,
            commented: Mutex::new(HashMap::new()),
            reviewer_recheck: REVIEWER_RECHECK.to_vec(),
            connect_retries: CONNECT_RETRIES.to_vec(),
        })
    }

    /// Read a commented-on MR seen merged or closed again after `after`
    /// instead of [`FINAL_RECHECK`] (tests).
    pub fn with_final_recheck(mut self, after: chrono::Duration) -> Self {
        self.final_recheck = after;
        self
    }

    /// Wait `delays` before the `/reviewers` reads after adding the user as
    /// a reviewer instead of [`REVIEWER_RECHECK`] (tests: no sleeps).
    pub fn with_reviewer_recheck(mut self, delays: &[Duration]) -> Self {
        self.reviewer_recheck = delays.to_vec();
        self
    }

    /// Wait `delays` before each try of a request that could not connect
    /// instead of [`CONNECT_RETRIES`] (tests: no sleeps).
    pub fn with_connect_retries(mut self, delays: &[Duration]) -> Self {
        self.connect_retries = delays.to_vec();
        self
    }

    /// Kill a glab call after `timeout` instead of the default 30 s.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.glab);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Not a repository: glab would otherwise read the git remotes
            // of wherever Argus runs (`--hostname` decides the host anyway).
            .current_dir(std::env::temp_dir())
            .env("NO_COLOR", "1")
            .env("GLAB_NO_PROMPT", "true")
            .env("GLAB_CHECK_UPDATE", "false")
            // A usage event per poll call is noise on the user's instance.
            .env("GLAB_SEND_TELEMETRY", "false")
            // glab's HTTP debugging prints the request headers, its
            // `PRIVATE-TOKEN` among them, on stderr.
            .env_remove("GLAB_DEBUG_HTTP")
            .env_remove("DEBUG");
        #[cfg(windows)]
        {
            // No console window flashing up for every poll.
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd
    }

    /// One `glab api` call: `method` on `endpoint` (a path under `/api/v4`,
    /// or `graphql`), with `header` and `body` (JSON, on stdin), killed after
    /// the timeout. The response, whatever its status, or why there is none.
    fn call(
        &self,
        method: &str,
        endpoint: &str,
        header: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> Result<HttpReply, Failure> {
        let mut args = vec![
            "api",
            "--hostname",
            &self.host,
            "--include",
            "--method",
            method,
        ];
        if let Some(h) = header {
            args.extend(["--header", h]);
        }
        args.push(endpoint);
        if body.is_some() {
            args.extend(["--input", "-"]);
        }
        let mut cmd = self.command();
        cmd.args(&args);
        let input = body.map(|b| b.to_string().into_bytes());
        match run_with_timeout(&mut cmd, input, self.timeout) {
            Err(e) => Err(Failure::from(e)),
            Ok(output) => match HttpReply::parse(&String::from_utf8_lossy(&output.stdout)) {
                Some(reply) => Ok(reply),
                None => {
                    tracing::debug!("glab api {method} printed no response");
                    Err(Failure::from_stderr(&output.stderr))
                }
            },
        }
    }

    /// A GET, tried again (after a short wait) while the connection could
    /// not be made. A response with any status is never retried, and
    /// neither is a timeout: a server that hung for the whole timeout would
    /// only hang again.
    fn send(&self, path: &str, validator: Option<&str>) -> CoreResult<HttpReply> {
        let mut last = Failure::NotConnected;
        for delay in &self.connect_retries {
            std::thread::sleep(*delay);
            match self.call("GET", path, validator, None) {
                Ok(reply) => return Ok(reply),
                Err(f @ (Failure::NotConnected | Failure::Broken)) => last = f,
                Err(f) => return Err(f.into_error(&self.host)),
            }
        }
        Err(last.into_error(&self.host))
    }

    /// One conditional GET of `path` (under `/api/v4`): the body (from the
    /// cache on a 304) and the next page, if the `X-Next-Page` header names
    /// one.
    fn get(&self, path: &str) -> CoreResult<(String, Option<u32>)> {
        self.limit.check(ProviderId::Gitlab)?;
        let cached = self.cache.get(path);
        let validator = cached
            .as_deref()
            .and_then(Cached::validator)
            .map(|(name, value)| format!("{name}: {value}"));
        let reply = self.send(path, validator.as_deref())?;
        let parse_next = |v: Option<&str>| v.and_then(|v| v.trim().parse().ok());
        match reply.status {
            200..=299 => {
                let next_raw = reply.header("x-next-page").filter(|v| !v.is_empty());
                let next = parse_next(next_raw.as_deref());
                self.cache.put(
                    path,
                    Cached {
                        etag: reply.header("etag"),
                        last_modified: reply.header("last-modified"),
                        body: reply.body.clone(),
                        next: next_raw,
                    },
                );
                Ok((reply.body, next))
            }
            // Unchanged: the cached response stands.
            304 => cached
                .map(|c| (c.body.clone(), parse_next(c.next.as_deref())))
                .ok_or_else(unexpected),
            429 => Err(self.limit.hit(
                ProviderId::Gitlab,
                reset_at(|h| reply.header(h), Utc::now()),
            )),
            // glab follows redirects; one it hands back went nowhere.
            300..=399 => Err(CoreError::Provider(
                "GitLab redirected the request; check the GitLab URL in Settings".into(),
            )),
            401 | 403 => Err(CoreError::GitlabAuth {
                host: self.host.clone(),
            }),
            404 => Err(CoreError::NotFound),
            422 => Err(super::unprocessable("GitLab")),
            status => {
                tracing::debug!("glab api answered HTTP {status}");
                Err(CoreError::Provider(
                    "GitLab could not fetch the merge request".into(),
                ))
            }
        }
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> CoreResult<T> {
        let (body, _) = self.get(path)?;
        serde_json::from_str(&body).map_err(|_| unexpected())
    }

    /// Every page of a list endpoint, following `X-Next-Page`.
    ///
    /// A full page without a next one is followed anyway: a 304 replays the
    /// header as it was when the page was cached, and a page that was the
    /// last at exactly [`PER_PAGE`] items stays unchanged (so a 304) when
    /// more items arrive after it. An empty next page ends it, and so does
    /// that unasked-for page not existing (a 404 or 422, see
    /// [`super::ends_list`]): the list read so far is complete as far as
    /// GitLab said. Any other failure of it fails the list.
    fn get_all<T: DeserializeOwned>(&self, path: &str) -> CoreResult<Vec<T>> {
        self.get_pages(path, MAX_PAGES)
    }

    /// [`get_all`](Self::get_all) with its own page cap; reaching it is
    /// logged.
    fn get_pages<T: DeserializeOwned>(&self, path: &str, max_pages: u32) -> CoreResult<Vec<T>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut all = Vec::new();
        let mut page = 1;
        let mut guessed = false;
        for _ in 0..max_pages {
            let (body, next) =
                match self.get(&format!("{path}{sep}per_page={PER_PAGE}&page={page}")) {
                    Ok(reply) => reply,
                    Err(e) if guessed && super::ends_list(&e) => {
                        tracing::debug!("a GitLab list ends at a page it did not name: {e}");
                        return Ok(all);
                    }
                    Err(e) => return Err(e),
                };
            let items: Vec<T> = serde_json::from_str(&body).map_err(|_| unexpected())?;
            let full = items.len() >= PER_PAGE as usize;
            all.extend(items);
            match next {
                Some(n) if n > page => {
                    page = n;
                    guessed = false;
                }
                None if full => {
                    page += 1;
                    guessed = true;
                }
                _ => return Ok(all),
            }
        }
        tracing::warn!("a GitLab list was cut at {max_pages} pages");
        Ok(all)
    }

    fn forget_commented(&self, project: u64, iid: u64) {
        self.commented
            .lock()
            .expect("commented lock poisoned")
            .remove(&(project, iid));
    }

    fn me(&self) -> CoreResult<GlUser> {
        let mut me = self.me.lock().expect("me lock poisoned");
        if let Some(m) = me.as_ref() {
            return Ok(m.clone());
        }
        let user: GlUser = self.get_json("/user")?;
        *me = Some(user.clone());
        Ok(user)
    }

    fn mr_path(r: &ReviewRef) -> String {
        format!(
            "/projects/{}/merge_requests/{}",
            encode_segment(&r.repo),
            r.number
        )
    }

    fn to_ref(&self, mr: &GlMr) -> ReviewRef {
        ReviewRef {
            provider: ProviderId::Gitlab,
            host: self.host.clone(),
            repo: mr.project_path(),
            number: mr.iid,
            url: mr.web_url.clone(),
            title: mr.title.clone(),
            author: mr.author.username.clone(),
            updated_at: mr.updated_at,
        }
    }
}

impl GitlabProvider {
    /// The MR's own diff, labelled with the version it is of. `/diffs` (and
    /// `/changes`) always diff the latest version, whatever head was asked
    /// for, so the versions are read before and after it: when the latest
    /// is the same both times, the diff is that version's. A push in
    /// between reads it all once more.
    fn review_diff(&self, r: &ReviewRef, base: &str, head: &str) -> CoreResult<FileDiff> {
        let p = Self::mr_path(r);
        let latest = || -> CoreResult<Option<GlVersion>> {
            // Newest first, as GitLab lists them.
            Ok(self
                .get_all::<GlVersion>(&format!("{p}/versions"))?
                .into_iter()
                .next())
        };
        for _ in 0..2 {
            let before = latest()?;
            // `/diffs` (15.7+) pages; an older instance only has the
            // single-shot `/changes`, whose `overflow` flags a cut list and
            // whose `diff_refs` name the version it is of.
            let (mut diffs, truncated, refs) = match self.get_all::<GlDiff>(&format!("{p}/diffs")) {
                Ok(d) => {
                    let full = d.len() >= (PER_PAGE * MAX_PAGES) as usize;
                    (d, full, None)
                }
                Err(CoreError::NotFound) => {
                    let c: GlChanges = self.get_json(&format!("{p}/changes"))?;
                    (c.changes, c.overflow, c.diff_refs)
                }
                Err(e) => return Err(e),
            };
            self.fill_collapsed(&p, &mut diffs);
            let after = latest()?;
            if before != after {
                continue;
            }
            // An MR without a version has no diff either; the shas asked
            // for are as good as any then.
            let (base_sha, head_sha) = match (refs, after) {
                (
                    Some(GlDiffRefs {
                        base_sha: Some(b),
                        head_sha: Some(h),
                        ..
                    }),
                    _,
                ) => (b, h),
                (_, Some(v)) => (v.base_commit_sha, v.head_commit_sha),
                _ => (base.to_string(), head.to_string()),
            };
            return Ok(FileDiff {
                files: diffs
                    .into_iter()
                    .map(|d| changed_file(d, &head_sha))
                    .collect(),
                base_sha,
                head_sha,
                truncated,
                base_approximate: false,
            });
        }
        Err(CoreError::Provider(
            "the merge request got new commits while loading its diff; try again".into(),
        ))
    }

    /// Fill in the diffs GitLab collapsed, reading them straight from
    /// Gitaly (`access_raw_diffs`).
    ///
    /// Past its size limits GitLab sends a file's entry without its diff;
    /// only since 18.4 does it say so (`collapsed`), and the web UI fetches
    /// such a file on demand. Argus cannot leave it at that: the code view
    /// still shows the file (it reads both sides whole), but a comment can
    /// only be placed on a patch, so every comment on that file would be
    /// refused as "no text diff". The raw read has no collapse limit.
    ///
    /// One extra read, and only when an entry came back without a diff; a
    /// failure leaves the entries as they were (the file then reads as too
    /// large, as before).
    fn fill_collapsed(&self, mr_path: &str, diffs: &mut [GlDiff]) {
        if !diffs.iter().any(needs_raw_diff) {
            return;
        }
        let raw: GlChanges =
            match self.get_json(&format!("{mr_path}/changes?access_raw_diffs=true")) {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("reading the raw GitLab diffs failed: {e}");
                    return;
                }
            };
        let mut by_path: HashMap<(String, String), String> = raw
            .changes
            .into_iter()
            .map(|d| ((d.old_path, d.new_path), d.diff))
            .collect();
        for d in diffs.iter_mut().filter(|d| needs_raw_diff(d)) {
            let key = (d.old_path.clone(), d.new_path.clone());
            if let Some(diff) = by_path.remove(&key).filter(|s| !s.is_empty()) {
                d.diff = diff;
            }
        }
    }
}

/// An entry GitLab sent without its diff, which a raw read may still have.
/// A file it flagged as too large to send at all, and a binary one, have no
/// text diff anywhere; a rename or mode change without content changes has
/// none either, but nothing in the entry tells it apart from a collapsed
/// one, so it costs the one raw read as well.
fn needs_raw_diff(d: &GlDiff) -> bool {
    d.diff.is_empty() && !d.too_large && !looks_binary(&d.new_path)
}

fn unexpected() -> CoreError {
    CoreError::Provider("unexpected response from GitLab".into())
}

fn same_user(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl ReviewProvider for GitlabProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Gitlab
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn current_user(&self) -> CoreResult<String> {
        Ok(self.me()?.username)
    }

    /// Open MRs with the user as reviewer, plus open MRs the user commented
    /// on. GitLab has no "commented by me" filter, and scanning every open MR
    /// on a large instance for the user's notes is far too expensive, so the
    /// second half reads the user's own `commented` events of the last
    /// [`COMMENTED_DAYS`] days and fetches the MRs they point at, a few at
    /// a time. One of those failing stays listed as it was last read (or is
    /// skipped when it never was; a rate limit stops the list): dropping it
    /// would mark the stored review unlisted for a poll. One seen merged or
    /// closed is not fetched again for [`FINAL_RECHECK`].
    fn list_reviews(&self) -> CoreResult<Vec<ReviewRef>> {
        let me = self.me()?;
        let reviewing: Vec<GlMr> = self.get_all(&format!(
            "/merge_requests?reviewer_id={}&state=opened&scope=all",
            me.id
        ))?;
        let mut known: BTreeSet<(u64, u64)> =
            reviewing.iter().map(|m| (m.project_id, m.iid)).collect();
        let mut list: Vec<ReviewRef> = reviewing
            .iter()
            .filter(|m| !same_user(&m.author.username, &me.username))
            .map(|m| self.to_ref(m))
            .collect();

        let after = (Utc::now() - chrono::Duration::days(COMMENTED_DAYS)).format("%Y-%m-%d");
        let events: Vec<GlEvent> =
            self.get_all(&format!("/events?action=commented&after={after}"))?;
        let now = Utc::now();
        let done: HashSet<(u64, u64)> = {
            let mut done = self.done.lock().expect("done lock poisoned");
            done.retain(|_, at| now - *at < self.final_recheck);
            done.keys().copied().collect()
        };
        let mut wanted: Vec<(u64, u64)> = Vec::new();
        for e in events {
            let Some(note) = e.note else { continue };
            let (Some(project), Some(iid)) = (e.project_id, note.noteable_iid) else {
                continue;
            };
            if note.noteable_type.as_deref() == Some("MergeRequest")
                && !done.contains(&(project, iid))
                && known.insert((project, iid))
            {
                wanted.push((project, iid));
            }
        }
        // A few at a time: one per commented MR, every poll.
        let mut fetched: Vec<CoreResult<GlMr>> = Vec::with_capacity(wanted.len());
        for chunk in wanted.chunks(LIST_PARALLELISM) {
            std::thread::scope(|s| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(project, iid)| {
                        s.spawn(move || {
                            self.get_json::<GlMr>(&format!(
                                "/projects/{project}/merge_requests/{iid}"
                            ))
                        })
                    })
                    .collect();
                fetched.extend(handles.into_iter().map(join));
            });
        }
        for ((project, iid), mr) in wanted.into_iter().zip(fetched) {
            let mr = match mr {
                Ok(mr) => mr,
                // The whole provider is paused: so is the list.
                Err(e) if super::is_rate_limited(&e) => return Err(e),
                // Gone or no longer visible: just not listed.
                Err(CoreError::NotFound) => {
                    self.forget_commented(project, iid);
                    continue;
                }
                // One MR failing never costs the rest of the list, nor
                // that MR its place in it for a few polls; the next poll
                // tries it again.
                Err(e) => {
                    tracing::debug!("commented MR {project}!{iid} unreadable: {e}");
                    let mut commented = self.commented.lock().expect("commented lock poisoned");
                    if let Some((last, failures)) = commented.get_mut(&(project, iid)) {
                        *failures += 1;
                        if *failures <= COMMENTED_FAILURES_KEPT {
                            list.push(last.clone());
                        } else {
                            commented.remove(&(project, iid));
                        }
                    }
                    continue;
                }
            };
            if mr.state != "opened" {
                // Merged or closed is final for Argus (the poll treats it
                // so too): not fetched again for a while.
                self.done
                    .lock()
                    .expect("done lock poisoned")
                    .insert((project, iid), now);
                self.forget_commented(project, iid);
                continue;
            }
            if !same_user(&mr.author.username, &me.username) {
                let r = self.to_ref(&mr);
                self.commented
                    .lock()
                    .expect("commented lock poisoned")
                    .insert((project, iid), (r.clone(), 0));
                list.push(r);
            }
        }
        list.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(list)
    }

    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
        let me = self.me()?.username;
        let p = Self::mr_path(r);
        let (mr, commits, versions, reviewers, approvals, notes) = std::thread::scope(|s| {
            let a = s.spawn(|| self.get_json::<GlMr>(&p));
            let b = s.spawn(|| self.get_all::<GlCommit>(&format!("{p}/commits")));
            let c = s.spawn(|| self.get_all::<GlVersion>(&format!("{p}/versions")));
            let d = s.spawn(|| self.get_all::<GlReviewer>(&format!("{p}/reviewers")));
            let e = s.spawn(|| self.get_json::<GlApprovals>(&format!("{p}/approvals")));
            let f = s.spawn(|| self.review_notes(&p));
            (join(a), join(b), join(c), join(d), join(e), join(f))
        });
        let mr = mr?;
        // Reviewer states and approvals decide whether the review is
        // requested and approved, so a failure fails the fetch (the stored
        // review stays as it was) rather than guessing: a guess of
        // "requested" starts a false re-request round, one of "approved by
        // nobody" clears the approval. Only an instance or tier that does
        // not have the endpoint falls back.
        let reviewers = match reviewers {
            Ok(r) => Some(r),
            Err(CoreError::NotFound) => None,
            Err(e) => return Err(e),
        };
        let approved_by: Option<Vec<String>> = match approvals {
            Ok(a) => Some(a.approved_by.into_iter().map(|u| u.user.username).collect()),
            // 403: approvals are not in this tier (the MR itself was read).
            Err(CoreError::NotFound | CoreError::GitlabAuth { .. }) => None,
            Err(e) => return Err(e),
        };
        let mut detail = assemble_detail(
            self.to_ref(&mr),
            &me,
            mr,
            commits?,
            versions?,
            reviewers,
            approved_by,
        );
        // Only dates the state the reviewers read gave: without the notes
        // (a failed read) the state still decides, undated.
        if let Ok(notes) = notes {
            apply_review_notes(&mut detail, &me, &notes);
        }
        Ok(detail)
    }

    /// The MR and its commits: not `/reviewers` or `/approvals`.
    fn fetch_head(&self, r: &ReviewRef) -> CoreResult<ReviewHead> {
        let p = Self::mr_path(r);
        let (mr, commits) = std::thread::scope(|s| {
            let a = s.spawn(|| self.get_json::<GlMr>(&p));
            let b = s.spawn(|| self.get_all::<GlCommit>(&format!("{p}/commits")));
            (join(a), join(b))
        });
        let mr = mr?;
        Ok(ReviewHead {
            head_sha: mr.head_sha(),
            base_sha: mr
                .diff_refs
                .as_ref()
                .and_then(|r| r.base_sha.clone())
                .unwrap_or_default(),
            commits: commit_infos(commits?),
        })
    }

    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
        let p = Self::mr_path(r);
        let (mr, discussions) = std::thread::scope(|s| {
            let a = s.spawn(|| self.get_json::<GlMr>(&p));
            let b = s.spawn(|| {
                self.get_pages::<GlDiscussion>(&format!("{p}/discussions"), DISCUSSION_PAGES)
            });
            (join(a), join(b))
        });
        let head = mr?.head_sha();
        Ok(discussions?
            .into_iter()
            .filter_map(|d| discussion_thread(d, &head))
            .collect())
    }

    fn file_at(&self, r: &ReviewRef, sha: &str, path: &str) -> CoreResult<String> {
        check_sha(sha)?;
        let (body, _) = self.get(&format!(
            "/projects/{}/repository/files/{}/raw?ref={sha}",
            encode_segment(&r.repo),
            encode_segment(path)
        ))?;
        Ok(body)
    }

    fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        let project = encode_segment(&r.repo);
        match range {
            DiffRange::Review { base, head } => {
                check_sha(base)?;
                check_sha(head)?;
                self.review_diff(r, base, head)
            }
            DiffRange::Compare { from, to } => {
                check_sha(from)?;
                check_sha(to)?;
                // Straight: the plain from..to diff, so the left side is
                // exactly the commit last reviewed.
                let cmp: GlCompare = self.get_json(&format!(
                    "/projects/{project}/repository/compare?from={from}&to={to}&straight=true"
                ))?;
                Ok(FileDiff {
                    files: cmp.diffs.into_iter().map(|d| changed_file(d, to)).collect(),
                    base_sha: from.clone(),
                    head_sha: to.clone(),
                    truncated: cmp.compare_timeout,
                    base_approximate: false,
                })
            }
        }
    }
    // --- M4 writes (helpers in the "M4: writes" section at the end) ---

    /// GitLab takes a text position on any unchanged line, also outside
    /// the diff's hunks: given both `old_line` and `new_line`, it unfolds
    /// the diff around the position to find its line code
    /// (`Gitlab::Diff::Position#diff_file` → `unfold_diff_lines`), as its
    /// own UI does for a comment on expanded lines.
    fn outside_diff(&self) -> OutsideDiff {
        OutsideDiff::Native
    }

    /// The position comes from the latest diff version, read right before
    /// posting: GitLab anchors a comment to a version's three shas.
    fn post_comment(&self, r: &ReviewRef, c: &PlacedComment) -> PostResultOf<PostedComment> {
        let p = Self::mr_path(r);
        let versions: Vec<GlVersion> = self.get_json(&format!("{p}/versions"))?;
        // Newest first, as GitLab lists them.
        let latest = versions.into_iter().next().ok_or_else(|| {
            PostError::new(
                PostErrorKind::Provider,
                "GitLab has no diff for this merge request yet",
            )
        })?;
        if latest.head_commit_sha != c.head_sha {
            return Err(PostError::new(
                PostErrorKind::HeadMoved,
                "The merge request got new commits while posting; refresh and post again.",
            ));
        }
        let body = discussion_body(c, &latest);
        let created: GlCreatedDiscussion =
            self.post_json(&format!("{p}/discussions"), Some(&body), GlWrite::Comment)?;
        let note = created
            .notes
            .first()
            .map(|n| n.id)
            .ok_or_else(|| PostError::from(unexpected()))?;
        Ok(PostedComment {
            url: self.note_url(r, note),
            provider_thread_id: created.id,
            provider_note_id: note.to_string(),
            path: c.path.clone(),
            side: c.side,
            line: c.line_number(),
            start_line: c.start_number(),
            head_sha: c.head_sha.clone(),
        })
    }

    fn reply(&self, r: &ReviewRef, thread: &str, body: &str) -> PostResultOf<PostedNote> {
        let path = format!(
            "{}/discussions/{}/notes",
            Self::mr_path(r),
            encode_segment(thread)
        );
        let note: GlCreatedNote = self.post_json(
            &path,
            Some(&serde_json::json!({ "body": body })),
            GlWrite::Reply,
        )?;
        Ok(PostedNote {
            url: self.note_url(r, note.id),
            provider_thread_id: thread.to_string(),
            provider_note_id: note.id.to_string(),
        })
    }

    /// `sha` makes GitLab refuse (409) when the source branch's head is no
    /// longer the one the user reviewed.
    fn approve(&self, r: &ReviewRef, head_sha: &str) -> PostResultOf<()> {
        let _: serde_json::Value = self.post_json(
            &format!("{}/approve", Self::mr_path(r)),
            Some(&serde_json::json!({ "sha": head_sha })),
            GlWrite::Approve,
        )?;
        Ok(())
    }

    /// GitLab answers a user who may not resolve (on instances where only
    /// the MR author and maintainers can) with a 403: its message is
    /// forwarded as a refusal.
    fn set_thread_resolved(&self, r: &ReviewRef, thread: &str, resolved: bool) -> PostResultOf<()> {
        if thread.trim().is_empty() {
            return Err(PostError::new(
                PostErrorKind::InvalidInput,
                "not a GitLab discussion",
            ));
        }
        let _: serde_json::Value = self.write_json(
            "PUT",
            &format!(
                "{}/discussions/{}",
                Self::mr_path(r),
                encode_segment(thread)
            ),
            Some(&serde_json::json!({ "resolved": resolved })),
            GlWrite::Resolve,
        )?;
        Ok(())
    }

    // --- M3: thread history ---

    /// The diff endpoint names no parent, so the commit itself is read
    /// alongside for its first `parent_ids` entry: without it the walk
    /// cannot tell a merged side branch from a line of commits.
    fn commit_diff(&self, r: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        check_sha(sha)?;
        let commit = format!(
            "/projects/{}/repository/commits/{sha}",
            encode_segment(&r.repo)
        );
        let (diffs, parent) = std::thread::scope(|s| {
            let a = s.spawn(|| self.get_all::<GlDiff>(&format!("{commit}/diff")));
            let b = s.spawn(|| self.get_json::<GlCommitParents>(&commit));
            (join(a), join(b))
        });
        let diffs = diffs?;
        let parent = match parent {
            Ok(c) => c.parent_ids.into_iter().next().unwrap_or_default(),
            // Not readable on its own: no parent named (the walk then diffs
            // directly).
            Err(CoreError::NotFound) => String::new(),
            Err(e) => return Err(e),
        };
        Ok(FileDiff {
            base_sha: parent,
            head_sha: sha.to_string(),
            truncated: diffs.len() >= (PER_PAGE * MAX_PAGES) as usize,
            files: diffs.into_iter().map(|d| changed_file(d, sha)).collect(),
            base_approximate: false,
        })
    }

    /// GitLab keeps only the current resolution: a resolved discussion's
    /// latest `resolved_at` and who did it. A discussion resolved on an
    /// instance that does not send `resolved_at` has no time and is left
    /// out.
    fn thread_resolutions(&self, r: &ReviewRef) -> CoreResult<Vec<ThreadResolution>> {
        let discussions: Vec<GlResolveDiscussion> = self.get_pages(
            &format!("{}/discussions", Self::mr_path(r)),
            DISCUSSION_PAGES,
        )?;
        Ok(discussions
            .into_iter()
            .filter_map(|d| {
                let notes: Vec<&GlResolveNote> = d.notes.iter().filter(|n| n.resolvable).collect();
                if notes.is_empty() || !notes.iter().all(|n| n.resolved) {
                    return None;
                }
                let last = notes
                    .iter()
                    .filter(|n| n.resolved_at.is_some())
                    .max_by_key(|n| n.resolved_at)?;
                Some(ThreadResolution {
                    thread_id: d.id.clone(),
                    resolved: true,
                    by: last
                        .resolved_by
                        .as_ref()
                        .map(|u| u.username.clone())
                        .unwrap_or_default(),
                    at: last.resolved_at?,
                })
            })
            .collect())
    }

    // --- Submit review (helpers in the "Review state" section) ---

    fn submit_review(
        &self,
        r: &ReviewRef,
        verdict: ReviewVerdict,
        head_sha: &str,
        summary: Option<&str>,
    ) -> PostResultOf<()> {
        match verdict {
            ReviewVerdict::Approve => self.approve(r, head_sha)?,
            ReviewVerdict::RequestChanges => self.request_changes(r)?,
            ReviewVerdict::Comment => {
                return Err(PostError::new(
                    PostErrorKind::Unsupported,
                    "GitLab has no comment-only review: approve or request changes",
                ))
            }
        }
        match summary {
            Some(text) => self.post_summary(r, text),
            None => Ok(()),
        }
    }

    fn rate_limited_until(&self) -> Option<DateTime<Utc>> {
        self.limit.active()
    }
}

/// A GitLab diff entry as a [`ChangedFile`]. GitLab sends no line counts
/// (they are counted from the diff) and no blob sha: the content key is a
/// digest of the diff, or of the head when the diff was left out.
fn changed_file(d: GlDiff, head: &str) -> ChangedFile {
    let status = if d.new_file {
        FileStatus::Added
    } else if d.deleted_file {
        FileStatus::Removed
    } else if d.renamed_file {
        FileStatus::Renamed
    } else {
        FileStatus::Modified
    };
    let binary_marker = d.diff.starts_with("Binary files") || d.diff.contains("GIT binary patch");
    let binary = binary_marker || (d.diff.is_empty() && looks_binary(&d.new_path));
    let too_large = !binary && (d.too_large || (d.collapsed && d.diff.is_empty()));
    let has_patch = !d.diff.is_empty() && !binary_marker;
    let (additions, deletions) = if has_patch {
        count_patch(&d.diff)
    } else {
        (0, 0)
    };
    let content_sha = if has_patch {
        digest(&[&d.new_path, &d.b_mode, &d.diff])
    } else {
        digest(&[&d.new_path, &d.b_mode, head])
    };
    let path = if status == FileStatus::Removed {
        d.old_path.clone()
    } else {
        d.new_path.clone()
    };
    ChangedFile {
        old_path: (status == FileStatus::Renamed).then_some(d.old_path),
        path,
        status,
        additions,
        deletions,
        patch: has_patch.then_some(d.diff),
        content_sha,
        binary,
        too_large,
    }
}

fn join<T>(h: std::thread::ScopedJoinHandle<'_, CoreResult<T>>) -> CoreResult<T> {
    h.join()
        .unwrap_or_else(|_| Err(CoreError::Provider("a GitLab fetch panicked".into())))
}

fn reviewer_state(s: &str) -> ReviewerState {
    match s {
        "approved" => ReviewerState::Approved,
        "requested_changes" => ReviewerState::ChangesRequested,
        "reviewed" | "unapproved" => ReviewerState::Commented,
        // `unreviewed`, `review_started`: the review is still owed.
        _ => ReviewerState::Requested,
    }
}

fn assemble_detail(
    review: ReviewRef,
    me: &str,
    mr: GlMr,
    commits: Vec<GlCommit>,
    versions: Vec<GlVersion>,
    reviewers: Option<Vec<GlReviewer>>,
    approved_by: Option<Vec<String>>,
) -> ReviewDetail {
    // Without the approvals API, the user's own reviewer state says it.
    let approved_by = approved_by.unwrap_or_else(|| {
        reviewers
            .iter()
            .flatten()
            .filter(|r| reviewer_state(&r.state) == ReviewerState::Approved)
            .map(|r| r.user.username.clone())
            .collect()
    });
    let i_approved = approved_by.iter().any(|u| same_user(u, me));
    let my_entry: Option<String> = reviewers
        .iter()
        .flatten()
        .find(|r| same_user(&r.user.username, me))
        .map(|r| r.state.clone());
    let listed_me = reviewers.is_none() && mr.reviewers.iter().any(|u| same_user(&u.username, me));
    let (mut list, requested_from_me) = match reviewers {
        Some(rs) => {
            let list: Vec<Reviewer> = rs
                .into_iter()
                .map(|r| Reviewer {
                    is_me: same_user(&r.user.username, me),
                    state: reviewer_state(&r.state),
                    username: r.user.username,
                })
                .collect();
            // A re-request resets the entry to `unreviewed`.
            let requested = list
                .iter()
                .any(|r| r.is_me && r.state == ReviewerState::Requested);
            (list, requested)
        }
        None => {
            // No per-reviewer state: being listed counts as requested until
            // the user's approval shows up.
            let list: Vec<Reviewer> = mr
                .reviewers
                .iter()
                .map(|u| Reviewer {
                    is_me: same_user(&u.username, me),
                    state: ReviewerState::Requested,
                    username: u.username.clone(),
                })
                .collect();
            let requested = !i_approved && list.iter().any(|r| r.is_me);
            (list, requested)
        }
    };
    for user in &approved_by {
        match list.iter_mut().find(|r| same_user(&r.username, user)) {
            Some(r) if !(r.is_me && requested_from_me) => r.state = ReviewerState::Approved,
            Some(_) => {}
            None => list.push(Reviewer {
                is_me: same_user(user, me),
                username: user.clone(),
                state: ReviewerState::Approved,
            }),
        }
    }
    let state = match mr.state.as_str() {
        "merged" => ReviewState::Merged,
        "closed" => ReviewState::Closed,
        _ => ReviewState::Open,
    };
    let head_sha = mr.head_sha();
    let refs = mr.diff_refs.clone();
    let commits = commit_infos(commits);
    let mut versions: Vec<DiffVersion> = versions
        .into_iter()
        .map(|v| DiffVersion {
            head_sha: v.head_commit_sha,
            base_sha: v.base_commit_sha,
            start_sha: v.start_commit_sha,
            created_at: v.created_at,
        })
        .collect();
    versions.reverse();
    ReviewDetail {
        review,
        description: mr.description.unwrap_or_default(),
        state,
        draft: mr.draft || mr.work_in_progress,
        base_sha: refs
            .as_ref()
            .and_then(|r| r.base_sha.clone())
            .unwrap_or_default(),
        start_sha: refs.and_then(|r| r.start_sha),
        head_sha,
        commits,
        versions,
        reviewers: list,
        requested_from_me,
        approved_by_me: i_approved && !requested_from_me,
        my_review_state: my_review_state(my_entry.as_deref(), listed_me, i_approved),
        // Dated from the notes (`apply_review_notes`).
        my_reviewed_at: None,
        my_reviewed_sha: None,
        review_requested_at: None,
    }
}

/// GitLab's commits (newest first) as the model's, oldest first.
fn commit_infos(commits: Vec<GlCommit>) -> Vec<CommitInfo> {
    let mut out: Vec<CommitInfo> = commits
        .into_iter()
        .map(|c| CommitInfo {
            title: c.title.unwrap_or_else(|| first_line(&c.message)),
            author: c.author_name,
            authored_at: c.authored_date,
            sha: c.id,
        })
        .collect();
    out.reverse();
    out
}

/// A discussion as a thread; `None` for one made only of system notes
/// ("added 2 commits", "approved this merge request", ...). Outdated means
/// the thread's position was taken against a head that is no longer the MR
/// head.
fn discussion_thread(d: GlDiscussion, head: &str) -> Option<Thread> {
    let first = d.notes.iter().find(|n| !n.system)?;
    let anchor = d
        .notes
        .iter()
        .find_map(|n| n.position.as_ref())
        .filter(|p| p.position_type.as_deref().unwrap_or("text") == "text")
        .map(position_anchor);
    let resolvable: Vec<&GlNote> = d.notes.iter().filter(|n| n.resolvable).collect();
    let resolved = !resolvable.is_empty() && resolvable.iter().all(|n| n.resolved);
    Some(Thread {
        id: d.id.clone(),
        author: first.author.username.clone(),
        created_at: first.created_at,
        resolved,
        // GitLab marks the notes of a discussion it lets be resolved (diff
        // notes, MR-level threads); a single comment has none.
        resolvable: !d.individual_note && !resolvable.is_empty(),
        outdated: anchor
            .as_ref()
            .is_some_and(|a| !head.is_empty() && a.head_sha != head),
        anchor,
        notes: d
            .notes
            .iter()
            .map(|n| Note {
                id: n.id.to_string(),
                author: n.author.username.clone(),
                body: n.body.clone(),
                created_at: n.created_at,
                system: n.system,
            })
            .collect(),
    })
}

fn position_anchor(p: &GlPosition) -> Anchor {
    // A line only on the new side is added, only on the old side removed;
    // both means an unchanged (context) line, anchored on the new side.
    let side = if p.new_line.is_some() {
        Side::New
    } else {
        Side::Old
    };
    let start_line = p.line_range.as_ref().and_then(|r| {
        let start = match side {
            Side::New => r.start.new_line.or(r.start.old_line),
            Side::Old => r.start.old_line,
        }?;
        let end = match side {
            Side::New => p.new_line,
            Side::Old => p.old_line,
        };
        (Some(start) != end).then_some(start)
    });
    Anchor {
        path_old: p.old_path.clone(),
        path_new: p.new_path.clone(),
        line_old: p.old_line,
        line_new: p.new_line,
        start_line,
        side,
        base_sha: p.base_sha.clone(),
        start_sha: p.start_sha.clone(),
        head_sha: p.head_sha.clone().unwrap_or_default(),
        diff_hunk: None,
        current_line: None,
        outside_diff: false,
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GlUser {
    #[serde(default)]
    id: u64,
    username: String,
}

#[derive(Clone, Deserialize)]
struct GlDiffRefs {
    base_sha: Option<String>,
    head_sha: Option<String>,
    start_sha: Option<String>,
}

#[derive(Deserialize)]
struct GlReferences {
    full: Option<String>,
}

#[derive(Deserialize)]
struct GlMr {
    iid: u64,
    project_id: u64,
    title: String,
    #[serde(default)]
    description: Option<String>,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    work_in_progress: bool,
    web_url: String,
    author: GlUser,
    updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    diff_refs: Option<GlDiffRefs>,
    #[serde(default)]
    references: Option<GlReferences>,
    #[serde(default)]
    reviewers: Vec<GlUser>,
}

impl GlMr {
    /// `group/sub/project` from `references.full` (`group/sub/project!12`),
    /// else from the web URL.
    fn project_path(&self) -> String {
        if let Some(full) = self.references.as_ref().and_then(|r| r.full.as_deref()) {
            if let Some((path, _)) = full.rsplit_once('!') {
                return path.to_string();
            }
        }
        let rest = self
            .web_url
            .split_once("://")
            .map_or(&*self.web_url, |(_, r)| r);
        let path = rest.split_once('/').map_or("", |(_, p)| p);
        path.split("/-/").next().unwrap_or("").to_string()
    }

    fn head_sha(&self) -> String {
        self.diff_refs
            .as_ref()
            .and_then(|r| r.head_sha.clone())
            .or_else(|| self.sha.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct GlCommit {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    author_name: String,
    authored_date: Option<DateTime<Utc>>,
}

/// A commit, for its parents.
#[derive(Deserialize)]
struct GlCommitParents {
    #[serde(default)]
    parent_ids: Vec<String>,
}

#[derive(Deserialize, PartialEq, Eq)]
struct GlVersion {
    head_commit_sha: String,
    base_commit_sha: String,
    start_commit_sha: String,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct GlReviewer {
    user: GlUser,
    state: String,
}

#[derive(Deserialize)]
struct GlApprover {
    user: GlUser,
}

#[derive(Deserialize)]
struct GlApprovals {
    #[serde(default)]
    approved_by: Vec<GlApprover>,
}

#[derive(Deserialize)]
struct GlEventNote {
    noteable_type: Option<String>,
    noteable_iid: Option<u64>,
}

#[derive(Deserialize)]
struct GlEvent {
    project_id: Option<u64>,
    note: Option<GlEventNote>,
}

#[derive(Deserialize)]
struct GlLinePoint {
    old_line: Option<u32>,
    new_line: Option<u32>,
}

#[derive(Deserialize)]
struct GlLineRange {
    start: GlLinePoint,
}

#[derive(Deserialize)]
struct GlPosition {
    base_sha: Option<String>,
    start_sha: Option<String>,
    head_sha: Option<String>,
    old_path: Option<String>,
    new_path: Option<String>,
    position_type: Option<String>,
    old_line: Option<u32>,
    new_line: Option<u32>,
    line_range: Option<GlLineRange>,
}

#[derive(Deserialize)]
struct GlNote {
    id: u64,
    body: String,
    author: GlUser,
    created_at: DateTime<Utc>,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    resolvable: bool,
    #[serde(default)]
    resolved: bool,
    #[serde(default)]
    position: Option<GlPosition>,
}

#[derive(Deserialize)]
struct GlDiff {
    old_path: String,
    new_path: String,
    #[serde(default)]
    b_mode: String,
    #[serde(default)]
    diff: String,
    #[serde(default)]
    new_file: bool,
    #[serde(default)]
    renamed_file: bool,
    #[serde(default)]
    deleted_file: bool,
    #[serde(default)]
    too_large: bool,
    #[serde(default)]
    collapsed: bool,
}

#[derive(Deserialize)]
struct GlChanges {
    #[serde(default)]
    changes: Vec<GlDiff>,
    #[serde(default)]
    overflow: bool,
    #[serde(default)]
    diff_refs: Option<GlDiffRefs>,
}

#[derive(Deserialize)]
struct GlCompare {
    #[serde(default)]
    diffs: Vec<GlDiff>,
    #[serde(default)]
    compare_timeout: bool,
}

#[derive(Deserialize)]
struct GlDiscussion {
    id: String,
    /// A single comment rather than a thread ("Comment", not "Start a
    /// thread"): never resolvable.
    #[serde(default)]
    individual_note: bool,
    notes: Vec<GlNote>,
}

// ---------------------------------------------------------------------------
// M4: writes. Only `Core`'s post API reaches these, on an explicit user
// action.
// ---------------------------------------------------------------------------

/// Which write a response answers (GitLab's 401 means different things).
#[derive(Clone, Copy, PartialEq, Eq)]
enum GlWrite {
    Comment,
    Reply,
    Approve,
    Resolve,
    /// Adding the user as a reviewer (GraphQL).
    Reviewer,
}

impl GitlabProvider {
    /// POST `body` (JSON) to `path` under `/api/v4`; the response parsed as
    /// `T`. Retried only when the connection could not be made: a request
    /// that was sent is never sent twice, so a comment is never doubled.
    fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&serde_json::Value>,
        what: GlWrite,
    ) -> PostResultOf<T> {
        self.write_json("POST", path, body, what)
    }

    /// [`post_json`](Self::post_json) with any method (`PUT` to resolve).
    fn write_json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
        what: GlWrite,
    ) -> PostResultOf<T> {
        self.write_to(method, path, body, what)
    }

    /// One GraphQL mutation (`query` with `variables`) through `glab api
    /// graphql`: sent like any other write, GitLab's answer parsed as `T`
    /// (its `errors` are the caller's to read: GraphQL fails with a 200,
    /// which glab prints and exits 1 on).
    fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
        what: GlWrite,
    ) -> PostResultOf<GqlReply<T>> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        self.write_to("POST", "graphql", Some(&body), what)
    }

    /// The write itself: `body` (JSON) sent with `method` to `endpoint`,
    /// tried again only when the connection could not be made (glab's
    /// `dial tcp` failure: nothing left this machine).
    fn write_to<T: DeserializeOwned>(
        &self,
        method: &str,
        endpoint: &str,
        body: Option<&serde_json::Value>,
        what: GlWrite,
    ) -> PostResultOf<T> {
        let mut reply = None;
        for delay in &self.connect_retries {
            std::thread::sleep(*delay);
            match self.call(method, endpoint, None, body) {
                Ok(r) => {
                    reply = Some(r);
                    break;
                }
                Err(Failure::NotConnected) => {
                    tracing::debug!("a GitLab write could not connect");
                }
                // It may have reached GitLab before it hung or broke.
                Err(Failure::TimedOut | Failure::Broken) => {
                    return Err(PostError::new(
                        PostErrorKind::Provider,
                        "GitLab did not answer; refresh to see whether it went through",
                    ));
                }
                Err(f) => return Err(f.into_error(&self.host).into()),
            }
        }
        let Some(reply) = reply else {
            return Err(Failure::NotConnected.into_error(&self.host).into());
        };
        if (200..300).contains(&reply.status) {
            return serde_json::from_str(&reply.body).map_err(|_| unexpected().into());
        }
        tracing::debug!("glab api {method} answered HTTP {}", reply.status);
        let reason = gitlab_error_message(&reply.body).map(|m| clean(&m));
        Err(write_error(reply.status, reason, what, &self.host))
    }

    /// The web URL of note `id` of the MR.
    fn note_url(&self, r: &ReviewRef, id: u64) -> String {
        let mr = if r.url.is_empty() {
            format!("{}/{}/-/merge_requests/{}", self.base, r.repo, r.number)
        } else {
            r.url.clone()
        };
        format!("{mr}#note_{id}")
    }
}

/// A failed write as a [`PostError`]; `reason` is GitLab's own message
/// (tokens redacted, see [`clean`]), forwarded so the user sees why.
fn write_error(status: u16, reason: Option<String>, what: GlWrite, host: &str) -> PostError {
    let with = |fallback: &str| {
        format!(
            "GitLab: {}",
            reason.clone().unwrap_or_else(|| fallback.to_string())
        )
    };
    match status {
        // glab follows redirects; one it hands back went nowhere.
        300..=399 => CoreError::Provider(
            "GitLab redirected the request; check the GitLab URL in Settings".into(),
        )
        .into(),
        // Approving answers 401 when the user may not approve (already
        // approved, own MR, not eligible), not only for a bad token.
        401 if what == GlWrite::Approve => PostError::new(
            PostErrorKind::PostRejected,
            with("401 Unauthorized")
                + " (already approved, or you may not approve this merge request)",
        ),
        401 => CoreError::GitlabAuth {
            host: host.to_string(),
        }
        .into(),
        // Approving with a `sha` that is not the source branch's head.
        409 if what == GlWrite::Approve => PostError::new(
            PostErrorKind::HeadMoved,
            "The merge request got new commits since you looked; review them before approving.",
        ),
        404 => PostError::new(PostErrorKind::NotFound, with("404 Not found")),
        429 => CoreError::Provider("GitLab rate limit reached, try again later".into()).into(),
        400..=499 => PostError::new(PostErrorKind::PostRejected, with(&format!("HTTP {status}"))),
        _ => PostError::new(
            PostErrorKind::Provider,
            with(&format!("could not post (HTTP {status})")),
        ),
    }
}

/// GitLab's error text from a JSON error body: `message` (a string, a list
/// or a field → errors object) or `error` + `error_description`. `None` for
/// a body that is not such JSON (a proxy's HTML page is never forwarded).
fn gitlab_error_message(body: &str) -> Option<String> {
    fn flatten(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => {
                items.iter().map(flatten).collect::<Vec<_>>().join(", ")
            }
            serde_json::Value::Object(map) => map
                .iter()
                .map(|(k, v)| format!("{k}: {}", flatten(v)))
                .collect::<Vec<_>>()
                .join("; "),
            other => other.to_string(),
        }
    }
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if let Some(m) = v.get("message") {
        return Some(flatten(m));
    }
    let error = flatten(v.get("error")?);
    Some(match v.get("error_description").and_then(|d| d.as_str()) {
        Some(d) => format!("{error}: {d}"),
        None => error,
    })
}

/// GitLab's line code: `<sha1 of the path>_<old>_<new>` with its own line
/// counters ([`DiffLine::old_pos`] / [`DiffLine::new_pos`]).
fn line_code(path: &str, row: &DiffLine) -> String {
    let digest = sha1_smol::Sha1::from(path).digest().to_string();
    format!("{digest}_{}_{}", row.old_pos, row.new_pos)
}

/// One end of a `line_range`.
fn range_point(path: &str, row: &DiffLine) -> serde_json::Value {
    serde_json::json!({
        "line_code": line_code(path, row),
        // GitLab: "new" for an added line, otherwise "old".
        "type": if row.kind == LineKind::Added { "new" } else { "old" },
        "old_line": row.old_line(),
        "new_line": row.new_line(),
    })
}

/// The body of a new diff discussion: a text position on `version`.
/// `new_line` alone for an added line, `old_line` alone for a removed one,
/// both for an unchanged line (GitLab rejects a context line given one
/// side only).
fn discussion_body(c: &PlacedComment, version: &GlVersion) -> serde_json::Value {
    let mut position = serde_json::json!({
        "position_type": "text",
        "base_sha": version.base_commit_sha,
        "start_sha": version.start_commit_sha,
        "head_sha": version.head_commit_sha,
        "old_path": c.old_path,
        "new_path": c.path,
    });
    if let Some(old) = c.line.old_line() {
        position["old_line"] = old.into();
    }
    if let Some(new) = c.line.new_line() {
        position["new_line"] = new.into();
    }
    if let Some(start) = &c.start {
        position["line_range"] = serde_json::json!({
            "start": range_point(&c.path, start),
            "end": range_point(&c.path, &c.line),
        });
    }
    serde_json::json!({ "body": c.body, "position": position })
}

#[derive(Deserialize)]
struct GlCreatedNote {
    id: u64,
}

#[derive(Deserialize)]
struct GlCreatedDiscussion {
    id: String,
    notes: Vec<GlCreatedNote>,
}

/// A discussion as read for its resolve events (M3).
#[derive(Deserialize)]
struct GlResolveDiscussion {
    id: String,
    notes: Vec<GlResolveNote>,
}

#[derive(Deserialize)]
struct GlResolveNote {
    #[serde(default)]
    resolvable: bool,
    #[serde(default)]
    resolved: bool,
    #[serde(default)]
    resolved_by: Option<GlUser>,
    #[serde(default)]
    resolved_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Review state: the user's own reviewer state, the notes that date it, and
// "Submit review" (approve / request changes). The reads run with every
// fetch; the writes only from `Core::submit_review`, on an explicit action.
// ---------------------------------------------------------------------------

/// The user's [`MyReviewState`] from their `/reviewers` entry (`entry`),
/// or without that endpoint from being listed as a reviewer (`listed`).
fn my_review_state(entry: Option<&str>, listed: bool, approved: bool) -> MyReviewState {
    match entry {
        Some("unreviewed") => MyReviewState::AwaitingReview,
        Some("review_started") => MyReviewState::ReviewStarted,
        Some("requested_changes") => MyReviewState::RequestedChanges,
        Some("approved") => MyReviewState::Approved,
        Some("reviewed" | "unapproved") => MyReviewState::Commented,
        // A state this build does not know: owed until it says otherwise.
        Some(_) => MyReviewState::AwaitingReview,
        None if approved => MyReviewState::Approved,
        None if listed => MyReviewState::AwaitingReview,
        None => MyReviewState::NotReviewer,
    }
}

/// The first GitLab whose `/request_changes` quick action is on by default
/// (introduced in 16.11 behind `mr_reviewer_requests_changes`, default on
/// in 17.2, the flag removed in 17.3). Earlier instances would post the
/// command as a plain comment, so Argus refuses before writing.
const REQUEST_CHANGES_SINCE: (u32, u32) = (17, 2);

/// The waits before each `/reviewers` read after adding the user as a
/// reviewer: GitLab updates the reviewers in the mutation itself, so the
/// first read normally lists them; the rest cover a replica that lags.
const REVIEWER_RECHECK: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_millis(250),
    Duration::from_millis(750),
    Duration::from_millis(1500),
];

/// What Argus says when an instance cannot request changes by API.
const NO_REQUEST_CHANGES: &str = "Your GitLab doesn't support requesting changes via the API (it needs GitLab 17.2 or later). Request changes in GitLab itself.";

#[derive(Deserialize)]
struct GlVersionInfo {
    version: String,
}

/// `major.minor` of a GitLab version string (`17.5.1-ee`).
pub(crate) fn parse_gitlab_version(v: &str) -> Option<(u32, u32)> {
    let mut parts = v.trim().split(['.', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// GitLab's answer to a note made only of quick actions (HTTP 202): the
/// command ran, no note was saved. A saved note (`id`) means the command
/// was not recognised and its text was posted as is.
#[derive(Deserialize)]
struct GlCommandsOnly {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    summary: Vec<String>,
}

/// Appends the users named to an MR's reviewers and returns the reviewers
/// after it (and whether the MR takes more than one).
const SET_REVIEWERS_APPEND: &str = "mutation($projectPath: ID!, $iid: String!, $usernames: [String!]!) { mergeRequestSetReviewers(input: { projectPath: $projectPath, iid: $iid, reviewerUsernames: $usernames, operationMode: APPEND }) { errors mergeRequest { allowsMultipleReviewers reviewers { nodes { username } } } } }";

/// What adding the user as a reviewer needs.
const NEEDS_EDIT: &str =
    "You need permission to edit this merge request's reviewers (the Developer role or higher).";

/// The user could not be added as a reviewer, so no change request went
/// out; `why` ends in a full stop.
fn not_added(why: &str) -> PostError {
    PostError::new(
        PostErrorKind::PostRejected,
        format!(
            "Argus couldn't add you as a reviewer, which GitLab requires before you can request changes: {why}"
        ),
    )
}

/// A GraphQL answer: `data`, and `errors` for a refused operation (GitLab
/// answers those with a 200).
#[derive(Deserialize)]
struct GqlReply<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlSetReviewersData {
    merge_request_set_reviewers: Option<GlSetReviewersPayload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlSetReviewersPayload {
    #[serde(default)]
    errors: Vec<String>,
    merge_request: Option<GlGqlMr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlGqlMr {
    allows_multiple_reviewers: Option<bool>,
    reviewers: Option<GlGqlNodes>,
}

#[derive(Deserialize)]
struct GlGqlNodes {
    #[serde(default)]
    nodes: Vec<Option<GlGqlUser>>,
}

#[derive(Deserialize)]
struct GlGqlUser {
    username: String,
}

impl GitlabProvider {
    /// Set the user's reviewer state to "requested changes" with the
    /// `/request_changes` quick action: the one way GitLab's API offers
    /// before 19.x (`draft_notes/bulk_publish` with `reviewer_state` came
    /// later, and the GraphQL `mergeRequestRequestChanges` later still).
    /// Checked first (reads only): the instance is 17.2 or later, and
    /// whether the user is a reviewer of the MR (the quick action exists
    /// only for one). If not, the user is added
    /// ([`add_me_as_reviewer`](Self::add_me_as_reviewer), the one extra
    /// write) and `/reviewers` read again until it lists them, a bounded
    /// number of times ([`REVIEWER_RECHECK`]). Checked after: GitLab ran it
    /// as a command (no note saved) and the user's state is now
    /// `requested_changes`.
    fn request_changes(&self, r: &ReviewRef) -> PostResultOf<()> {
        let p = Self::mr_path(r);
        let version: GlVersionInfo = self.get_json("/version")?;
        if parse_gitlab_version(&version.version).is_some_and(|v| v < REQUEST_CHANGES_SINCE) {
            return Err(PostError::new(
                PostErrorKind::Unsupported,
                NO_REQUEST_CHANGES,
            ));
        }
        let me = self.me()?.username;
        let mine = |reviewers: &[GlReviewer]| {
            reviewers
                .iter()
                .find(|x| same_user(&x.user.username, &me))
                .map(|x| x.state.clone())
        };
        let reviewers: Vec<GlReviewer> = self.get_all(&format!("{p}/reviewers"))?;
        if mine(&reviewers).is_none() {
            // The quick action exists only for a reviewer: add the user
            // first, then wait until `/reviewers` lists them.
            self.add_me_as_reviewer(r, &me)?;
            let mut listed = false;
            for delay in &self.reviewer_recheck {
                std::thread::sleep(*delay);
                let now: Vec<GlReviewer> = self.get_all(&format!("{p}/reviewers"))?;
                if mine(&now).is_some() {
                    listed = true;
                    break;
                }
            }
            if !listed {
                return Err(not_added(&format!(
                    "GitLab did not list you as one afterwards. {NEEDS_EDIT}"
                )));
            }
        }
        let answer: GlCommandsOnly = self.post_json(
            &format!("{p}/notes"),
            Some(&serde_json::json!({ "body": "/request_changes" })),
            GlWrite::Comment,
        )?;
        if answer.id.is_some() {
            return Err(PostError::new(
                PostErrorKind::Unsupported,
                format!(
                    "{NO_REQUEST_CHANGES} It posted \"/request_changes\" as a comment; delete it there."
                ),
            ));
        }
        let after: Vec<GlReviewer> = self.get_all(&format!("{p}/reviewers"))?;
        if mine(&after).as_deref() != Some("requested_changes") {
            let said = clean(&answer.summary.join(" "));
            return Err(PostError::new(
                PostErrorKind::PostRejected,
                if said.trim().is_empty() {
                    "GitLab did not record the change request.".to_string()
                } else {
                    format!("GitLab did not record the change request: {said}")
                },
            ));
        }
        Ok(())
    }

    /// Add the user (`me`) to the MR's reviewers, keeping the others:
    /// GraphQL `mergeRequestSetReviewers` with `operationMode: APPEND`
    /// (GitLab merges the lists server-side, so a reviewer added meanwhile
    /// is not lost the way a REST `PUT reviewer_ids` read-modify-write would
    /// lose them; in GitLab 17.2 already, `APPEND` included). Not the
    /// `/assign_reviewer me` quick action: without the permission GitLab
    /// drops it silently (a 202 without a reason), where the mutation says
    /// why. On Free (one reviewer per MR) GitLab keeps the existing one and
    /// does not add the user; that is reported, never forced.
    fn add_me_as_reviewer(&self, r: &ReviewRef, me: &str) -> PostResultOf<()> {
        let said = |messages: &[String]| clean(&messages.join("; "));
        let reply: GqlReply<GlSetReviewersData> = self
            .graphql(
                SET_REVIEWERS_APPEND,
                serde_json::json!({
                    "projectPath": r.repo,
                    "iid": r.number.to_string(),
                    "usernames": [me],
                }),
                GlWrite::Reviewer,
            )
            .map_err(|e| match e.kind {
                // GitLab's refusal, with its reason ("GitLab: …").
                PostErrorKind::PostRejected | PostErrorKind::NotFound => not_added(&format!(
                    "{}. {NEEDS_EDIT}",
                    e.message.trim_end_matches('.')
                )),
                _ => e,
            })?;
        if !reply.errors.is_empty() {
            let messages: Vec<String> = reply.errors.into_iter().map(|e| e.message).collect();
            return Err(not_added(&format!(
                "GitLab: {}. {NEEDS_EDIT}",
                said(&messages)
            )));
        }
        let Some(payload) = reply.data.and_then(|d| d.merge_request_set_reviewers) else {
            return Err(not_added("GitLab did not answer the change."));
        };
        if !payload.errors.is_empty() {
            return Err(not_added(&format!(
                "GitLab: {}. {NEEDS_EDIT}",
                said(&payload.errors)
            )));
        }
        // GitLab returns the MR as it now is: without the user in it, the
        // add did not happen (Free's one reviewer, or no permission).
        if let Some(mr) = payload.merge_request {
            let listed = mr.reviewers.is_none_or(|list| {
                list.nodes
                    .iter()
                    .flatten()
                    .any(|u| same_user(&u.username, me))
            });
            if !listed {
                return Err(not_added(&if mr.allows_multiple_reviewers == Some(false) {
                    "this GitLab allows only one reviewer per merge request and it already has one, whom Argus won't replace.".to_string()
                } else {
                    format!("GitLab left the reviewers unchanged. {NEEDS_EDIT}")
                }));
            }
        }
        Ok(())
    }

    /// The review's summary as an MR note, after its verdict was set: a
    /// failure says the verdict went through.
    fn post_summary(&self, r: &ReviewRef, text: &str) -> PostResultOf<()> {
        let posted: PostResultOf<GlCreatedNote> = self.post_json(
            &format!("{}/notes", Self::mr_path(r)),
            Some(&serde_json::json!({ "body": text })),
            GlWrite::Comment,
        );
        posted.map(|_| ()).map_err(|e| {
            PostError::new(
                e.kind,
                format!(
                    "Your review was submitted, but its summary was not posted: {}",
                    e.message
                ),
            )
        })
    }
}

/// How many of the newest notes are read for the review dates: one page.
const REVIEW_NOTE_PAGES: u32 = 1;

impl GitlabProvider {
    /// The MR's newest notes (system notes included), newest first.
    fn review_notes(&self, mr: &str) -> CoreResult<Vec<GlNote>> {
        self.get_pages(
            &format!("{mr}/notes?sort=desc&order_by=created_at"),
            REVIEW_NOTE_PAGES,
        )
    }
}

/// Date the user's review from the MR's notes: the newest "requested
/// review from" naming them, and their newest review verdict ("approved
/// this merge request", "requested changes"; for a `reviewed` or
/// `requested_changes` state without one, their newest comment), at the
/// diff version current then.
fn apply_review_notes(detail: &mut ReviewDetail, me: &str, notes: &[GlNote]) {
    detail.review_requested_at = notes
        .iter()
        .filter(|n| n.system)
        .filter(|n| {
            requested_reviewers(&n.body)
                .iter()
                .any(|u| same_user(u, me))
        })
        .map(|n| n.created_at)
        .max();
    let mine = notes.iter().filter(|n| same_user(&n.author.username, me));
    let verdict = mine
        .clone()
        .filter(|n| n.system && is_review_verdict(&n.body))
        .map(|n| n.created_at)
        .max();
    let dated = verdict.or_else(|| {
        detail
            .my_review_state
            .submitted_without_approval()
            .then(|| mine.filter(|n| !n.system).map(|n| n.created_at).max())
            .flatten()
    });
    detail.my_reviewed_at = dated;
    detail.my_reviewed_sha = dated.map(|at| head_at(detail, at));
}

/// A system note that records a review verdict of its author.
fn is_review_verdict(body: &str) -> bool {
    let b = body.trim().to_ascii_lowercase();
    b.starts_with("approved this merge request") || b == "requested changes"
}

/// The head the MR had at `at`: the newest diff version made by then (the
/// oldest when all are later, the current head without versions).
fn head_at(detail: &ReviewDetail, at: DateTime<Utc>) -> String {
    let made_by = detail
        .versions
        .iter()
        .rfind(|v| v.created_at.is_none_or(|c| c <= at));
    made_by
        .or(detail.versions.first())
        .map_or_else(|| detail.head_sha.clone(), |v| v.head_sha.clone())
}

/// The usernames a "requested review from @a, @b, and @c" system note
/// names (also "@a and @b", or one user; anything after the list, such as
/// "and removed review request for @d", is not part of it). Empty for any
/// other note.
pub(crate) fn requested_reviewers(body: &str) -> Vec<String> {
    let Some(rest) = body.trim().strip_prefix("requested review from ") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for word in rest.split_whitespace() {
        let word = word.trim_end_matches([',', '.']);
        if word == "and" {
            continue;
        }
        match word.strip_prefix('@') {
            Some(user) if !user.is_empty() => out.push(user.to_string()),
            _ => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_and_host() {
        assert_eq!(
            normalize_base_url(" https://gitlab.example.test/ ").as_deref(),
            Some("https://gitlab.example.test")
        );
        assert_eq!(normalize_base_url(""), None);
        // A bare host, as glab names it, is an https instance.
        assert_eq!(
            normalize_base_url(" gitlab.example.test ").as_deref(),
            Some("https://gitlab.example.test")
        );
        assert_eq!(
            normalize_base_url("gitlab.example.test:8443").as_deref(),
            Some("https://gitlab.example.test:8443")
        );
        assert_eq!(normalize_base_url("someone@gitlab.example.test"), None);
        assert_eq!(url_host("http://127.0.0.1:8080/x"), "127.0.0.1:8080");
        assert!(matches!(
            GitlabProvider::with_binary("glab", ""),
            Err(CoreError::GitlabNotConfigured)
        ));
        // Plain http names an instance too; other schemes do not.
        assert!(GitlabProvider::with_binary("glab", "http://gitlab.example.test").is_ok());
        assert!(GitlabProvider::with_binary("glab", "http://127.0.0.1:9").is_ok());
        assert!(GitlabProvider::with_binary("glab", "ftp://gitlab.example.test").is_err());
        let p = GitlabProvider::with_binary("glab", "https://GitLab.example.test:8443/").unwrap();
        assert_eq!(p.host(), "gitlab.example.test:8443");
    }

    #[test]
    fn userinfo_never_passes_for_a_local_host() {
        // The part before `@` is userinfo, not the host: refused whatever
        // the scheme, so a local-looking prefix never hides the real host.
        for url in [
            "http://127.0.0.1:80@gitlab.example.test",
            "http://localhost@gitlab.example.test",
            "http://[::1]@gitlab.example.test",
            "http://[::1]:80@gitlab.example.test/api",
            "http://localhost:x@gitlab.example.test",
            // Userinfo has no place in the instance URL at all.
            "https://someone@gitlab.example.test",
            "http://someone@localhost:8080",
            "http://someone:pw@10.0.0.5",
            // Query and fragment have no place in an instance URL.
            "http://10.0.0.5/?a=b",
            "https://gitlab.example.test#x",
            // Not a URL a GitLab can be at.
            "http://localhost:99999",
            "https://",
            "ftp://gitlab.example.test",
            "ssh://gitlab.example.test",
        ] {
            assert_eq!(normalize_base_url(url), None, "{url}");
        }
        for (url, host) in [
            ("http://localhost:8080/", "localhost:8080"),
            ("http://127.0.0.1:3000", "127.0.0.1:3000"),
            ("http://[::1]:8080", "[::1]:8080"),
            ("https://gitlab.example.test/sub", "gitlab.example.test"),
            // Plain http on a VPN / LAN host: accepted (the scheme is only
            // for web links; how glab connects is its own login's).
            ("http://GitLab.vpn.test:8080/", "gitlab.vpn.test:8080"),
            ("http://10.0.0.5", "10.0.0.5"),
            ("http://gitlab.example.test:80", "gitlab.example.test"),
        ] {
            let ok = normalize_base_url(url).unwrap_or_else(|| panic!("{url}"));
            assert_eq!(url_host(&ok), host);
        }
    }

    #[test]
    fn redacts_anything_shaped_like_a_gitlab_token() {
        let pat = "glpat-fixtureTOKENnotreal.01.abcdef";
        let oauth = "f".repeat(64);
        for (text, kept) in [
            (
                format!("401 Unauthorized {pat}"),
                "401 Unauthorized [redacted]",
            ),
            (format!("({pat}) bad"), "([redacted]) bad"),
            (format!("{pat}{pat} twice"), "[redacted] twice"),
            (
                format!("see https://x.test/?access_token={pat}&x=1"),
                "see https://x.test/?access_token=[redacted]&x=1",
            ),
            (
                format!("Authorization: Bearer {oauth}"),
                "Authorization: Bearer [redacted]",
            ),
            (
                "PRIVATE-TOKEN: s3cr3t-value".to_string(),
                "PRIVATE-TOKEN: [redacted]",
            ),
            (
                "private_token=abc123".to_string(),
                "private_token=[redacted]",
            ),
            (format!("token {oauth}"), "token [redacted]"),
            (
                "deploy gldt-AbC_123 and glrt-x".to_string(),
                "deploy [redacted] and [redacted]",
            ),
            (format!("x-{pat}"), "x-[redacted]"),
        ] {
            assert_eq!(redact_tokens(&text), kept, "{text}");
        }
        // What is not a token stays: a commit sha (40 hex), plain words,
        // GitLab's messages.
        let sha = "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2";
        let msg = format!("SHA does not match HEAD of source branch: {sha}");
        assert_eq!(redact_tokens(&msg), msg);
        assert_eq!(
            redact_tokens("global glob-pattern ok"),
            "global glob-pattern ok"
        );
        assert_eq!(redact_tokens("ünïcode — fine"), "ünïcode — fine");
        assert_eq!(clean("line\n two"), "line two");
        assert_eq!(clean("a\r\nb {glpat-x1}"), "a b {[redacted]}");
    }

    #[test]
    fn classifies_what_glab_says_without_a_response() {
        for (stderr, failure) in [
            (
                "x gitlab.example.test has not been authenticated with glab; run `glab auth login --hostname gitlab.example.test`",
                Failure::Auth,
            ),
            ("glab: 401 Unauthorized (HTTP 401)", Failure::Auth),
            (
                "Get \"https://gitlab.example.test/api/v4/user\": dial tcp: lookup gitlab.example.test: no such host",
                Failure::NotConnected,
            ),
            (
                "Post \"http://127.0.0.1:9/api/v4/x\": dial tcp 127.0.0.1:9: connect: connection refused",
                Failure::NotConnected,
            ),
            ("read: connection reset by peer", Failure::Broken),
            ("tls: handshake failure", Failure::Broken),
            ("something else entirely", Failure::Other),
        ] {
            assert_eq!(Failure::from_stderr(stderr.as_bytes()), failure, "{stderr}");
        }
        let e = Failure::Auth.into_error("gitlab.example.test");
        assert!(e
            .to_string()
            .contains("glab auth login --hostname gitlab.example.test"));
        assert!(matches!(
            Failure::Missing.into_error("h"),
            CoreError::GlabMissing
        ));
    }

    #[test]
    fn reviewer_states_map() {
        assert_eq!(reviewer_state("unreviewed"), ReviewerState::Requested);
        assert_eq!(reviewer_state("approved"), ReviewerState::Approved);
        assert_eq!(
            reviewer_state("requested_changes"),
            ReviewerState::ChangesRequested
        );
        assert_eq!(reviewer_state("reviewed"), ReviewerState::Commented);
    }

    // --- Review state ---

    #[test]
    fn reads_who_a_review_request_names() {
        assert_eq!(
            requested_reviewers("requested review from @a, @b, and @c"),
            ["a", "b", "c"]
        );
        assert_eq!(
            requested_reviewers("requested review from @a and @b"),
            ["a", "b"]
        );
        assert_eq!(
            requested_reviewers("requested review from @rev-fixture"),
            ["rev-fixture"]
        );
        // What follows the list is not part of it.
        assert_eq!(
            requested_reviewers("requested review from @a and removed review request for @b"),
            ["a"]
        );
        assert!(requested_reviewers("removed review request for @a").is_empty());
        assert!(requested_reviewers("approved this merge request").is_empty());
        assert!(requested_reviewers("requested review from").is_empty());
    }

    #[test]
    fn maps_the_users_reviewer_entry() {
        use MyReviewState::*;
        let state = |entry, listed, approved| my_review_state(entry, listed, approved);
        assert_eq!(state(Some("unreviewed"), false, false), AwaitingReview);
        assert_eq!(state(Some("review_started"), false, false), ReviewStarted);
        assert_eq!(
            state(Some("requested_changes"), false, false),
            RequestedChanges
        );
        assert_eq!(state(Some("reviewed"), false, false), Commented);
        assert_eq!(state(Some("unapproved"), false, false), Commented);
        assert_eq!(state(Some("approved"), false, true), Approved);
        assert_eq!(state(None, false, true), Approved);
        // No `/reviewers` endpoint: listed counts as owed.
        assert_eq!(state(None, true, false), AwaitingReview);
        assert_eq!(state(None, false, false), NotReviewer);
    }

    #[test]
    fn parses_gitlab_versions() {
        assert_eq!(parse_gitlab_version("17.5.1-ee"), Some((17, 5)));
        assert_eq!(parse_gitlab_version("16.11.0"), Some((16, 11)));
        assert_eq!(parse_gitlab_version("18.0.0-pre"), Some((18, 0)));
        assert_eq!(parse_gitlab_version("nightly"), None);
        assert!(parse_gitlab_version("16.11.10-ee").unwrap() < REQUEST_CHANGES_SINCE);
        assert!(parse_gitlab_version("17.2.0").unwrap() >= REQUEST_CHANGES_SINCE);
    }

    fn gl_note(id: u64, author: &str, at: &str, system: bool, body: &str) -> GlNote {
        serde_json::from_value(serde_json::json!({
            "id": id, "body": body, "author": { "username": author },
            "created_at": at, "system": system
        }))
        .unwrap()
    }

    fn mr_detail(state: MyReviewState) -> ReviewDetail {
        let version = |head: &str, at: &str| DiffVersion {
            head_sha: head.into(),
            base_sha: "a0".into(),
            start_sha: "a0".into(),
            created_at: Some(at.parse().unwrap()),
        };
        ReviewDetail {
            review: ReviewRef {
                provider: ProviderId::Gitlab,
                host: "gitlab.example.test".into(),
                repo: "grp/app".into(),
                number: 21,
                url: String::new(),
                title: String::new(),
                author: "alice-fixture".into(),
                updated_at: None,
            },
            description: String::new(),
            state: ReviewState::Open,
            draft: false,
            head_sha: "a3".into(),
            base_sha: "a0".into(),
            start_sha: None,
            commits: Vec::new(),
            versions: vec![
                version("a1", "2026-10-08T09:00:00Z"),
                version("a2", "2026-10-09T09:00:00Z"),
                version("a3", "2026-10-11T09:00:00Z"),
            ],
            reviewers: Vec::new(),
            requested_from_me: state.owes_review(),
            approved_by_me: false,
            my_review_state: state,
            my_reviewed_at: None,
            my_reviewed_sha: None,
            review_requested_at: None,
        }
    }

    #[test]
    fn the_notes_date_the_users_review_and_the_latest_request() {
        let me = "rev-fixture";
        // Newest first, as GitLab sorts them for Argus.
        let notes = [
            gl_note(
                9,
                "alice-fixture",
                "2026-10-11T10:00:00Z",
                true,
                "requested review from @rev-fixture",
            ),
            gl_note(
                8,
                "alice-fixture",
                "2026-10-11T09:00:00Z",
                true,
                "added 1 commit",
            ),
            gl_note(7, me, "2026-10-10T12:00:00Z", true, "requested changes"),
            gl_note(6, me, "2026-10-10T11:00:00Z", false, "Why three retries?"),
            gl_note(
                5,
                "dana-fixture",
                "2026-10-10T10:00:00Z",
                true,
                "approved this merge request",
            ),
            gl_note(
                4,
                "alice-fixture",
                "2026-10-08T10:00:00Z",
                true,
                "requested review from @dana-fixture, @erin-fixture, and @Rev-Fixture",
            ),
        ];
        let mut d = mr_detail(MyReviewState::AwaitingReview);
        apply_review_notes(&mut d, me, &notes);
        assert_eq!(
            d.review_requested_at,
            Some("2026-10-11T10:00:00Z".parse().unwrap())
        );
        assert_eq!(
            d.my_reviewed_at,
            Some("2026-10-10T12:00:00Z".parse().unwrap())
        );
        // The version current at 10-10 12:00 was a2.
        assert_eq!(d.my_reviewed_sha.as_deref(), Some("a2"));

        // No verdict note of the user's: a `reviewed` state is dated by
        // their newest comment; an owed one is not dated at all.
        let quiet = &notes[..2];
        let comments = [gl_note(
            6,
            me,
            "2026-10-10T11:00:00Z",
            false,
            "Why three retries?",
        )];
        let mut d = mr_detail(MyReviewState::Commented);
        apply_review_notes(&mut d, me, &comments);
        assert_eq!(
            d.my_reviewed_at,
            Some("2026-10-10T11:00:00Z".parse().unwrap())
        );
        let mut d = mr_detail(MyReviewState::AwaitingReview);
        apply_review_notes(&mut d, me, &comments);
        assert_eq!(d.my_reviewed_at, None);
        let mut d = mr_detail(MyReviewState::AwaitingReview);
        apply_review_notes(&mut d, me, quiet);
        assert_eq!(d.my_reviewed_at, None);
        assert!(d.review_requested_at.is_some());
    }
}
