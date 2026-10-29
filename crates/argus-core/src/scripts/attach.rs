//! What a "Write with AI" request points at, attached by Argus so the agent
//! needs no tools: a merge or pull request link of a host Argus polls
//! (an enabled account, Settings → Accounts) is fetched read-only through
//! that host's provider, an absolute (or `~/…`) path to a
//! file inside a local clone or Argus' scripts folder is read. Everything
//! else is left alone, with a note the user sees on the turn and the agent
//! sees in the prompt. Attached content goes into the prompt as data.
//!
//! Never writes: only `fetch_review` / `fetch_threads` and file reads.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::provider::{ProviderId, ReviewProvider, ReviewRef};

/// Most links one request has fetched.
const MAX_LINKS: usize = 3;
/// Most paths one request has read.
const MAX_PATHS: usize = 5;
/// Most characters of one fetched merge or pull request.
pub(crate) const REVIEW_CAP: usize = 20_000;
/// Most characters of one file.
pub(crate) const FILE_CAP: usize = 30_000;
/// Most characters of one comment of a fetched review.
const NOTE_CAP: usize = 1_500;
/// How many of a review's latest comments come along.
const NOTES: usize = 5;

/// One piece of material for the prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Attachment {
    /// Short, for the turn ("MR !4242", "Code_review.md").
    pub label: String,
    /// The prompt's heading: what it is and where it came from.
    pub heading: String,
    pub content: String,
}

/// What one request's text brought along.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Gathered {
    pub material: Vec<Attachment>,
    /// What was not attached and why (one line each).
    pub notes: Vec<String>,
}

/// One of the user's hosts (Settings → Accounts), for the note on a link
/// to a host without a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnownHost {
    pub provider: ProviderId,
    pub host: String,
    /// The user's switch: off means Argus leaves the host alone.
    pub enabled: bool,
}

/// Where Argus may look for a request's material.
#[derive(Clone, Default)]
pub(crate) struct AttachContext {
    /// The providers Argus polls with: one per enabled host with a login.
    pub providers: Vec<Arc<dyn ReviewProvider>>,
    /// Every host of Settings → Accounts, providers or not.
    pub hosts: Vec<KnownHost>,
    /// The local clones and Argus' scripts folder.
    pub roots: Vec<PathBuf>,
    /// For `~/…` paths.
    pub home: Option<PathBuf>,
}

/// Whether `text` names anything [`gather`] would look at (so the turn can
/// say "Reading…" first).
pub(crate) fn mentions_anything(text: &str) -> bool {
    words(text).any(|w| w.contains("://") || is_path_like(w))
}

/// Fetch the review links and read the paths `text` names (blocking).
pub(crate) fn gather(text: &str, ctx: &AttachContext) -> Gathered {
    let mut out = Gathered::default();
    let mut links = 0;
    let mut paths = 0;
    let mut seen: Vec<String> = Vec::new();
    for word in words(text) {
        if seen.iter().any(|s| s == word) {
            continue;
        }
        seen.push(word.to_string());
        if word.contains("://") {
            let Some(link) = parse_link(word) else {
                continue;
            };
            let Some(provider) = ctx
                .providers
                .iter()
                .find(|p| p.id() == link.provider && p.host().eq_ignore_ascii_case(&link.host))
            else {
                out.notes.push(format!(
                    "Not fetched: {} ({})",
                    link.label(),
                    no_provider(&link, &ctx.hosts)
                ));
                continue;
            };
            links += 1;
            if links > MAX_LINKS {
                out.notes.push(format!(
                    "Not fetched: {} (at most {MAX_LINKS} links per request)",
                    link.label()
                ));
                continue;
            }
            match fetch(provider.as_ref(), &link) {
                Ok(a) => out.material.push(a),
                Err(why) => out
                    .notes
                    .push(format!("Couldn't fetch {}: {why}", link.label())),
            }
        } else if is_path_like(word) {
            paths += 1;
            if paths > MAX_PATHS {
                out.notes.push(format!(
                    "Not read: {word} (at most {MAX_PATHS} files per request)"
                ));
                continue;
            }
            match read_file(word, ctx) {
                Ok(a) => out.material.push(a),
                Err(why) => out.notes.push(format!("Not read: {word} ({why})")),
            }
        }
    }
    out
}

/// Why `link`'s host has no provider: switched off, on but not logged in,
/// or not one of the user's hosts at all.
fn no_provider(link: &Link, hosts: &[KnownHost]) -> String {
    let known = hosts
        .iter()
        .find(|h| h.provider == link.provider && h.host.eq_ignore_ascii_case(&link.host));
    match known {
        Some(h) if !h.enabled => format!("{} is switched off in Settings → Accounts", link.host),
        Some(_) => format!("{} is not logged in; see Settings → Accounts", link.host),
        None => format!("{} is not set up in Argus", link.host),
    }
}

/// The text's words, without the punctuation and quotes around them.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
        .map(|mut w| loop {
            let t = w
                .trim_matches(|c: char| "<>()[]{}\"'`,;!?".contains(c))
                .trim_end_matches(['.', ':']);
            if t == w {
                break w;
            }
            w = t;
        })
        .filter(|w| !w.is_empty())
}

fn is_path_like(word: &str) -> bool {
    (word.starts_with('/') && word.len() > 1 && !word.starts_with("//")) || word.starts_with("~/")
}

/// A merge or pull request link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    pub provider: ProviderId,
    /// Host, port included.
    pub host: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
}

impl Link {
    pub fn label(&self) -> String {
        match self.provider {
            ProviderId::Gitlab => format!("MR !{}", self.number),
            ProviderId::Github => format!("PR #{}", self.number),
        }
    }
}

/// `…/group/project/-/merge_requests/<iid>[/…]` (GitLab) or
/// `…/owner/repo/pull/<n>[/…]` (GitHub); query and fragment ignored.
pub(crate) fn parse_link(word: &str) -> Option<Link> {
    let (scheme, rest) = word.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let rest = rest.split(['?', '#']).next()?;
    let (host, path) = rest.split_once('/')?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    let url = format!("{scheme}://{host}/{}", path.trim_end_matches('/'));
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let number = |s: &str| s.parse::<u64>().ok().filter(|n| *n > 0);
    if let Some(dash) = segments.iter().position(|s| *s == "-") {
        if dash >= 2 && segments.get(dash + 1) == Some(&"merge_requests") {
            let n = number(segments.get(dash + 2)?)?;
            return Some(Link {
                provider: ProviderId::Gitlab,
                host: host.to_ascii_lowercase(),
                repo: segments[..dash].join("/"),
                number: n,
                url,
            });
        }
        return None;
    }
    if segments.len() >= 4 && segments[2] == "pull" {
        let n = number(segments[3])?;
        return Some(Link {
            provider: ProviderId::Github,
            host: host.to_ascii_lowercase(),
            repo: format!("{}/{}", segments[0], segments[1]),
            number: n,
            url,
        });
    }
    None
}

/// The review behind `link`, read only: title, state, author, description
/// and its latest comments on the whole review.
fn fetch(provider: &dyn ReviewProvider, link: &Link) -> Result<Attachment, String> {
    let r = ReviewRef {
        provider: link.provider,
        host: link.host.clone(),
        repo: link.repo.clone(),
        number: link.number,
        url: link.url.clone(),
        title: String::new(),
        author: String::new(),
        updated_at: None,
    };
    let detail = provider
        .fetch_review(&r)
        .map_err(|e| one_line(&e.to_string()))?;
    let kind = match link.provider {
        ProviderId::Gitlab => "Merge request",
        ProviderId::Github => "Pull request",
    };
    let mut content = format!(
        "Title: {}\nAuthor: @{}\nState: {:?}\nDescription:\n{}\n",
        detail.review.title.trim(),
        detail.review.author,
        detail.state,
        detail.description.trim_end()
    );
    // The comments are a bonus: a failure leaves them out.
    if let Ok(threads) = provider.fetch_threads(&r) {
        let mut notes: Vec<_> = threads
            .iter()
            .filter(|t| t.anchor.is_none())
            .flat_map(|t| t.notes.iter())
            .filter(|n| !n.system && !n.body.trim().is_empty())
            .collect();
        notes.sort_by_key(|n| n.created_at);
        let latest = &notes[notes.len().saturating_sub(NOTES)..];
        if !latest.is_empty() {
            content.push_str("\nLatest comments on it as a whole, oldest first:\n");
            for n in latest {
                content.push_str(&format!(
                    "--- @{} ({}):\n{}\n",
                    n.author,
                    n.created_at.format("%Y-%m-%d"),
                    cut(n.body.trim(), NOTE_CAP, "comment")
                ));
            }
        }
    }
    Ok(Attachment {
        label: link.label(),
        heading: format!(
            "{kind} {} of {} on {}, fetched by Argus from {} as an example",
            link.label().split_once(' ').map_or("", |(_, n)| n),
            link.repo,
            link.host,
            link.url
        ),
        content: cut(&content, REVIEW_CAP, "merge request"),
    })
}

/// `raw` (absolute or `~/…`) read when it is a text file inside one of the
/// roots; else why not.
pub(crate) fn read_file(raw: &str, ctx: &AttachContext) -> Result<Attachment, String> {
    let path = match raw.strip_prefix("~/") {
        Some(rest) => ctx.home.as_ref().ok_or("no home folder")?.join(rest),
        None => PathBuf::from(raw),
    };
    // No `..` or `.`: the path must say plainly where it is.
    if raw.split('/').any(|p| p == ".." || p == ".")
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("`..` and `.` are not allowed".into());
    }
    if !path.is_absolute() {
        return Err("not an absolute path".into());
    }
    let roots: Vec<(PathBuf, Option<PathBuf>)> = ctx
        .roots
        .iter()
        .map(|r| (r.clone(), r.canonicalize().ok()))
        .collect();
    let inside = |p: &Path, canonical: bool| {
        roots.iter().any(|(r, c)| {
            if canonical {
                c.as_ref().is_some_and(|c| p.starts_with(c))
            } else {
                p.starts_with(r) || c.as_ref().is_some_and(|c| p.starts_with(c))
            }
        })
    };
    if !inside(&path, false) {
        return Err("outside your local clones".into());
    }
    let real = path.canonicalize().map_err(|_| "not found")?;
    if !inside(&real, true) {
        return Err("a link that leads outside your local clones".into());
    }
    if !real.is_file() {
        return Err("not a file".into());
    }
    // A little more than the cap in bytes: enough to fill it with text.
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(&real)
        .and_then(|f| f.take((FILE_CAP * 4 + 4) as u64).read_to_end(&mut bytes))
        .map_err(|_| "could not be read")?;
    if bytes.contains(&0) {
        return Err("not a text file".into());
    }
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        // Cut inside a character at the byte limit: keep what is whole.
        Err(e) if e.utf8_error().error_len().is_none() => {
            let valid = e.utf8_error().valid_up_to();
            let mut b = e.into_bytes();
            b.truncate(valid);
            String::from_utf8(b).map_err(|_| "not a text file")?
        }
        Err(_) => return Err("not a text file".into()),
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| raw.to_string());
    Ok(Attachment {
        label: name,
        heading: format!(
            "File {}, read by Argus from your local files",
            path.display()
        ),
        content: cut(&text, FILE_CAP, "file"),
    })
}

/// `text` cut to `cap` characters, saying so.
fn cut(text: &str, cap: usize, what: &str) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let kept: String = text.chars().take(cap).collect();
    format!("{kept}\n[… cut by Argus: the {what} is longer than {cap} characters]")
}

fn one_line(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 200 {
        format!("{}…", line.chars().take(199).collect::<String>())
    } else {
        line
    }
}
