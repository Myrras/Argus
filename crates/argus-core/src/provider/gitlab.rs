//! GitLab, over its REST API with a personal access token.
//!
//! The token lives in the OS keychain (service `com.argus.app.gitlab`, the
//! account is the instance host), read on demand, never logged and never
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

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::conditional::{Cached, ConditionalCache};
use super::github::check_sha;
use super::model::{
    Anchor, ChangedFile, CommitInfo, DiffRange, DiffVersion, FileDiff, FileStatus, Note,
    ProviderId, ReviewDetail, ReviewRef, ReviewState, Reviewer, ReviewerState, Side, Thread,
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
    host: String,
}

impl KeychainToken {
    /// The keychain entry for the instance at `base_url`. Keyed by host, so a
    /// token never goes to a different instance after the URL changes.
    pub fn for_base_url(base_url: &str) -> Self {
        Self {
            host: url_host(base_url).to_string(),
        }
    }
}

impl TokenSource for KeychainToken {
    fn token(&self) -> Option<String> {
        keychain_entry(&self.host)
            .ok()?
            .get_password()
            .ok()
            .filter(|t| !t.is_empty())
    }
}

fn keychain_entry(host: &str) -> CoreResult<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, host)
        .map_err(|_| CoreError::Provider("could not open the keychain".into()))
}

/// Store the token for the instance at `base_url`; a blank token deletes it.
pub fn gitlab_token_set(base_url: &str, token: &str) -> CoreResult<()> {
    let entry = keychain_entry(url_host(base_url))?;
    let result = if token.trim().is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(token.trim())
    };
    result.map_err(|_| CoreError::Provider("could not update the keychain".into()))
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

/// Trim and drop a trailing slash; `None` when not an http(s) URL.
pub fn normalize_base_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let ok =
        (url.starts_with("https://") || url.starts_with("http://")) && !url_host(url).is_empty();
    ok.then(|| url.to_string())
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
        })
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
    /// more items arrive after it. An empty next page ends it.
    fn get_all<T: DeserializeOwned>(&self, path: &str) -> CoreResult<Vec<T>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut all = Vec::new();
        let mut page = 1;
        for _ in 0..MAX_PAGES {
            let (body, next) = self.get(&format!("{path}{sep}per_page={PER_PAGE}&page={page}"))?;
            let items: Vec<T> = serde_json::from_str(&body).map_err(|_| unexpected())?;
            let full = items.len() >= PER_PAGE as usize;
            all.extend(items);
            match next {
                Some(n) if n > page => page = n,
                None if full => page += 1,
                _ => break,
            }
        }
        Ok(all)
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
    /// [`COMMENTED_DAYS`] days and fetches the MRs they point at.
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
        for e in events {
            let Some(note) = e.note else { continue };
            let (Some(project), Some(iid)) = (e.project_id, note.noteable_iid) else {
                continue;
            };
            if note.noteable_type.as_deref() != Some("MergeRequest")
                || !known.insert((project, iid))
            {
                continue;
            }
            let mr: GlMr = match self.get_json(&format!("/projects/{project}/merge_requests/{iid}"))
            {
                Ok(mr) => mr,
                // Gone or no longer visible: just not listed.
                Err(CoreError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            if mr.state == "opened" && !same_user(&mr.author.username, &me.username) {
                list.push(self.to_ref(&mr));
            }
        }
        list.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(list)
    }

    fn fetch_review(&self, r: &ReviewRef) -> CoreResult<ReviewDetail> {
        let me = self.me()?.username;
        let p = Self::mr_path(r);
        let (mr, commits, versions, reviewers, approvals) = std::thread::scope(|s| {
            let a = s.spawn(|| self.get_json::<GlMr>(&p));
            let b = s.spawn(|| self.get_all::<GlCommit>(&format!("{p}/commits")));
            let c = s.spawn(|| self.get_all::<GlVersion>(&format!("{p}/versions")));
            let d = s.spawn(|| self.get_all::<GlReviewer>(&format!("{p}/reviewers")));
            let e = s.spawn(|| self.get_json::<GlApprovals>(&format!("{p}/approvals")));
            (join(a), join(b), join(c), join(d), join(e))
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
        Ok(assemble_detail(
            self.to_ref(&mr),
            &me,
            mr,
            commits?,
            versions?,
            reviewers,
            approved_by,
        ))
    }

    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
        let p = Self::mr_path(r);
        let (mr, discussions) = std::thread::scope(|s| {
            let a = s.spawn(|| self.get_json::<GlMr>(&p));
            let b = s.spawn(|| self.get_all::<GlDiscussion>(&format!("{p}/discussions")));
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
                let p = Self::mr_path(r);
                // `/diffs` (15.7+) pages; an older instance only has the
                // single-shot `/changes`, whose `overflow` flags a cut list.
                let (diffs, truncated) = match self.get_all::<GlDiff>(&format!("{p}/diffs")) {
                    Ok(d) => {
                        let full = d.len() >= (PER_PAGE * MAX_PAGES) as usize;
                        (d, full)
                    }
                    Err(CoreError::NotFound) => {
                        let c: GlChanges = self.get_json(&format!("{p}/changes"))?;
                        (c.changes, c.overflow)
                    }
                    Err(e) => return Err(e),
                };
                Ok(FileDiff {
                    files: diffs.into_iter().map(|d| changed_file(d, head)).collect(),
                    base_sha: base.clone(),
                    head_sha: head.clone(),
                    truncated,
                })
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

    fn commit_diff(&self, r: &ReviewRef, sha: &str) -> CoreResult<FileDiff> {
        check_sha(sha)?;
        let diffs: Vec<GlDiff> = self.get_all(&format!(
            "/projects/{}/repository/commits/{sha}/diff",
            encode_segment(&r.repo)
        ))?;
        Ok(FileDiff {
            // The diff endpoint names no parent.
            base_sha: String::new(),
            head_sha: sha.to_string(),
            truncated: diffs.len() >= (PER_PAGE * MAX_PAGES) as usize,
            files: diffs.into_iter().map(|d| changed_file(d, sha)).collect(),
        })
    }

    /// GitLab keeps only the current resolution: a resolved discussion's
    /// latest `resolved_at` and who did it. A discussion resolved on an
    /// instance that does not send `resolved_at` has no time and is left
    /// out.
    fn thread_resolutions(&self, r: &ReviewRef) -> CoreResult<Vec<ThreadResolution>> {
        let discussions: Vec<GlResolveDiscussion> =
            self.get_all(&format!("{}/discussions", Self::mr_path(r)))?;
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
    let mut commits: Vec<CommitInfo> = commits
        .into_iter()
        .map(|c| CommitInfo {
            title: c.title.unwrap_or_else(|| first_line(&c.message)),
            author: c.author_name,
            authored_at: c.authored_date,
            sha: c.id,
        })
        .collect();
    // GitLab lists commits newest first; the model is oldest first.
    commits.reverse();
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
    }
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

#[derive(Deserialize)]
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
}
