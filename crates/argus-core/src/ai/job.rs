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

use super::adapter::{self, Adapter, Invocation, PromptDelivery};
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
}

/// A directory under the system temp dir, removed (with its contents) on
/// drop, so a run cleans up after itself however it ends.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!("argus-ai-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path)?;
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

/// `path` from a diff as a relative path with only normal components, so a
/// hostile `../../x` cannot write outside the snapshot dir.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let p: PathBuf = Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    (!p.as_os_str().is_empty()).then_some(p)
}

/// Fill a snapshot dir: `diff/<path>.diff` per file with a patch, and
/// `threads.json` with every thread of the review.
pub(crate) fn write_snapshot(
    dir: &Path,
    diff: &FileDiff,
    threads: &[StoredThread],
) -> std::io::Result<()> {
    for f in &diff.files {
        let (Some(patch), Some(rel)) = (f.patch.as_deref(), safe_relative(&f.path)) else {
            continue;
        };
        let mut target = dir.join("diff").join(rel);
        let name = format!(
            "{}.diff",
            target.file_name().unwrap_or_default().to_string_lossy()
        );
        target.set_file_name(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, patch)?;
    }
    let json = serde_json::to_string_pretty(threads).map_err(std::io::Error::other)?;
    std::fs::write(dir.join("threads.json"), json)
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
    // M3 HOOK: pass the thread's then/now snapshot (store `snapshots` at the
    // current head) instead of `None` once M3 writes them.
    let asked = if first {
        Vec::new()
    } else {
        prompt::open_threads(&threads, |_| None)
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
        .map(Path::to_path_buf);
    let temp = TempDir::new().map_err(|_| "could not create a temp dir for the run".to_string())?;
    let (cwd, workspace, conventions) = match &clone {
        Some(c) => (c.clone(), Workspace::Clone, conventions(c)),
        None => {
            write_snapshot(temp.path(), &diff, &threads)
                .map_err(|_| "could not write the review snapshot".to_string())?;
            (temp.path().to_path_buf(), Workspace::Snapshot, Vec::new())
        }
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
        let inv = invocation(&adapter, &program, &cwd, temp.path(), prompt, ctx)?;
        adapter::run(&inv, cancel).map_err(|e| e.message())
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
    Ok(output::filter(parsed, &paths, &known))
}

fn invocation(
    adapter: &Adapter,
    program: &str,
    cwd: &Path,
    temp: &Path,
    prompt: &str,
    ctx: &JobContext,
) -> Result<Invocation, String> {
    let (stdin, prompt_file) = match adapter.prompt {
        PromptDelivery::Stdin => (Some(prompt.to_string()), None),
        PromptDelivery::File(_) => {
            let file = temp.join("prompt.md");
            std::fs::write(&file, prompt).map_err(|_| "could not write the prompt".to_string())?;
            (None, Some(file))
        }
    };
    Ok(Invocation {
        program: PathBuf::from(program),
        args: adapter.args(prompt_file.as_deref(), &ctx.settings.ai.args),
        cwd: cwd.to_path_buf(),
        env: adapter::sanitized_env(std::env::vars_os(), crate::platform::login_path()),
        stdin,
        timeout: ctx.timeout,
        output: adapter.output,
    })
}
