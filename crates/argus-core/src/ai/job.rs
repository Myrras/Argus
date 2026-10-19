//! One AI run, end to end: read the diff and threads, prepare the working
//! directory, build the prompt, run the agent (once more with a repair
//! instruction when its answer is invalid), validate and filter.
//!
//! Nothing is stored here: the runtime saves the [`RunOutput`] in one
//! transaction, so a failed run leaves the store as it was.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::adapter::{self, Adapter, ChildSlot, Invocation, PromptDelivery, RunDirs};
use super::dedupe;
use super::model::Finding;
use super::output::{self, RunOutput};
use super::prompt::{self, DiffScope, ExistingThread, PromptInput, Workspace};
use crate::files::{review_range, since_range};
use crate::provider::{FileDiff, ReviewProvider};
use crate::settings::Settings;
use crate::store::{ReviewRecord, Store, StoredThread};

/// Files read from a local clone's root as repository conventions.
pub(crate) const CONVENTION_FILES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// What a queued run needs besides the store, captured when it is queued.
#[derive(Clone)]
pub(crate) struct JobContext {
    pub settings: Settings,
    /// Read access only: the AI pass never posts.
    pub provider: Arc<dyn ReviewProvider>,
    pub timeout: Duration,
    /// The agent process while one runs; the runtime gives each started run
    /// a fresh one and kills what is in it on shutdown.
    pub child: ChildSlot,
}

/// Prefix of a run's temp dir under the system temp dir.
const TEMP_PREFIX: &str = "argus-ai-";

/// A directory under the system temp dir, removed (with its contents) on
/// drop, so a run cleans up after itself however it ends.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// A fresh dir, by its canonical path (macOS' temp dir is behind the
    /// `/var` → `/private/var` link), which the agent's deny rules name.
    pub fn new() -> std::io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path)?;
        let path = path.canonicalize().unwrap_or(path);
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Remove the temp dirs of runs a previous Argus left behind (it crashed or
/// was killed mid-run): they hold private diffs. Only for a process that
/// knows no run of its own is live (the app at startup; one instance runs
/// at a time). Returns how many went.
pub fn sweep_stale_temp_dirs() -> usize {
    sweep_temp_dirs_in(&std::env::temp_dir())
}

pub(crate) fn sweep_temp_dirs_in(base: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(base) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            // Exactly Argus' own names: the prefix and a simple UUID.
            name.strip_prefix(TEMP_PREFIX)
                .is_some_and(|id| id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()))
                && e.file_type().is_ok_and(|t| t.is_dir())
        })
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

/// Longest file name most file systems take (`NAME_MAX`), in bytes.
const MAX_NAME: usize = 255;

/// A path component that fits [`MAX_NAME`] with `suffix` added: as is, or
/// (a name only a platform could hold) its SHA-1 instead.
fn fit_name(name: &std::ffi::OsStr, suffix: &str) -> std::ffi::OsString {
    if name.len() + suffix.len() <= MAX_NAME {
        let mut out = name.to_os_string();
        out.push(suffix);
        return out;
    }
    let hash = sha1_smol::Sha1::from(name.to_string_lossy().as_bytes())
        .digest()
        .to_string();
    format!("{hash}{suffix}").into()
}

/// `diff/<path>.diff` for a diff's `path`: only its normal components (a
/// hostile `../../x` cannot write outside the snapshot dir), each fitting
/// a file name. `None` for a path with no normal component.
fn snapshot_path(path: &str) -> Option<PathBuf> {
    let parts: Vec<_> = Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    let (last, dirs) = parts.split_last()?;
    let mut out = PathBuf::from("diff");
    for d in dirs {
        out.push(fit_name(d, ""));
    }
    out.push(fit_name(last, ".diff"));
    Some(out)
}

/// Fill a snapshot dir: `diff/<path>.diff` per file with a patch, and
/// `threads.json` with every thread of the review. A file that cannot be
/// written is left out (its patch is in the prompt all the same); only
/// `threads.json` failing fails.
pub(crate) fn write_snapshot(
    dir: &Path,
    diff: &FileDiff,
    threads: &[StoredThread],
) -> std::io::Result<()> {
    for f in &diff.files {
        let (Some(patch), Some(rel)) = (f.patch.as_deref(), snapshot_path(&f.path)) else {
            continue;
        };
        let target = dir.join(rel);
        let written = target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&target, patch));
        if let Err(e) = written {
            tracing::debug!("AI run: a diff file was left out of the snapshot: {e}");
        }
    }
    let json = serde_json::to_string_pretty(threads).map_err(std::io::Error::other)?;
    std::fs::write(dir.join("threads.json"), json)
}

/// Tokens the agent's answer must not carry into a finding, besides the
/// shapes [`super::output::redact`] knows: the configured GitLab token.
/// (`gh`'s token stays inside the provider; GitHub tokens have fixed
/// prefixes, which the shapes cover.)
pub(crate) fn known_secrets(settings: &Settings) -> Vec<String> {
    use crate::provider::gitlab::{KeychainToken, TokenSource};
    let url = settings.gitlab.base_url.trim();
    if url.is_empty() {
        return Vec::new();
    }
    KeychainToken::for_base_url(url)
        .token()
        .into_iter()
        .collect()
}

/// `CLAUDE.md` / `AGENTS.md` at the clone's root, those that exist.
pub(crate) fn conventions(clone: &Path) -> Vec<(String, String)> {
    CONVENTION_FILES
        .iter()
        .filter_map(|name| {
            let text = std::fs::read_to_string(clone.join(name)).ok()?;
            (!text.trim().is_empty()).then(|| (name.to_string(), text))
        })
        .collect()
}

/// Compute (and cache) the histories of the threads a later round asks about at the review's
/// head. Errors are logged and dropped: history is context, not a
/// requirement of the run.
fn warm_histories(
    store: &Store,
    record: &ReviewRecord,
    ctx: &JobContext,
    threads: &[StoredThread],
) {
    let ids: Vec<_> = prompt::open_threads(threads, |_| None)
        .iter()
        .map(|t| t.thread.thread_id)
        .collect();
    if ids.is_empty() {
        return;
    }
    let provider = Ok(ctx.provider.clone());
    let clone = ctx.settings.clone_path(&record.review);
    if let Err(e) = crate::history::compute_histories(store, record, &provider, clone, Some(&ids)) {
        tracing::debug!("AI run: thread history unavailable: {e}");
    }
}

/// The cached then/now text of a thread at the review's head, as the prompt
/// wants it. `None` without a cached history (or with nothing in it).
fn thread_snapshot(
    store: &Store,
    record: &ReviewRecord,
    thread: crate::ids::ThreadId,
) -> Option<prompt::ThreadSnapshot> {
    let text = store.snapshot_text(thread, &record.head_sha).ok()??;
    let snapshot = prompt::ThreadSnapshot {
        then_code: text.then_code,
        now_code: text.now_code,
        now_path: text.now_path,
        now_line: text.now_line,
    };
    (snapshot != prompt::ThreadSnapshot::default()).then_some(snapshot)
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(super::CANCELLED.into())
    } else {
        Ok(())
    }
}

/// Run the AI pass for `record` (blocking).
pub(crate) fn execute(
    store: &Store,
    record: &ReviewRecord,
    ctx: &JobContext,
    cancel: &AtomicBool,
) -> Result<RunOutput, String> {
    let ai = &ctx.settings.ai;
    let adapter = adapter::adapter(&ai.adapter)
        .ok_or_else(|| "unknown agent adapter: choose claude or auggie in Settings".to_string())?;
    let program = adapter
        .resolve(ai.binary.as_deref())
        .ok_or_else(|| adapter::AgentError::NotFound(adapter.binary).message())?;

    // Round 1 (or never marked reviewed): the whole diff. Later: since the
    // watermark; with no new commits, the whole diff again for context.
    let provider = ctx.provider.as_ref();
    let fetch = |range| {
        provider
            .fetch_files(&record.review, &range)
            .map_err(|e| e.to_string())
    };
    let first = record.round <= 1 || record.last_reviewed_sha.is_none();
    let since = if first {
        None
    } else {
        since_range(record).ok()
    };
    let new_commits = since.is_some();
    let mut diff = fetch(since.unwrap_or_else(|| review_range(record)))?;
    // Old-side findings are numbered at the left side the code view and
    // posting use (see `crate::exact_base`).
    crate::exact_base(&ctx.settings, record, &mut diff);
    let scope = match (first, new_commits) {
        (true, _) => DiffScope::Full(&diff),
        (false, true) => DiffScope::SinceLastReview(&diff),
        (false, false) => DiffScope::FullNoNewCommits(&diff),
    };
    check(cancel)?;

    let threads = store.threads(record.id).map_err(|e| e.to_string())?;
    // Later rounds: the then/now text of the user's open threads. Computed
    // here (cached per thread and head, so a re-run is cheap) because the
    // history is otherwise only built when the UI opens a thread. A failure
    // never fails the run: the thread goes in with its own excerpt only.
    let asked = if first {
        Vec::new()
    } else {
        warm_histories(store, record, ctx, &threads);
        check(cancel)?;
        prompt::open_threads(&threads, |id| thread_snapshot(store, record, id))
    };
    let known: HashSet<_> = asked.iter().map(|t| t.thread.thread_id).collect();
    // Every thread, in every round, so nothing the review already says is
    // raised again; with its then/now code when the history cache has it
    // (only read here: computing it for every thread would cost a walk
    // each).
    let snapshots: Vec<_> = threads
        .iter()
        .map(|t| thread_snapshot(store, record, t.thread_id))
        .collect();
    let earlier = super::store::findings(store, record.id).map_err(|e| e.to_string())?;
    let posted: Vec<&Finding> = earlier.iter().filter(|f| f.posted_at.is_some()).collect();
    let raised = dedupe::raised_points(
        &threads
            .iter()
            .zip(snapshots.iter().cloned())
            .collect::<Vec<_>>(),
        &posted,
    );
    let existing: Vec<ExistingThread<'_>> = threads
        .iter()
        .zip(snapshots)
        .map(|(thread, snapshot)| ExistingThread { thread, snapshot })
        .collect();
    // A posted finding whose thread is already fetched is in `existing`.
    let unfetched: Vec<&Finding> = posted
        .iter()
        .copied()
        .filter(|f| !threads.iter().any(|t| posted_as(t, f)))
        .collect();
    let paths: HashSet<String> = diff
        .files
        .iter()
        .flat_map(|f| std::iter::once(f.path.clone()).chain(f.old_path.clone()))
        .collect();

    let clone = ctx
        .settings
        .clone_path(&record.review)
        .filter(|p| p.is_dir())
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    // The working directory is always the run's own temp dir: an agent
    // started in the clone would load the checkout's own agent settings
    // (hooks, env, an API key helper), which a PR branch can change.
    let temp = TempDir::new().map_err(|_| "could not create a temp dir for the run".to_string())?;
    write_snapshot(temp.path(), &diff, &threads)
        .map_err(|_| "could not write the review snapshot".to_string())?;
    let (workspace, conventions) = match &clone {
        Some(c) => (Workspace::Clone(c), conventions(c)),
        None => (Workspace::Snapshot, Vec::new()),
    };
    let settings_file = match adapter.id {
        "claude" => {
            let mut allowed = vec![temp.path().to_path_buf()];
            allowed.extend(clone.clone());
            let file = temp.path().join("claude-settings.json");
            let json = adapter::claude_settings(&allowed, clone.as_deref()).to_string();
            std::fs::write(&file, json)
                .map_err(|_| "could not write the agent's settings".to_string())?;
            Some(file)
        }
        _ => None,
    };
    // Auggie has no allow-list: what it lists for this user (MCP tools
    // included) is removed, all but the read tools.
    let listed_tools = match adapter.id {
        "auggie" => {
            check(cancel)?;
            let inv = Invocation {
                program: PathBuf::from(&program),
                args: vec!["tools".into(), "list".into()],
                cwd: temp.path().to_path_buf(),
                env: agent_env(),
                stdin: None,
                timeout: ctx.timeout.min(TOOL_LIST_TIMEOUT),
                output: adapter::OutputKind::Text,
            };
            // Fail closed: without the listing, a tool that writes (an MCP
            // server of the user's) would stay switched on.
            match adapter::run_tracked(&inv, cancel, &ctx.child) {
                Ok(listing) => {
                    let tools = adapter::parse_auggie_tools(&listing);
                    if tools.is_empty() {
                        tracing::debug!("AI run: `auggie tools list` named no tools");
                        return Err(TOOLS_UNLISTED.into());
                    }
                    tools
                }
                Err(adapter::AgentError::Cancelled) => return Err(super::CANCELLED.into()),
                Err(e) => {
                    tracing::debug!("AI run: `auggie tools list` failed: {}", e.message());
                    return Err(TOOLS_UNLISTED.into());
                }
            }
        }
        _ => Vec::new(),
    };
    let dirs = RunDirs {
        temp: temp.path(),
        clone: clone.as_deref(),
        settings: settings_file.as_deref(),
        listed_tools: &listed_tools,
    };
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let text = prompt::build(&PromptInput {
        record,
        description: None,
        scope,
        threads: asked,
        existing,
        posted: unfetched,
        conventions,
        workspace,
        nonce: &nonce,
    });

    let spawn = |prompt: &str| -> Result<String, String> {
        check(cancel)?;
        let inv = invocation(&adapter, &program, &dirs, prompt, ctx)?;
        adapter::run_tracked(&inv, cancel, &ctx.child).map_err(|e| e.message())
    };
    let answer = spawn(&text)?;
    let parsed = match output::parse(&answer) {
        Ok(out) => out,
        Err(first_error) => {
            let retry = spawn(&prompt::repair(&text, &first_error))?;
            output::parse(&retry)
                .map_err(|e| format!("the agent's answer was invalid after a retry: {e}"))?
        }
    };
    let mut run = output::filter(parsed, &paths, &known);
    dedupe::drop_repeats(&mut run, &raised);
    Ok(RunOutput {
        // The shas of the diff actually returned (the provider's contract),
        // which for a run since the last review has the watermark as its
        // left side.
        head_sha: Some(diff.head_sha.clone()),
        base_sha: Some(diff.base_sha.clone()),
        ..run
    })
}

/// Whether `t` is the thread posting `f` made: started by the user (a
/// colleague's thread the user replied in is theirs, but not a post), on
/// its file, its root carrying the finding's title (the posted body starts
/// with it unless the user rewrote it).
fn posted_as(t: &StoredThread, f: &Finding) -> bool {
    t.started_by_me
        && t.thread.anchor.as_ref().is_some_and(|a| {
            a.path_new.as_deref() == Some(f.path.as_str())
                || a.path_old.as_deref() == Some(f.path.as_str())
        })
        && t.thread
            .notes
            .first()
            .is_some_and(|n| n.body.contains(f.title.as_str()))
}

/// Why an auggie run did not start: its tools could not be listed (the
/// listing failed, timed out, or named none), so the ones that write cannot
/// be removed.
pub(crate) const TOOLS_UNLISTED: &str = "Could not list auggie's tools, so Argus can't switch off the ones that write; the run was not started";

/// Most `auggie tools list` may take (it starts the user's MCP servers).
const TOOL_LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// The environment of every agent process (see [`adapter::sanitized_env`]).
fn agent_env() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    adapter::sanitized_env(std::env::vars_os(), crate::platform::login_path())
}

fn invocation(
    adapter: &Adapter,
    program: &str,
    dirs: &RunDirs<'_>,
    prompt: &str,
    ctx: &JobContext,
) -> Result<Invocation, String> {
    let (stdin, prompt_file) = match adapter.prompt {
        PromptDelivery::Stdin => (Some(prompt.to_string()), None),
        PromptDelivery::File(_) => {
            let file = dirs.temp.join("prompt.md");
            std::fs::write(&file, prompt).map_err(|_| "could not write the prompt".to_string())?;
            (None, Some(file))
        }
    };
    Ok(Invocation {
        program: PathBuf::from(program),
        args: adapter.args(dirs, prompt_file.as_deref(), &ctx.settings.ai.args),
        cwd: dirs.temp.to_path_buf(),
        env: agent_env(),
        stdin,
        timeout: ctx.timeout,
        output: adapter.output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::Severity;
    use crate::ids::{ReviewId, ThreadId};
    use crate::provider::Side;
    use crate::testutil::{at, note, thread, AUTHOR, ME};

    fn stored(author: &str, root: &str) -> StoredThread {
        let mut root_note = note("1", author, at(10, 1));
        root_note.body = root.into();
        let t = thread("1", author, "h1", vec![root_note, note("2", ME, at(10, 2))]);
        let review = ReviewId::from_key("r");
        StoredThread {
            thread_id: ThreadId::from_key(review, "1"),
            review_id: review,
            mine: true,
            started_by_me: t.started_by(ME),
            thread: t,
        }
    }

    #[test]
    fn a_posted_finding_is_a_thread_the_user_started_not_one_they_replied_in() {
        let f = Finding {
            id: crate::ai::FindingId::new(),
            review_id: ReviewId::from_key("r"),
            round: 1,
            path: "src/cache.rs".into(),
            line: 10,
            side: Side::New,
            severity: Severity::High,
            title: "Off by one".into(),
            body: "b".into(),
            dismissed: false,
            posted_at: Some(at(10, 0)),
            posted_url: None,
        };
        assert!(posted_as(&stored(ME, "**Off by one**\n\nb"), &f));
        // A colleague's thread making the same point, with the user's reply.
        assert!(!posted_as(&stored(AUTHOR, "Off by one here?"), &f));
    }
}
