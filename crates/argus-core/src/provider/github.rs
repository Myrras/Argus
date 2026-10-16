//! GitHub, via the `gh` CLI — exactly Podium's mechanism, so Argus never
//! stores a GitHub token.
//!
//! With an account configured, `gh auth token --hostname <host> --user
//! <account>` picks that account's token, which goes to every later `gh` call
//! through `GH_TOKEN` (`GH_ENTERPRISE_TOKEN` off github.com) and nowhere
//! else. Without one, `gh` uses its active account. A host other than
//! github.com is selected with `GH_HOST`, which both `gh search` and `gh api`
//! honour.
//!
//! `gh`'s stderr is only inspected to classify a failure, never forwarded.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::Mutex;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::model::{
    Anchor, CommitInfo, Note, ProviderId, ReviewDetail, ReviewRef, ReviewState, Reviewer,
    ReviewerState, Side, Thread,
};
use super::{encode_path, first_line, ReviewProvider};
use crate::error::{CoreError, CoreResult};

/// Upper bound per search; a reviewer with more than this many open PRs in
/// one bucket is not the user Argus is built for.
const SEARCH_LIMIT: &str = "100";
const SEARCH_FIELDS: &str = "number,title,url,repository,author,updatedAt";

/// GitHub through the `gh` binary.
pub struct GithubProvider {
    /// Program to run; `gh` normally, a fake script in tests (the same
    /// injection Podium uses instead of mutating `PATH`).
    gh: String,
    host: String,
    account: Option<String>,
    /// The picked account's token, held in memory only.
    token: Mutex<Option<String>>,
    me: Mutex<Option<String>>,
}

impl GithubProvider {
    /// `host` is `github.com` or a GHE host; `account` picks a logged-in `gh`
    /// account, `None` uses the active one.
    pub fn new(host: impl Into<String>, account: Option<String>) -> Self {
        Self::with_binary("gh", host, account)
    }

    /// Like [`new`](Self::new) with an explicit `gh` program.
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
        }
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

    /// The configured account's token, fetched once and kept in memory.
    fn account_token(&self) -> CoreResult<Option<String>> {
        let Some(account) = &self.account else {
            return Ok(None);
        };
        let mut cached = self.token.lock().expect("token lock poisoned");
        if let Some(t) = cached.as_ref() {
            return Ok(Some(t.clone()));
        }
        let mut cmd = self.command();
        cmd.args(["auth", "token", "--hostname", &self.host, "--user", account])
            .stderr(Stdio::null());
        let output = cmd
            .spawn()
            .and_then(|c| c.wait_with_output())
            .map_err(|_| CoreError::GithubUnavailable)?;
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success() || token.is_empty() {
            return Err(CoreError::GithubUnavailable);
        }
        *cached = Some(token.clone());
        Ok(Some(token))
    }

    /// Run one `gh` call and return its stdout.
    fn run(&self, args: &[&str]) -> CoreResult<String> {
        let token = self.account_token()?;
        let mut cmd = self.command();
        cmd.args(args);
        if let Some(token) = &token {
            let var = if self.host == "github.com" {
                "GH_TOKEN"
            } else {
                "GH_ENTERPRISE_TOKEN"
            };
            cmd.env(var, token);
        }
        let output = cmd
            .spawn()
            .and_then(|c| c.wait_with_output())
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => CoreError::GithubUnavailable,
                _ => CoreError::Provider("could not run gh".into()),
            })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        tracing::debug!("gh {} failed", args.first().unwrap_or(&""));
        Err(
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
                CoreError::Provider("GitHub rate limit reached, try again later".into())
            } else {
                CoreError::Provider("gh could not fetch from GitHub".into())
            },
        )
    }

    fn api<T: DeserializeOwned>(&self, path: &str) -> CoreResult<T> {
        let out = self.run(&["api", path])?;
        serde_json::from_str(&out).map_err(|_| unexpected())
    }

    /// A paginated list endpoint. `gh api --paginate` prints one JSON array
    /// per page back to back (`[..][..]`), so this reads a stream of arrays
    /// and flattens it — which also covers a single array.
    fn api_list<T: DeserializeOwned>(&self, path: &str) -> CoreResult<Vec<T>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let out = self.run(&["api", "--paginate", &format!("{path}{sep}per_page=100")])?;
        parse_pages(&out)
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

fn unexpected() -> CoreError {
    CoreError::Provider("unexpected response from gh".into())
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

    fn fetch_threads(&self, r: &ReviewRef) -> CoreResult<Vec<Thread>> {
        let comments: Vec<GhComment> =
            self.api_list(&format!("repos/{}/pulls/{}/comments", r.repo, r.number))?;
        Ok(group_threads(comments))
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
        commits: commits
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
            .collect(),
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
struct GhContent {
    content: Option<String>,
    encoding: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_concatenated_pages() {
        let v: Vec<u32> = parse_pages("[1,2]\n[3]").unwrap();
        assert_eq!(v, [1, 2, 3]);
        let v: Vec<u32> = parse_pages("[]").unwrap();
        assert!(v.is_empty());
        assert!(parse_pages::<u32>("{oops").is_err());
    }

    #[test]
    fn rejects_non_sha() {
        assert!(check_sha("abc123").is_ok());
        assert!(check_sha("--help").is_err());
        assert!(check_sha("").is_err());
    }
}
