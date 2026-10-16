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
use std::io::Write as _;
use std::process::Stdio;
use std::sync::Mutex;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::model::{
    Anchor, ChangedFile, CommitInfo, DiffRange, FileDiff, FileStatus, Note, ProviderId,
    ReviewDetail, ReviewRef, ReviewState, Reviewer, ReviewerState, Side, Thread,
};
use super::{encode_path, first_line, looks_binary, PostResultOf, ReviewProvider};
use crate::error::{CoreError, CoreResult};
use crate::post::{sanitize, PlacedComment, PostError, PostErrorKind, PostedComment, PostedNote};

/// Upper bound per search; a reviewer with more than this many open PRs in
/// one bucket is not the user Argus is built for.
const SEARCH_LIMIT: &str = "100";
const SEARCH_FIELDS: &str = "number,title,url,repository,author,updatedAt";

/// GitHub's caps: `pulls/{n}/files` lists at most 3000 files, a compare at
/// most 300. Reaching one means the list is cut off.
const PULL_FILES_LIMIT: usize = 3000;
const COMPARE_FILES_LIMIT: usize = 300;

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
    /// The picked account's token, held in memory only.
    token: Mutex<Option<String>>,
    me: Mutex<Option<String>>,
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
        match cmd.status() {
            Ok(status) => GhAuth {
                installed: true,
                logged_in: status.success(),
            },
            Err(_) => GhAuth {
                installed: false,
                logged_in: false,
            },
        }
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

    fn fetch_files(&self, r: &ReviewRef, range: &DiffRange) -> CoreResult<FileDiff> {
        match range {
            DiffRange::Review { base, head } => {
                check_sha(base)?;
                check_sha(head)?;
                // The PR's files are diffed from the merge base (three-dot),
                // not from the base branch tip the PR reports: the compare
                // API names that commit.
                let (files, merge_base) = std::thread::scope(|s| {
                    let a = s.spawn(|| {
                        self.api_list::<GhFile>(&format!(
                            "repos/{}/pulls/{}/files",
                            r.repo, r.number
                        ))
                    });
                    let b = s.spawn(|| {
                        self.run(&[
                            "api",
                            &format!("repos/{}/compare/{base}...{head}", r.repo),
                            "--jq",
                            ".merge_base_commit.sha",
                        ])
                    });
                    (join(a), join(b))
                });
                let files = files?;
                let merge_base = merge_base?.trim().to_string();
                check_sha(&merge_base).map_err(|_| unexpected())?;
                Ok(FileDiff {
                    base_sha: merge_base,
                    head_sha: head.clone(),
                    truncated: files.len() >= PULL_FILES_LIMIT,
                    files: files.into_iter().map(changed_file).collect(),
                })
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
    let binary =
        no_patch && f.changes == 0 && (status == FileStatus::Modified || looks_binary(&f.filename));
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
        let token = self.account_token()?;
        let mut cmd = self.command();
        cmd.args(["api", "--method", "POST", path, "--input", "-"])
            .stdin(Stdio::piped());
        if let Some(token) = &token {
            let var = if self.host == "github.com" {
                "GH_TOKEN"
            } else {
                "GH_ENTERPRISE_TOKEN"
            };
            cmd.env(var, token);
        }
        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CoreError::GithubUnavailable,
            _ => CoreError::Provider("could not run gh".into()),
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            // gh reads all of stdin before it sends anything. A write error
            // means gh exited early; its status says why.
            let _ = stdin.write_all(body.to_string().as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|_| CoreError::Provider("could not run gh".into()))?;
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
