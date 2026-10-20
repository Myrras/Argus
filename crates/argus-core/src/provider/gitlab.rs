//! GitLab, over its REST API with a personal access token.
//!
//! The token lives in the OS keychain (service `com.argus.app.gitlab`, the
//! account is the instance host, `http://host` for plain http off this
//! machine), read on demand, never logged and never
//! written to settings. A [`TokenSource`] seam lets tests inject one.
//!
//! The HTTP client is blocking and never follows redirects, as in Podium: a
//! redirect would carry the `PRIVATE-TOKEN` header to whatever host it names.
//! Errors are Argus-owned text, never reqwest's (which holds the URL).
//!
//! Every GET is conditional: the last `ETag` (or `Last-Modified`) of the URL
//! goes out as `If-None-Match` (`If-Modified-Since`) and a 304 replays the
//! cached body and `X-Next-Page`. A 429 stops all calls until GitLab's
//! `Retry-After` / `RateLimit-Reset`. Requests time out (connect and total).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::conditional::{Cached, ConditionalCache};
use super::github::check_sha;
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
use crate::post::{
    sanitize, DiffLine, LineKind, PlacedComment, PostError, PostErrorKind, PostedComment,
    PostedNote,
};

const KEYCHAIN_SERVICE: &str = "com.argus.app.gitlab";

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

/// Where the GitLab token comes from.
pub trait TokenSource: Send + Sync {
    /// The token, or `None` when none is set.
    fn token(&self) -> Option<String>;
}

/// A fixed token (tests, or a caller that already holds one).
pub struct StaticToken(pub String);

impl TokenSource for StaticToken {
    fn token(&self) -> Option<String> {
        Some(self.0.clone()).filter(|t| !t.is_empty())
    }
}

/// The token in the OS keychain for one GitLab instance.
pub struct KeychainToken {
    base_url: String,
}

impl KeychainToken {
    /// The keychain entry for the instance at `base_url`. Keyed by host, so a
    /// token never goes to a different instance after the URL changes; the
    /// host as the provider talks to it, and apart for plain http (see
    /// [`TokenKeys`]).
    pub fn for_base_url(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
        }
    }
}

impl TokenSource for KeychainToken {
    fn token(&self) -> Option<String> {
        read_token(&Keychain, &self.base_url)
    }
}

/// Where tokens are kept, by account (the instance host): the OS keychain,
/// or memory in tests.
trait SecretStore {
    /// `Ok(None)` when there is no such entry; `Err` when the store could
    /// not be read (a locked keychain, a denied prompt).
    fn get(&self, account: &str) -> CoreResult<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> CoreResult<()>;
    fn delete(&self, account: &str) -> CoreResult<()>;
}

/// The OS keychain, service [`KEYCHAIN_SERVICE`].
struct Keychain;

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> CoreResult<Option<String>> {
        match keychain_entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CoreError::Provider("could not read the keychain".into())),
        }
    }
    fn set(&self, account: &str, secret: &str) -> CoreResult<()> {
        keychain_entry(account)?
            .set_password(secret)
            .map_err(|_| CoreError::Provider("could not update the keychain".into()))
    }
    fn delete(&self, account: &str) -> CoreResult<()> {
        match keychain_entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(CoreError::Provider("could not update the keychain".into())),
        }
    }
}

fn keychain_entry(host: &str) -> CoreResult<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, host)
        .map_err(|_| CoreError::Provider("could not open the keychain".into()))
}

/// The keychain accounts of the instance at `base_url`.
struct TokenKeys {
    /// Where its token is: the host of its normalized URL
    /// ([`normalize_base_url`]: lowercased, default port dropped), as the
    /// provider talks to it, so every spelling of one instance finds one
    /// entry; `http://host` for plain http off this machine
    /// ([`is_insecure`]), so a token stored for https never goes out
    /// unencrypted because the URL changed to http.
    key: String,
    /// Where a token for it may still be, read once and moved to `key`
    /// (see [`read_token`]): the raw URL's host Argus used before, and for
    /// https the same instance's plain-http token (going to https is
    /// safe). Never any for plain http (see [`migrate_plain_http_token`]).
    fallbacks: Vec<String>,
}

fn token_keys(base_url: &str) -> TokenKeys {
    let raw = url_host(base_url.trim()).to_string();
    let Some(base) = normalize_base_url(base_url) else {
        return TokenKeys {
            key: raw,
            fallbacks: Vec::new(),
        };
    };
    let host = url_host(&base).to_string();
    if is_insecure(&base) {
        return TokenKeys {
            key: format!("http://{host}"),
            fallbacks: Vec::new(),
        };
    }
    let mut fallbacks = Vec::new();
    if raw != host && !raw.is_empty() {
        fallbacks.push(raw);
    }
    fallbacks.push(format!("http://{host}"));
    TokenKeys {
        key: host,
        fallbacks,
    }
}

fn nonblank(token: Option<String>) -> Option<String> {
    token.filter(|t| !t.is_empty())
}

/// What a read of `account` finds, a keychain that cannot be read taken as
/// nothing there: right for using a token (no token is what the user then
/// sees), never for deciding a move happened.
fn found(store: &dyn SecretStore, account: &str) -> Option<String> {
    nonblank(store.get(account).ok().flatten())
}

/// Delete an entry a token was moved from or that a save replaces. A
/// failure is logged, not returned: the token itself is where it belongs,
/// and the leftover only matters as a fallback.
fn delete_leftover(store: &dyn SecretStore, account: &str) {
    if store.delete(account).is_err() {
        tracing::warn!("could not remove an old GitLab token entry from the keychain");
    }
}

/// The token for `base_url`; one still under a fallback account is moved to
/// the current one on the way.
fn read_token(store: &dyn SecretStore, base_url: &str) -> Option<String> {
    let TokenKeys { key, fallbacks } = token_keys(base_url);
    if let Some(token) = found(store, &key) {
        return Some(token);
    }
    for old in fallbacks {
        let Some(token) = found(store, &old) else {
            continue;
        };
        // The old entry goes only once the new one holds the token.
        if store.set(&key, &token).is_ok() {
            delete_leftover(store, &old);
        }
        return Some(token);
    }
    None
}

/// Store `token` for `base_url`; a blank one deletes it. The fallback
/// entries go either way: they would otherwise come back as the fallback.
/// Only storing (or deleting) the token itself can fail the save.
fn write_token(store: &dyn SecretStore, base_url: &str, token: &str) -> CoreResult<()> {
    let TokenKeys { key, fallbacks } = token_keys(base_url);
    let token = token.trim();
    if token.is_empty() {
        store.delete(&key)?;
    } else {
        store.set(&key, token)?;
    }
    for old in fallbacks {
        delete_leftover(store, &old);
    }
    Ok(())
}

/// Once, on the first start of a version that keys plain-http tokens apart
/// (see [`TokenKeys::key`]): a plain-http instance configured before kept
/// its token under its host. The user set it up that way on purpose and it
/// works, so the token moves to the instance's http account instead of
/// being asked for again. Anything else is left alone: an https instance,
/// or one that already has an http token (moved before).
///
/// `Ok` when there is nothing (left) to move, so the caller records it done
/// and never runs it again: a later switch from https to http must find no
/// token. `Err` when the keychain could not be read or written (anything
/// but "no such entry"): nothing is recorded and the next start tries
/// again. The host entry goes only once the http one holds the token.
fn migrate_plain_http_token(store: &dyn SecretStore, base_url: &str) -> CoreResult<()> {
    if !is_insecure(base_url) {
        return Ok(());
    }
    let Some(base) = normalize_base_url(base_url) else {
        return Ok(());
    };
    let key = token_keys(&base).key;
    if nonblank(store.get(&key)?).is_some() {
        return Ok(());
    }
    let host = url_host(&base).to_string();
    let raw = url_host(base_url.trim()).to_string();
    let mut old = vec![host];
    if !raw.is_empty() && !old.contains(&raw) {
        old.push(raw);
    }
    for account in &old {
        if let Some(token) = nonblank(store.get(account)?) {
            store.set(&key, &token)?;
            delete_leftover(store, account);
            return Ok(());
        }
    }
    Ok(())
}

/// [`migrate_plain_http_token`] on the OS keychain. Touches the keychain
/// only for a plain-http instance URL.
pub fn gitlab_migrate_plain_http_token(base_url: &str) -> CoreResult<()> {
    migrate_plain_http_token(&Keychain, base_url)
}

/// Store the token for the instance at `base_url`; a blank token deletes it.
pub fn gitlab_token_set(base_url: &str, token: &str) -> CoreResult<()> {
    write_token(&Keychain, base_url, token)
}

/// Whether a token is stored for `base_url` (the token never leaves here).
pub fn gitlab_token_present(base_url: &str) -> bool {
    KeychainToken::for_base_url(base_url).token().is_some()
}

/// `https://host[:port]/path` → `host[:port]`.
pub fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or("")
}

/// Trim, parse and drop a trailing slash; `None` when not an http(s) URL
/// of an instance. Plain http is allowed for any host (a self-hosted GitLab
/// behind a VPN has no TLS): the token then travels unencrypted, which
/// [`is_insecure`] reports so the UI can say so.
///
/// The host is the parsed one, what a request really connects to: in
/// `http://127.0.0.1:80@gitlab.example.test` the part before `@` is
/// userinfo, not the host. Userinfo, a query or a fragment have no place in
/// an instance URL and are refused; the URL comes back in its parsed form,
/// so [`url_host`] reads the same host.
pub fn normalize_base_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
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

/// Whether requests to `url` send the token unencrypted: plain http to a
/// host that is not this machine. Loopback http (`localhost`, `127.0.0.1`,
/// `[::1]`) never leaves the machine. `false` for an unusable URL.
pub fn is_insecure(url: &str) -> bool {
    let Some(base) = normalize_base_url(url) else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(&base) else {
        return false;
    };
    parsed.scheme() == "http" && !parsed.host_str().is_some_and(is_local)
}

/// A parsed host (no port; IPv6 in brackets) names this machine.
fn is_local(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
}

/// The longest one request may take, start to end of body.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest a connection may take to open (a VPN that is down).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn build_client(timeout: Duration) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(CONNECT_TIMEOUT.min(timeout))
        .build()
        // Same failure mode as `Client::new()`: no TLS backend at all.
        .expect("GitLab HTTP client")
}

/// GitLab for the token's owner on one instance.
pub struct GitlabProvider {
    base: String,
    host: String,
    tokens: Box<dyn TokenSource>,
    me: Mutex<Option<GlUser>>,
    /// Per provider (it keeps its connections alive between polls).
    client: reqwest::blocking::Client,
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
}

impl GitlabProvider {
    /// `base_url` is the instance root (`https://gitlab.example.com`). An
    /// empty or non-http(s) URL is [`CoreError::GitlabNotConfigured`].
    pub fn new(base_url: &str, tokens: impl TokenSource + 'static) -> CoreResult<Self> {
        let base = normalize_base_url(base_url).ok_or(CoreError::GitlabNotConfigured)?;
        Ok(Self {
            host: url_host(&base).to_string(),
            base,
            tokens: Box::new(tokens),
            me: Mutex::new(None),
            client: build_client(REQUEST_TIMEOUT),
            cache: ConditionalCache::default(),
            limit: Backoff::default(),
            done: Mutex::new(HashMap::new()),
            final_recheck: FINAL_RECHECK,
            commented: Mutex::new(HashMap::new()),
        })
    }

    /// Read a commented-on MR seen merged or closed again after `after`
    /// instead of [`FINAL_RECHECK`] (tests).
    pub fn with_final_recheck(mut self, after: chrono::Duration) -> Self {
        self.final_recheck = after;
        self
    }

    /// Give up on a request after `timeout` instead of the default 30 s.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client = build_client(timeout);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// Send a GET (with `validator` as a conditional header), retrying
    /// connection-level failures (VPN or Wi-Fi coming back after sleep)
    /// twice with a short backoff, as Podium does. A response with any
    /// status is never retried, and neither is a timeout: a server that hung
    /// for the whole timeout would only hang again.
    fn send(
        &self,
        url: &str,
        token: &str,
        validator: Option<(&str, &str)>,
    ) -> CoreResult<reqwest::blocking::Response> {
        let mut last = "connection failed";
        for delay_ms in [0, 500, 1500] {
            std::thread::sleep(Duration::from_millis(delay_ms));
            let mut req = self.client.get(url).header("PRIVATE-TOKEN", token);
            if let Some((name, value)) = validator {
                req = req.header(name, value);
            }
            match req.send() {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::debug!("GitLab request failed: {e}");
                    if e.is_timeout() {
                        last = "timed out";
                        break;
                    }
                    last = "connection failed";
                }
            }
        }
        Err(CoreError::Provider(format!(
            "could not reach GitLab ({last})"
        )))
    }

    /// One conditional GET of `path` (under `/api/v4`): the body (from the
    /// cache on a 304) and the next page, if the `X-Next-Page` header names
    /// one.
    fn get(&self, path: &str) -> CoreResult<(String, Option<u32>)> {
        self.limit.check(ProviderId::Gitlab)?;
        let token = self.tokens.token().ok_or(CoreError::GitlabNotConfigured)?;
        let cached = self.cache.get(path);
        let response = self.send(
            &format!("{}/api/v4{path}", self.base),
            &token,
            cached.as_deref().and_then(Cached::validator),
        )?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim().to_string())
        };
        let parse_next = |v: Option<&str>| v.and_then(|v| v.trim().parse().ok());
        match response.status().as_u16() {
            200 => {
                let next_raw = header("x-next-page").filter(|v| !v.is_empty());
                let etag = header("etag");
                let last_modified = header("last-modified");
                let body = response
                    .text()
                    .map_err(|_| CoreError::Provider("could not read GitLab's response".into()))?;
                let next = parse_next(next_raw.as_deref());
                self.cache.put(
                    path,
                    Cached {
                        etag,
                        last_modified,
                        body: body.clone(),
                        next: next_raw,
                    },
                );
                Ok((body, next))
            }
            // Unchanged: the cached response stands.
            304 => cached
                .map(|c| (c.body.clone(), parse_next(c.next.as_deref())))
                .ok_or_else(unexpected),
            429 => Err(self
                .limit
                .hit(ProviderId::Gitlab, reset_at(header, Utc::now()))),
            300..=399 => Err(CoreError::Provider(
                "GitLab redirected the request; check the URL in Settings (https, host and path)"
                    .into(),
            )),
            401 | 403 => Err(CoreError::GitlabAuth),
            404 => Err(CoreError::NotFound),
            422 => Err(super::unprocessable("GitLab")),
            _ => Err(CoreError::Provider(
                "GitLab could not fetch the merge request".into(),
            )),
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
            let (diffs, truncated, refs) = match self.get_all::<GlDiff>(&format!("{p}/diffs")) {
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
            Err(CoreError::NotFound | CoreError::GitlabAuth) => None,
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
        let token = self.tokens.token().ok_or(CoreError::GitlabNotConfigured)?;
        let url = format!("{}/api/v4{path}", self.base);
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let mut response = None;
        for delay_ms in [0, 500, 1500] {
            std::thread::sleep(Duration::from_millis(delay_ms));
            let mut req = self.client.post(&url).header("PRIVATE-TOKEN", &token);
            if body.is_some() {
                req = req
                    .header("Content-Type", "application/json")
                    .body(payload.clone());
            }
            match req.send() {
                Ok(r) => {
                    response = Some(r);
                    break;
                }
                Err(e) if e.is_connect() => {
                    tracing::debug!("GitLab write could not connect: {e}");
                }
                Err(e) => {
                    tracing::debug!("GitLab write failed: {e}");
                    return Err(PostError::new(
                        PostErrorKind::Provider,
                        "GitLab did not answer; refresh to see whether it went through",
                    ));
                }
            }
        }
        let Some(response) = response else {
            return Err(
                CoreError::Provider("could not reach GitLab (connection failed)".into()).into(),
            );
        };
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        if (200..300).contains(&status) {
            return serde_json::from_str(&text).map_err(|_| unexpected().into());
        }
        let reason = gitlab_error_message(&text).map(|m| sanitize(&m, &[&token]));
        Err(write_error(status, reason, what))
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
/// (sanitized), forwarded so the user sees why.
fn write_error(status: u16, reason: Option<String>, what: GlWrite) -> PostError {
    let with = |fallback: &str| {
        format!(
            "GitLab: {}",
            reason.clone().unwrap_or_else(|| fallback.to_string())
        )
    };
    match status {
        300..=399 => CoreError::Provider(
            "GitLab redirected the request; check the URL in Settings (https, host and path)"
                .into(),
        )
        .into(),
        // Approving answers 401 when the user may not approve (already
        // approved, own MR, not eligible), not only for a bad token.
        401 if what == GlWrite::Approve => PostError::new(
            PostErrorKind::PostRejected,
            with("401 Unauthorized")
                + " (already approved, or you may not approve this merge request)",
        ),
        401 => CoreError::GitlabAuth.into(),
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

impl GitlabProvider {
    /// Set the user's reviewer state to "requested changes" with the
    /// `/request_changes` quick action: the one way GitLab's API offers
    /// before 19.x (`draft_notes/bulk_publish` with `reviewer_state` came
    /// later, and the GraphQL `mergeRequestRequestChanges` later still).
    /// Checked first (reads only): the instance is 17.2 or later and the
    /// user is a reviewer of the MR (the quick action exists only for one);
    /// checked after: GitLab ran it as a command (no note saved) and the
    /// user's state is now `requested_changes`.
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
            return Err(PostError::new(
                PostErrorKind::PostRejected,
                "GitLab only takes a change request from a reviewer of the merge request; ask to be added as one.",
            ));
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
            let token = self.tokens.token().unwrap_or_default();
            let said = sanitize(&answer.summary.join(" "), &[&token]);
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
        assert_eq!(normalize_base_url("gitlab.example.test"), None);
        assert_eq!(url_host("http://127.0.0.1:8080/x"), "127.0.0.1:8080");
        assert!(matches!(
            GitlabProvider::new("", StaticToken("t".into())),
            Err(CoreError::GitlabNotConfigured)
        ));
        // Plain http is fine on any host (a VPN); other schemes are not.
        assert!(GitlabProvider::new("http://gitlab.example.test", StaticToken("t".into())).is_ok());
        assert!(GitlabProvider::new("http://127.0.0.1:9", StaticToken("t".into())).is_ok());
        assert!(GitlabProvider::new("ftp://gitlab.example.test", StaticToken("t".into())).is_err());
    }

    /// An in-memory keychain: account → secret.
    #[derive(Default)]
    struct MemStore(Mutex<HashMap<String, String>>);

    impl SecretStore for MemStore {
        fn get(&self, account: &str) -> CoreResult<Option<String>> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }
        fn set(&self, account: &str, secret: &str) -> CoreResult<()> {
            self.0.lock().unwrap().insert(account.into(), secret.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> CoreResult<()> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    const FIXTURE_TOKEN: &str = "glpat-fixture-not-a-real-token";

    #[test]
    fn the_token_is_keyed_by_the_host_the_provider_talks_to() {
        // Any spelling of one instance finds the same entry.
        let store = MemStore::default();
        write_token(&store, "https://GitLab.example.test:443/", FIXTURE_TOKEN).unwrap();
        for url in [
            "https://gitlab.example.test",
            "https://GitLab.example.test",
            "https://gitlab.example.test:443",
        ] {
            assert_eq!(
                read_token(&store, url).as_deref(),
                Some(FIXTURE_TOKEN),
                "{url}"
            );
        }
        assert_eq!(
            store.0.lock().unwrap().keys().collect::<Vec<_>>(),
            ["gitlab.example.test"]
        );
        // Another instance does not.
        assert_eq!(read_token(&store, "https://gitlab.other.test"), None);
    }

    #[test]
    fn a_token_under_the_old_raw_host_key_is_read_once_and_moved() {
        let store = MemStore::default();
        store.set("GitLab.example.test:443", FIXTURE_TOKEN).unwrap();
        let url = "https://GitLab.example.test:443";
        assert_eq!(read_token(&store, url).as_deref(), Some(FIXTURE_TOKEN));
        let keys: Vec<String> = store.0.lock().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["gitlab.example.test"]);
        // Saved settings hold the normalized URL from then on.
        assert_eq!(
            read_token(&store, "https://gitlab.example.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
    }

    #[test]
    fn clearing_the_token_also_clears_the_old_entry() {
        let store = MemStore::default();
        store.set("GitLab.example.test:443", FIXTURE_TOKEN).unwrap();
        write_token(&store, "https://GitLab.example.test:443", " ").unwrap();
        assert!(store.0.lock().unwrap().is_empty());
        assert_eq!(read_token(&store, "https://GitLab.example.test:443"), None);
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
            // Plain http on a VPN / LAN host: accepted, host kept for the
            // keychain key.
            ("http://GitLab.vpn.test:8080/", "gitlab.vpn.test:8080"),
            ("http://10.0.0.5", "10.0.0.5"),
            ("http://gitlab.example.test:80", "gitlab.example.test"),
        ] {
            let ok = normalize_base_url(url).unwrap_or_else(|| panic!("{url}"));
            assert_eq!(url_host(&ok), host);
        }
    }

    #[test]
    fn only_plain_http_off_this_machine_is_insecure() {
        for url in [
            "http://gitlab.example.test",
            "http://10.0.0.5:8080/",
            "http://localhost.example.test",
            "http://[fd00::1]",
        ] {
            assert!(is_insecure(url), "{url}");
        }
        for url in [
            "https://gitlab.example.test",
            "https://10.0.0.5",
            "http://localhost:8080",
            "http://127.0.0.1:3000/",
            "http://[::1]:8080",
            // Unusable URLs are a problem, not an insecure connection.
            "",
            "gitlab.example.test",
            "http://someone@10.0.0.5",
        ] {
            assert!(!is_insecure(url), "{url}");
        }
    }

    #[test]
    fn a_plain_http_instance_has_a_token_of_its_own() {
        let store = MemStore::default();
        write_token(&store, "http://GitLab.vpn.test:80/", FIXTURE_TOKEN).unwrap();
        assert_eq!(
            read_token(&store, "http://gitlab.vpn.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
        assert_eq!(keys(&store), ["http://gitlab.vpn.test"]);
        // Loopback http never leaves the machine: keyed as https is.
        write_token(&store, "http://localhost:8080", FIXTURE_TOKEN).unwrap();
        assert!(keys(&store).contains(&"localhost:8080".to_string()));
    }

    #[test]
    fn switching_an_https_instance_to_http_finds_no_token() {
        // The token was stored for https: it must not go out unencrypted
        // because the URL changed.
        let store = MemStore::default();
        write_token(&store, "https://gitlab.example.test", FIXTURE_TOKEN).unwrap();
        assert_eq!(read_token(&store, "http://gitlab.example.test"), None);
        assert_eq!(read_token(&store, "http://GitLab.example.test:80/"), None);
        assert_eq!(keys(&store), ["gitlab.example.test"]);
        // A token entered for http is its own, and the https one stays.
        write_token(&store, "http://gitlab.example.test", "glpat-fixture-http").unwrap();
        assert_eq!(
            read_token(&store, "https://gitlab.example.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
    }

    #[test]
    fn going_from_http_to_https_keeps_the_token() {
        let store = MemStore::default();
        write_token(&store, "http://gitlab.vpn.test", FIXTURE_TOKEN).unwrap();
        assert_eq!(
            read_token(&store, "https://gitlab.vpn.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
        // Moved to the https account on the way.
        assert_eq!(keys(&store), ["gitlab.vpn.test"]);
    }

    #[test]
    fn the_one_time_move_keeps_a_working_plain_http_setup() {
        // Before tokens had a key of their own for plain http, the token of
        // a deliberately configured http instance was under its host.
        let store = MemStore::default();
        store.set("gitlab.vpn.test", FIXTURE_TOKEN).unwrap();
        migrate_plain_http_token(&store, "http://gitlab.vpn.test").unwrap();
        assert_eq!(keys(&store), ["http://gitlab.vpn.test"]);
        assert_eq!(
            read_token(&store, "http://gitlab.vpn.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
        // An old raw-host spelling moves too.
        let store = MemStore::default();
        store.set("GitLab.vpn.test:80", FIXTURE_TOKEN).unwrap();
        migrate_plain_http_token(&store, "http://GitLab.vpn.test:80").unwrap();
        assert_eq!(keys(&store), ["http://gitlab.vpn.test"]);
        // An https instance, or one with an http token already, is left as is.
        let store = MemStore::default();
        store.set("gitlab.example.test", FIXTURE_TOKEN).unwrap();
        migrate_plain_http_token(&store, "https://gitlab.example.test").unwrap();
        migrate_plain_http_token(&store, "http://localhost").unwrap();
        assert_eq!(keys(&store), ["gitlab.example.test"]);
        store
            .set("http://gitlab.example.test", "glpat-fixture-http")
            .unwrap();
        migrate_plain_http_token(&store, "http://gitlab.example.test").unwrap();
        assert_eq!(
            store.get("http://gitlab.example.test").unwrap().as_deref(),
            Some("glpat-fixture-http")
        );
        assert_eq!(keys(&store).len(), 2);
    }

    /// A keychain whose reads or writes fail (a locked keychain, a denied
    /// prompt): not the same as an entry that is not there.
    #[derive(Default)]
    struct Failing {
        inner: MemStore,
        get: bool,
        set: bool,
    }

    impl SecretStore for Failing {
        fn get(&self, account: &str) -> CoreResult<Option<String>> {
            if self.get {
                return Err(CoreError::Provider("could not read the keychain".into()));
            }
            self.inner.get(account)
        }
        fn set(&self, account: &str, secret: &str) -> CoreResult<()> {
            if self.set {
                return Err(CoreError::Provider("could not update the keychain".into()));
            }
            self.inner.set(account, secret)
        }
        fn delete(&self, account: &str) -> CoreResult<()> {
            self.inner.delete(account)
        }
    }

    #[test]
    fn a_failed_one_time_move_fails_and_keeps_the_token_where_it_was() {
        // A keychain that cannot be read: the move did not happen, so it
        // must not be recorded as done (the caller retries next start).
        let store = Failing {
            get: true,
            ..Failing::default()
        };
        store.inner.set("gitlab.vpn.test", FIXTURE_TOKEN).unwrap();
        assert!(migrate_plain_http_token(&store, "http://gitlab.vpn.test").is_err());
        assert_eq!(keys(&store.inner), ["gitlab.vpn.test"]);
        // The http entry cannot be written: the old one is never deleted.
        let store = Failing {
            set: true,
            ..Failing::default()
        };
        store.inner.set("gitlab.vpn.test", FIXTURE_TOKEN).unwrap();
        assert!(migrate_plain_http_token(&store, "http://gitlab.vpn.test").is_err());
        assert_eq!(keys(&store.inner), ["gitlab.vpn.test"]);
        // Retried once the keychain answers: moved.
        migrate_plain_http_token(&store.inner, "http://gitlab.vpn.test").unwrap();
        assert_eq!(keys(&store.inner), ["http://gitlab.vpn.test"]);
        // An https instance never touches the keychain, failing or not.
        let store = Failing {
            get: true,
            set: true,
            ..Failing::default()
        };
        migrate_plain_http_token(&store, "https://gitlab.example.test").unwrap();
    }

    #[test]
    fn the_one_time_move_heals_whatever_an_earlier_build_left() {
        // Moved already: the http entry has it, nothing changes.
        let store = MemStore::default();
        store.set("http://gitlab.vpn.test", FIXTURE_TOKEN).unwrap();
        migrate_plain_http_token(&store, "http://gitlab.vpn.test").unwrap();
        assert_eq!(keys(&store), ["http://gitlab.vpn.test"]);
        // Marked done after a failed move: still under the host, moved now.
        let store = MemStore::default();
        store.set("gitlab.vpn.test", FIXTURE_TOKEN).unwrap();
        migrate_plain_http_token(&store, "http://gitlab.vpn.test").unwrap();
        assert_eq!(
            read_token(&store, "http://gitlab.vpn.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
        assert_eq!(keys(&store), ["http://gitlab.vpn.test"]);
        // No token anywhere: nothing to do, and that is done too.
        migrate_plain_http_token(&MemStore::default(), "http://gitlab.vpn.test").unwrap();
    }

    /// A keychain whose deletes fail (a locked or denied item).
    #[derive(Default)]
    struct NoDelete(MemStore);

    impl SecretStore for NoDelete {
        fn get(&self, account: &str) -> CoreResult<Option<String>> {
            self.0.get(account)
        }
        fn set(&self, account: &str, secret: &str) -> CoreResult<()> {
            self.0.set(account, secret)
        }
        fn delete(&self, _: &str) -> CoreResult<()> {
            Err(CoreError::Provider("could not update the keychain".into()))
        }
    }

    #[test]
    fn a_failed_cleanup_of_an_old_entry_does_not_fail_the_save() {
        let store = NoDelete::default();
        store
            .set("GitLab.example.test:443", "glpat-fixture-old")
            .unwrap();
        // The token is stored: that is what the save is about.
        write_token(&store, "https://GitLab.example.test:443", FIXTURE_TOKEN).unwrap();
        assert_eq!(
            read_token(&store, "https://gitlab.example.test").as_deref(),
            Some(FIXTURE_TOKEN)
        );
        // Removing still fails when the token itself cannot go.
        assert!(write_token(&store, "https://gitlab.example.test", "").is_err());
    }

    fn keys(store: &MemStore) -> Vec<String> {
        let mut keys: Vec<String> = store.0.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
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
