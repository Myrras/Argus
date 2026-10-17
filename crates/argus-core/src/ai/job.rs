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
use super::output::{self, RunOutput};
use super::prompt::{self, DiffScope, PromptInput, Workspace};
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
    let diff = fetch(since.unwrap_or_else(|| review_range(record)))?;
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
            match adapter::run_tracked(&inv, cancel, &ctx.child) {
                Ok(listing) => adapter::parse_auggie_tools(&listing),
                Err(adapter::AgentError::Cancelled) => return Err(super::CANCELLED.into()),
                // The static list still applies.
                Err(_) => Vec::new(),
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
    Ok(RunOutput {
        // The shas of the diff actually returned (the provider's contract),
        // which for a run since the last review has the watermark as its
        // left side.
        head_sha: Some(diff.head_sha.clone()),
        base_sha: Some(diff.base_sha.clone()),
        ..output::filter(parsed, &paths, &known)
    })
}

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
