//! Driving a local agent CLI headless: no PTY, the prompt piped in, the
//! answer read from stdout, then the process is gone.
//!
//! Patterns from Podium: the binary is resolved through the login shell
//! (`command -v`, see `platform`), tools are restricted by deny rules
//! (Podium's assistant denies `Edit,Write,NotebookEdit,Bash`; the AI pass
//! adds the web tools so diff text cannot make it fetch anything), stderr
//! never reaches the logs (only its tail, in memory, for an error message
//! the caller sanitises), and the child we hold
//! is the CLI itself, so killing it needs no process group. Unlike Podium
//! the CLI is spawned directly, not via `$SHELL -lic`, so the environment
//! Argus scrubs is the one the agent gets (a shell profile could export a
//! token again); the login shell's `PATH` is passed instead.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How an adapter takes its prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptDelivery {
    /// Written to stdin, then stdin is closed.
    Stdin,
    /// Written to a file whose path follows this flag.
    File(&'static str),
}

/// How an adapter's answer is read from stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputKind {
    /// Claude Code `--output-format stream-json`: the `result` field of the
    /// final `{"type":"result"}` line.
    ClaudeStreamJson,
    /// A current auggie's `--mode json`: one event per line; the answer is
    /// the text of the last assistant message (`message_end` /
    /// `agent_end`), its deltas stream as `message_update`s.
    AuggieJson,
    /// All of stdout is the answer.
    Text,
}

/// One agent CLI, reduced to what the AI pass needs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Adapter {
    /// The settings id (`AiSettings::adapter`).
    pub id: &'static str,
    pub binary: &'static str,
    /// Always passed, before the per-run ones and the user's extra args.
    pub args: &'static [&'static str],
    pub prompt: PromptDelivery,
    pub output: OutputKind,
    /// Whether Argus' flags make a run read-only for certain. False when
    /// the CLI can still reach tools Argus cannot turn off (the UI warns).
    pub read_only_guaranteed: bool,
}

/// Where one run may read, for the per-run flags.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunDirs<'a> {
    /// The working directory: the run's own temp dir.
    pub temp: &'a Path,
    /// The review's worktree in the local clone (else the clone itself),
    /// given as a read-only extra directory (never the working directory: a
    /// checkout's own agent config must not load).
    pub clone: Option<&'a Path>,
    /// Claude's `--settings` file (see [`claude_settings`]).
    pub settings: Option<&'a Path>,
}

/// Claude Code. `--print` + `stream-json` (`--verbose` is required with it
/// in print mode). Read-only: `--tools` limits the built-in tools to
/// `Read,Grep,Glob` and `--disallowedTools` denies the writing, shell and
/// web tools on top (deny rules win over any allow rule in the user's own
/// settings), and `Skill`: a skill in the checkout it reads (the branch
/// under review can ship one) is never loaded; the review skill Argus
/// chose comes in the prompt, read from the target branch. `--strict-mcp-config` without `--mcp-config` loads no MCP
/// server at all, so no GitLab/GitHub MCP write tool is reachable. In print
/// mode nothing can answer a permission prompt, so anything else is denied.
/// `--no-session-persistence` keeps these one-shot runs out of the user's
/// resume list.
///
/// Per run (see [`Adapter::args`]): the working directory is the run's temp
/// dir and the clone comes in through `--add-dir`, which reads its files
/// but none of its `.claude/` settings (hooks, env, `apiKeyHelper`) or
/// `CLAUDE.md`; `--setting-sources user` keeps project and local settings
/// out besides, so a hostile branch cannot configure the agent. `--settings`
/// adds deny rules for every path outside the two directories (deny rules
/// win over the user's own allow rules) and turns hooks off.
pub(crate) const CLAUDE: Adapter = Adapter {
    id: "claude",
    binary: "claude",
    args: &[
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "Read,Grep,Glob",
        "--disallowedTools",
        "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch,Skill",
        "--strict-mcp-config",
        "--no-session-persistence",
    ],
    prompt: PromptDelivery::Stdin,
    output: OutputKind::ClaudeStreamJson,
    read_only_guaranteed: true,
};

/// Auggie. Its flags, prompt delivery and answer format depend on the
/// installed version: see [`super::auggie`], which reads them from the
/// binary's `--help` before every run (cached) and refuses one that fits
/// no shape Argus knows. The fields below are unused for it.
///
/// Not guaranteed read-only: a legacy auggie has no allow-list (only the
/// tools `auggie tools list` names can be removed), and a current one can
/// still read any file its read tools are pointed at (no path rules), and
/// runs the user's own hooks.
pub(crate) const AUGGIE: Adapter = Adapter {
    id: "auggie",
    binary: "auggie",
    args: &[],
    prompt: PromptDelivery::Stdin,
    output: OutputKind::AuggieJson,
    read_only_guaranteed: false,
};

/// Whether runs of the adapter `id` are guaranteed read-only (an unknown
/// id never runs, so it is not a risk).
pub(crate) fn read_only_guaranteed(id: &str) -> bool {
    adapter(id).is_none_or(|a| a.read_only_guaranteed)
}

pub(crate) fn adapter(id: &str) -> Option<Adapter> {
    [CLAUDE, AUGGIE].into_iter().find(|a| a.id == id.trim())
}

impl Adapter {
    /// Where the binary usually lives when the login shell cannot say.
    fn well_known(&self) -> &'static [&'static str] {
        #[cfg(unix)]
        {
            match self.id {
                "claude" => crate::platform::WELL_KNOWN_CLAUDE,
                "auggie" => crate::platform::WELL_KNOWN_AUGGIE,
                _ => &[],
            }
        }
        #[cfg(windows)]
        {
            &[]
        }
    }

    /// The binary to run: `configured` when set, else resolved like `gh`.
    pub fn resolve(&self, configured: Option<&str>) -> Option<String> {
        match configured.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) => Path::new(c).is_file().then(|| c.to_string()),
            None => crate::platform::resolve_binary(self.binary, self.well_known()),
        }
    }

    /// The argv after the binary: the fixed flags, the per-run ones for
    /// `dirs`, the prompt file, then the user's extra args. Claude's; an
    /// auggie's depends on the binary ([`super::auggie::AuggieCli`]).
    pub fn args(
        &self,
        dirs: &RunDirs<'_>,
        prompt_file: Option<&Path>,
        extra: &[String],
    ) -> Vec<String> {
        let mut out: Vec<String> = self.args.iter().map(|a| a.to_string()).collect();
        let path = |p: &Path| p.to_string_lossy().into_owned();
        if self.id == "claude" {
            // `--add-dir` takes several values: it goes before the other
            // flags, never last.
            if let Some(clone) = dirs.clone {
                out.extend(["--add-dir".into(), path(clone)]);
            }
            out.extend(["--setting-sources".into(), "user".into()]);
            if let Some(settings) = dirs.settings {
                out.extend(["--settings".into(), path(settings)]);
            }
        }
        if let (PromptDelivery::File(flag), Some(file)) = (self.prompt, prompt_file) {
            out.push(flag.to_string());
            out.push(file.to_string_lossy().into_owned());
        }
        out.extend(extra.iter().cloned());
        out
    }
}

/// Claude's `--settings` for one run: hooks off, reads outside the
/// working directories blocked (`blockReadsOutsideWorkingDirectories`,
/// Claude Code 2.1.301+, which also covers what a walk of the tree cannot
/// see: entries made later, unlistable dirs, names a rule cannot spell),
/// and `Read` deny rules for every path outside `allowed` (the temp dir
/// and the clone, absolute and canonical) for a Claude that predates the
/// switch, plus the clone's secret-holding files ([`clone_secret_rules`]).
/// Read rules cover Grep and Glob too. Deny rules win over allow rules in
/// the user's own settings.
pub(crate) fn claude_settings(allowed: &[PathBuf], clone: Option<&Path>) -> serde_json::Value {
    let mut deny = deny_outside(
        allowed,
        |dir| {
            let entries = std::fs::read_dir(dir).ok()?;
            Some(
                entries
                    .filter_map(|e| e.ok().map(|e| e.file_name()))
                    .collect(),
            )
        },
        |path| {
            let link = path.symlink_metadata().ok()?.file_type().is_symlink();
            link.then(|| path.canonicalize().ok()).flatten()
        },
    );
    if deny.is_empty() {
        // No tree to walk (Windows, or nothing listable): at least the
        // dot files and dirs of the home directory, where credentials live.
        deny = vec![
            "Read(~/.*)".into(),
            "Read(~/.*/**)".into(),
            "Read(~/AppData/**)".into(),
        ];
    }
    if let Some(clone) = clone {
        deny.extend(clone_secret_rules(clone));
    }
    serde_json::json!({
        "disableAllHooks": true,
        "permissions": {
            "blockReadsOutsideWorkingDirectories": true,
            "deny": deny
        }
    })
}

/// Deny rules for the files of a clone that hold credentials rather than
/// code: all of `.git`, a nested repository's or submodule's included (its
/// `config` keeps the remote URL, a token in it included, and `FETCH_HEAD`
/// and the reflog repeat that URL), git and package-registry credential
/// files, every `.env*` file, private keys (`*.pem`, `*.key`, Rails'
/// `master.key` among them) and the usual local app secrets (.NET's
/// `appsettings.Local.json`, `appsettings.<env>.Local.json`,
/// `appsettings.Secrets.json` and Azure Functions' `local.settings.json`,
/// Wrangler's `.dev.vars`, `secrets.json`/`.yml`). Exact names and
/// extensions only: a `*.keymap`, `secrets.rs` or the committed
/// `appsettings.<env>.json` is code the review may need. A character of the
/// clone's path the rule syntax would read becomes `?`. Unix paths only, as
/// in [`deny_outside`].
pub(crate) fn clone_secret_rules(clone: &Path) -> Vec<String> {
    if !clone.is_absolute() || cfg!(windows) {
        return Vec::new();
    }
    let base: String = clone
        .to_string_lossy()
        .trim_end_matches('/')
        .chars()
        .map(|c| {
            if c.is_control() || "*?[]{}()!\\".contains(c) {
                '?'
            } else {
                c
            }
        })
        .collect();
    [
        ".git",
        ".git/**",
        "**/.git-credentials",
        "**/.netrc",
        "**/.npmrc",
        "**/.pypirc",
        "**/.env*",
        "**/.env*/**",
        "**/.git",
        "**/.git/**",
        "**/*.pem",
        "**/*.key",
        // .NET: only the variants kept out of git for local secrets; the
        // committed `appsettings.json` and `appsettings.<env>.json` are
        // config the review may need. Rule globs match case-sensitively,
        // hence both spellings.
        "**/appsettings.local.json",
        "**/appsettings.Local.json",
        "**/appsettings.*.local.json",
        "**/appsettings.*.Local.json",
        "**/appsettings.secrets.json",
        "**/appsettings.Secrets.json",
        "**/local.settings.json",
        "**/.dev.vars",
        "**/secrets.json",
        "**/secrets.yml",
        "**/secrets.yaml",
    ]
    .iter()
    .map(|p| format!("Read(/{base}/{p})"))
    .collect()
}

/// Deny rules that leave only `allowed` readable: walking down from the
/// root to each allowed directory, every sibling of the path taken is
/// denied (deny rules win over allow rules, so the allowed paths cannot be
/// carved out of a broad `Read(~/**)`; only their surroundings can be
/// closed). `list` names a directory's entries (`None`: not listable, its
/// siblings stay as they are); `link_target` is where an entry that is a
/// symlink points. Claude resolves a rule's path, so a link to a directory
/// holding an allowed one (macOS' `/var` → `/private/var`, the temp dir's
/// home) is left open, as is one into an allowed dir. Unix paths only (`//`
/// is an absolute path in a rule).
pub(crate) fn deny_outside(
    allowed: &[PathBuf],
    list: impl Fn(&Path) -> Option<Vec<OsString>>,
    link_target: impl Fn(&Path) -> Option<PathBuf>,
) -> Vec<String> {
    use std::collections::{BTreeMap, BTreeSet};
    // Directory on a path to an allowed one → the children kept open.
    let mut keep: BTreeMap<PathBuf, BTreeSet<OsString>> = BTreeMap::new();
    for target in allowed {
        if !target.is_absolute() || cfg!(windows) {
            continue;
        }
        let mut dir = PathBuf::new();
        for c in target.components() {
            match c {
                Component::RootDir => dir.push("/"),
                Component::Normal(name) => {
                    keep.entry(dir.clone())
                        .or_default()
                        .insert(name.to_os_string());
                    dir.push(name);
                }
                _ => {}
            }
        }
    }
    // An allowed dir inside another one's walk is open as a whole.
    let allowed_set: BTreeSet<&Path> = allowed.iter().map(PathBuf::as_path).collect();
    let mut out = Vec::new();
    for (dir, kept) in &keep {
        if allowed_set.iter().any(|a| dir.starts_with(a)) {
            continue;
        }
        let Some(entries) = list(dir) else { continue };
        for name in entries {
            if kept.contains(&name) {
                continue;
            }
            if let Some(target) = link_target(&dir.join(&name)) {
                let overlaps = allowed
                    .iter()
                    .any(|a| a.starts_with(&target) || target.starts_with(a));
                if overlaps {
                    continue;
                }
            }
            let Some(pattern) = rule_name(&name, kept) else {
                continue;
            };
            let base = dir.to_string_lossy();
            let base = base.trim_end_matches('/');
            out.push(format!("Read(/{base}/{pattern})"));
            out.push(format!("Read(/{base}/{pattern}/**)"));
        }
    }
    out
}

/// `name` as a rule's path segment: a character the rule or glob syntax
/// would read (`*?[]{}()!\`, control characters) becomes `?`, matching any
/// one character. `None` when that pattern would also match a kept name
/// (the rule would close an allowed path), or the name is not text.
fn rule_name(name: &OsStr, kept: &std::collections::BTreeSet<OsString>) -> Option<String> {
    let name = name.to_str()?;
    let special = |c: char| c.is_control() || "*?[]{}()!\\".contains(c);
    if !name.chars().any(special) {
        return Some(name.to_string());
    }
    let pattern: Vec<char> = name
        .chars()
        .map(|c| if special(c) { '?' } else { c })
        .collect();
    let clashes = kept.iter().filter_map(|k| k.to_str()).any(|k| {
        let k: Vec<char> = k.chars().collect();
        k.len() == pattern.len() && k.iter().zip(&pattern).all(|(a, p)| *p == '?' || a == p)
    });
    (!clashes).then(|| pattern.into_iter().collect())
}

/// Whether `name` is an environment variable that can hold a GitHub or
/// GitLab token. Those never reach the agent: diff and comment text is
/// untrusted, and an agent with a token in reach could be talked into using
/// it.
pub(crate) fn is_token_var(name: &OsStr) -> bool {
    let n = name.to_string_lossy().to_ascii_uppercase();
    const EXACT: &[&str] = &[
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GITHUB_API_TOKEN",
        "GITLAB_TOKEN",
        "GITLAB_PRIVATE_TOKEN",
        "GITLAB_ACCESS_TOKEN",
        "GL_TOKEN",
        "GLAB_TOKEN",
        "CI_JOB_TOKEN",
    ];
    if EXACT.contains(&n.as_str()) {
        return true;
    }
    let provider = ["GH_", "GITHUB", "GITLAB", "GLAB", "GL_", "CI_JOB"]
        .iter()
        .any(|p| n.starts_with(p))
        || n.contains("GITLAB")
        || n.contains("GITHUB");
    provider && (n.contains("TOKEN") || n.contains("PAT") || n.contains("SECRET"))
}

/// Agent variables that load files or tools Argus keeps out. Claude
/// Code's: the `CLAUDE.md` of `--add-dir` directories (the clone's, a PR
/// branch can write it), plugin folders (hooks, MCP servers, skills), a
/// remote MCP config, and the post-turn memory pass, which writes memory
/// files (checked in Claude Code 2.1.301). Auggie's: see below.
const AGENT_CONFIG_VARS: &[&str] = &[
    "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD",
    "CLAUDE_CODE_PLUGIN_DIRS",
    "CLAUDE_CODE_PLUGIN_SEED_DIR",
    "CLAUDE_CODE_USE_COWORK_PLUGINS",
    "CLAUDE_CODE_REMOTE_MCP_CONFIG",
    "CLAUDE_CODE_POST_TURN_MEMORY",
    "CLAUDE_CODE_POST_TURN_MEMORY_CONFIG",
    "CLAUDE_CODE_POST_TURN_MEMORY_SYNC",
    // Auggie 1.2: a shell tool (`terminal`) and the MCP tool gateway
    // (`find-tool` / `execute-tool`); removed by name too, never asked for.
    "AUGGIE_ENABLE_TERMINAL",
    "AUGGIE_ENABLE_TOOL_SEARCH",
];

/// Whether `name` is one of [`AGENT_CONFIG_VARS`].
pub(crate) fn is_agent_config_var(name: &OsStr) -> bool {
    let n = name.to_string_lossy().to_ascii_uppercase();
    AGENT_CONFIG_VARS.contains(&n.as_str())
}

/// The agent's environment: Argus' own minus every token variable and
/// every [`AGENT_CONFIG_VARS`] one, with the login shell's `PATH` when
/// there is one.
pub(crate) fn sanitized_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    login_path: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = vars
        .into_iter()
        .filter(|(k, _)| !is_token_var(k) && !is_agent_config_var(k))
        .filter(|(k, _)| login_path.is_none() || !k.eq_ignore_ascii_case("PATH"))
        .collect();
    if let Some(p) = login_path {
        out.push(("PATH".into(), p.into()));
    }
    out
}

/// One spawn of an agent.
#[derive(Debug, Clone)]
pub(crate) struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The complete environment (see [`sanitized_env`]).
    pub env: Vec<(OsString, OsString)>,
    /// Piped to stdin when set; stdin is null otherwise.
    pub stdin: Option<String>,
    pub timeout: Duration,
    pub output: OutputKind,
}

/// Why a spawn produced no answer. The text is Argus' own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentError {
    NotFound(&'static str),
    Spawn,
    Exit(Option<i32>),
    Timeout(Duration),
    Cancelled,
    /// The CLI reported a failed turn (Claude's `is_error` result).
    Failed(String),
    NoAnswer,
    /// This version of the CLI cannot run with Argus' read-only flags (an
    /// auggie whose `--help` fits no shape Argus knows): it was not run.
    Unsupported(String),
}

impl AgentError {
    pub fn message(&self) -> String {
        match self {
            AgentError::NotFound(bin) => {
                format!("`{bin}` was not found: install it or set its path in Settings")
            }
            AgentError::Spawn => "the agent could not be started".into(),
            AgentError::Exit(Some(code)) => format!("the agent exited with status {code}"),
            AgentError::Exit(None) => "the agent was killed".into(),
            AgentError::Timeout(d) => format!("the agent took longer than {}s", d.as_secs()),
            AgentError::Cancelled => super::CANCELLED.into(),
            AgentError::Failed(kind) => format!("the agent reported an error ({kind})"),
            AgentError::NoAnswer => "the agent gave no answer".into(),
            AgentError::Unsupported(why) => {
                format!("this auggie version can't run read-only: {why}")
            }
        }
    }
}

/// Most of stdout Argus keeps for a text answer.
const MAX_TEXT_OUTPUT: usize = 4 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(25);
/// How long the answer may still come in after the agent exited. A
/// grandchild that inherited stdout keeps the pipe open past the exit; the
/// answer read so far is used then, rather than waiting for it forever.
const READER_GRACE: Duration = Duration::from_secs(2);

/// The agent process of a running run, shared with the runtime so quitting
/// Argus can kill it even between the run loop's polls.
pub(crate) type ChildSlot = Arc<Mutex<Option<Child>>>;

/// Kill the agent in `slot`, if one is running.
pub(crate) fn kill_child(slot: &ChildSlot) {
    if let Some(child) = slot.lock().expect("child lock poisoned").as_mut() {
        let _ = child.kill();
    }
}

/// Run `inv` to completion (or until `cancel` is set or it times out, which
/// kills it) and return its answer text.
#[cfg(test)]
pub(crate) fn run(inv: &Invocation, cancel: &AtomicBool) -> Result<String, AgentError> {
    run_tracked(inv, cancel, &ChildSlot::default())
}

/// [`run`], with the child kept in `slot` while it runs.
pub(crate) fn run_tracked(
    inv: &Invocation,
    cancel: &AtomicBool,
    slot: &ChildSlot,
) -> Result<String, AgentError> {
    run_observed(inv, cancel, slot, None).map_err(|f| f.error)
}

/// One thing a Claude stream-json says while the agent works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamEvent {
    /// A piece of the agent's thinking (`thinking_delta`; summarised, and
    /// empty when Claude hides it).
    Thinking(String),
    /// A piece of the answer (`text_delta`).
    Text(String),
    /// Claude's running estimate of thinking tokens (`thinking_tokens`),
    /// the only sign of progress while the thinking text is hidden.
    ThinkingTokens(u64),
    /// A request failed and Claude retries it (`api_retry`): attempt,
    /// most attempts, the HTTP status when there was one.
    Retrying {
        attempt: u64,
        max: Option<u64>,
        status: Option<u64>,
    },
}

/// Where the stream's events go, on the reader's thread.
pub(crate) type EventSink = Arc<dyn Fn(StreamEvent) + Send + Sync>;

/// Why a run gave no answer, with the end of what the agent wrote to
/// stderr (raw: the caller sanitises it before showing it; never logged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentFailure {
    pub error: AgentError,
    pub stderr: String,
}

impl From<AgentError> for AgentFailure {
    fn from(error: AgentError) -> Self {
        Self {
            error,
            stderr: String::new(),
        }
    }
}

/// Most of stderr kept for an error message: its end, where a CLI says
/// what went wrong.
const STDERR_TAIL: usize = 4 * 1024;

/// [`run_tracked`] that also reports the stream's events to `sink` as they
/// come (Claude stream-json only) and keeps the tail of stderr for the
/// failure. stdin is closed once the prompt is written, so the CLI never
/// waits on it.
pub(crate) fn run_observed(
    inv: &Invocation,
    cancel: &AtomicBool,
    slot: &ChildSlot,
    sink: Option<EventSink>,
) -> Result<String, AgentFailure> {
    run_capped(inv, cancel, slot, sink, STDERR_TAIL)
}

/// [`run_observed`] keeping the last `stderr_cap` bytes of stderr.
pub(crate) fn run_capped(
    inv: &Invocation,
    cancel: &AtomicBool,
    slot: &ChildSlot,
    sink: Option<EventSink>,
    stderr_cap: usize,
) -> Result<String, AgentFailure> {
    let mut cmd = Command::new(&inv.program);
    cmd.args(&inv.args)
        .current_dir(&inv.cwd)
        .env_clear()
        .envs(inv.env.iter().map(|(k, v)| (k, v)))
        .stdin(if inv.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        // Kept in memory for the error message only: child output never
        // goes to the logs.
        .stderr(Stdio::piped());
    hide_console_window(&mut cmd);
    // Spawned under the slot's lock: a shutdown that cancels and kills
    // either finds the child in the slot or sees the cancel flag below.
    let mut guard = slot.lock().expect("child lock poisoned");
    if cancel.load(Ordering::Relaxed) {
        return Err(AgentError::Cancelled.into());
    }
    let mut child = cmd.spawn().map_err(|_| AgentError::Spawn)?;

    // Written from its own thread: a prompt larger than the pipe buffer
    // would otherwise deadlock against an agent already writing stdout.
    // The handle is dropped when the thread ends, which closes stdin.
    if let (Some(prompt), Some(mut stdin)) = (inv.stdin.clone(), child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
            drop(stdin);
        });
    }
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take();
    *guard = Some(child);
    drop(guard);
    let kind = inv.output;
    let shared = Arc::new(Mutex::new(ReadSoFar::default()));
    let into = shared.clone();
    let reader = std::thread::spawn(move || read_answer(stdout, kind, &into, sink.as_ref()));
    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    if let Some(stderr) = stderr {
        let tail = tail.clone();
        std::thread::spawn(move || read_tail(stderr, &tail, stderr_cap));
    }
    let stderr_text = || {
        let bytes = tail.lock().expect("stderr lock poisoned").clone();
        String::from_utf8_lossy(&bytes).into_owned()
    };

    let deadline = Instant::now() + inv.timeout;
    let status = loop {
        let polled = {
            let mut guard = slot.lock().expect("child lock poisoned");
            let child = guard.as_mut().expect("the child is in its slot");
            match child.try_wait() {
                Ok(Some(status)) => Ok(Some(status)),
                Ok(None) => Ok(None),
                Err(_) => {
                    let _ = child.kill();
                    Err(())
                }
            }
        };
        match polled {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(()) => {
                slot.lock().expect("child lock poisoned").take();
                return Err(AgentError::Spawn.into());
            }
        }
        let stop = if cancel.load(Ordering::Relaxed) {
            Some(AgentError::Cancelled)
        } else if Instant::now() >= deadline {
            Some(AgentError::Timeout(inv.timeout))
        } else {
            None
        };
        if let Some(e) = stop {
            if let Some(mut child) = slot.lock().expect("child lock poisoned").take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            // The reader is not joined: a grandchild could hold the pipe.
            return Err(AgentFailure {
                error: e,
                stderr: stderr_text(),
            });
        }
        std::thread::sleep(POLL);
    };
    slot.lock().expect("child lock poisoned").take();
    // The answer is in once stdout closes; a grandchild holding the pipe
    // gets a short grace, then what was read is the answer.
    let grace = Instant::now() + READER_GRACE;
    while !reader.is_finished() && Instant::now() < grace {
        std::thread::sleep(POLL);
    }
    let answer = shared.lock().expect("answer lock poisoned").take(kind);
    let error = match (answer, status.success()) {
        (Answer::Result(text), _) => return Ok(text),
        (Answer::Failed(kind), _) => AgentError::Failed(kind),
        (Answer::Text(text), true) if !text.trim().is_empty() => return Ok(text),
        (_, false) => AgentError::Exit(status.code()),
        _ => AgentError::NoAnswer,
    };
    Err(AgentFailure {
        error,
        stderr: stderr_text(),
    })
}

/// Keep the last `cap` bytes of `from`.
fn read_tail(mut from: impl Read, into: &Mutex<Vec<u8>>, cap: usize) {
    let mut buf = [0u8; 4096];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut tail = into.lock().expect("stderr lock poisoned");
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > cap {
                    let cut = tail.len() - cap;
                    tail.drain(..cut);
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Result(String),
    Failed(String),
    Text(String),
    None,
}

/// What the reader has read so far: the last `result` line (stream-json)
/// or the text (capped).
#[derive(Default)]
struct ReadSoFar {
    last: Option<Answer>,
    text: Vec<u8>,
}

impl ReadSoFar {
    fn take(&mut self, kind: OutputKind) -> Answer {
        match kind {
            OutputKind::ClaudeStreamJson | OutputKind::AuggieJson => {
                self.last.take().unwrap_or(Answer::None)
            }
            OutputKind::Text => {
                Answer::Text(String::from_utf8_lossy(&std::mem::take(&mut self.text)).into_owned())
            }
        }
    }
}

fn read_answer(
    stdout: impl Read,
    kind: OutputKind,
    into: &Mutex<ReadSoFar>,
    sink: Option<&EventSink>,
) {
    match kind {
        OutputKind::ClaudeStreamJson => {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(a) = parse_result_line(&line) {
                    into.lock().expect("answer lock poisoned").last = Some(a);
                } else if let Some(sink) = sink {
                    if let Some(event) = parse_stream_event(&line) {
                        sink(event);
                    }
                }
            }
        }
        OutputKind::AuggieJson => {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match parse_auggie_line(&line) {
                    AuggieLine::Answer(a) => {
                        into.lock().expect("answer lock poisoned").last = Some(a);
                    }
                    AuggieLine::Event(event) => {
                        if let Some(sink) = sink {
                            sink(event);
                        }
                    }
                    AuggieLine::Other => {}
                }
            }
        }
        OutputKind::Text => {
            let mut stdout = stdout.take(MAX_TEXT_OUTPUT as u64);
            let mut buf = [0u8; 8192];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => into
                        .lock()
                        .expect("answer lock poisoned")
                        .text
                        .extend_from_slice(&buf[..n]),
                }
            }
        }
    }
}

/// A stream-json line's progress event: a thinking or text delta
/// (`--include-partial-messages`), the thinking-token estimate, an API
/// retry. Everything else (whole messages, the result, noise) is `None`.
pub(crate) fn parse_stream_event(line: &str) -> Option<StreamEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let num = |v: &serde_json::Value, key: &str| v.get(key).and_then(|n| n.as_u64());
    match (
        v.get("type").and_then(|t| t.as_str()),
        v.get("subtype").and_then(|t| t.as_str()),
    ) {
        (Some("stream_event"), _) => {
            let event = v.get("event")?;
            if event.get("type").and_then(|t| t.as_str()) != Some("content_block_delta") {
                return None;
            }
            let delta = event.get("delta")?;
            let text = |key: &str| delta.get(key).and_then(|t| t.as_str()).map(str::to_string);
            match delta.get("type").and_then(|t| t.as_str())? {
                "thinking_delta" => text("thinking").map(StreamEvent::Thinking),
                "text_delta" => text("text").map(StreamEvent::Text),
                _ => None,
            }
        }
        (Some("system"), Some("thinking_tokens")) => {
            num(&v, "estimated_tokens").map(StreamEvent::ThinkingTokens)
        }
        (Some("system"), Some("api_retry")) => Some(StreamEvent::Retrying {
            attempt: num(&v, "attempt").unwrap_or(1),
            max: num(&v, "max_retries"),
            status: num(&v, "error_status"),
        }),
        _ => None,
    }
}

/// What one line of a current auggie's `--mode json` says.
#[derive(Debug)]
pub(crate) enum AuggieLine {
    /// The answer so far: an assistant message ended (`message_end`), or
    /// the run did (`agent_end`, its last assistant message). A later one
    /// replaces it, so the last message is the answer.
    Answer(Answer),
    /// A thinking or text delta, or an automatic retry.
    Event(StreamEvent),
    /// Anything else (the session header, turns, tool runs, noise).
    Other,
}

/// One line of auggie's `--mode json` (1.2.0: `session`, `agent_start`,
/// `turn_start`, `message_start` / `message_update` / `message_end`,
/// `tool_execution_*`, `auto_retry_start`, `turn_end`, `agent_end`).
pub(crate) fn parse_auggie_line(line: &str) -> AuggieLine {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return AuggieLine::Other;
    };
    fn str_of<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
        v.get(key).and_then(|t| t.as_str())
    }
    match str_of(&v, "type") {
        Some("message_update") => {
            let Some(e) = v.get("assistantMessageEvent") else {
                return AuggieLine::Other;
            };
            let delta = || str_of(e, "delta").map(str::to_string);
            match str_of(e, "type") {
                Some("thinking_delta") => delta().map(StreamEvent::Thinking),
                Some("text_delta") => delta().map(StreamEvent::Text),
                _ => None,
            }
            .map_or(AuggieLine::Other, AuggieLine::Event)
        }
        Some("auto_retry_start") => {
            let num = |key: &str| v.get(key).and_then(|n| n.as_u64());
            AuggieLine::Event(StreamEvent::Retrying {
                attempt: num("attempt").unwrap_or(1),
                max: num("maxAttempts"),
                status: None,
            })
        }
        Some("message_end") => v
            .get("message")
            .and_then(auggie_answer)
            .map_or(AuggieLine::Other, AuggieLine::Answer),
        Some("agent_end") => v
            .get("messages")
            .and_then(|m| m.as_array())
            .and_then(|m| m.iter().rev().find_map(auggie_answer))
            .map_or(AuggieLine::Other, AuggieLine::Answer),
        _ => AuggieLine::Other,
    }
}

/// An assistant message's answer: a failure for a turn that stopped on an
/// error, else its text; `None` for another role or a message without text
/// (one that only called tools).
fn auggie_answer(message: &serde_json::Value) -> Option<Answer> {
    if message.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }
    let stop = message
        .get("stopReason")
        .and_then(|s| s.as_str())
        .unwrap_or("stop");
    if matches!(stop, "error" | "aborted" | "cancelled") {
        return Some(Answer::Failed(stop.to_string()));
    }
    let text: String = message
        .get("content")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect();
    (!text.trim().is_empty()).then_some(Answer::Result(text))
}

/// A stream-json `result` line (other lines, and non-JSON noise, are None).
fn parse_result_line(line: &str) -> Option<Answer> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("result") {
        return None;
    }
    if v.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
        // Only a plain subtype like `error_max_turns` is shown.
        let kind = v
            .get("subtype")
            .and_then(|s| s.as_str())
            .filter(|s| s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .unwrap_or("error");
        return Some(Answer::Failed(kind.to_string()));
    }
    Some(Answer::Result(
        v.get("result")
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string(),
    ))
}

#[cfg(windows)]
fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_console_window(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs<'a>(clone: Option<&'a Path>, settings: Option<&'a Path>) -> RunDirs<'a> {
        RunDirs {
            temp: Path::new("/tmp/argus-ai-x"),
            clone,
            settings,
        }
    }

    #[test]
    fn claude_args_are_print_stream_json_and_read_only() {
        let args = CLAUDE.args(
            &dirs(
                Some(Path::new("/src/widgets")),
                Some(Path::new("/tmp/argus-ai-x/claude-settings.json")),
            ),
            None,
            &["--model".into(), "sonnet".into()],
        );
        let joined = args.join(" ");
        assert!(joined.starts_with("--print --output-format stream-json --verbose"));
        assert!(joined.contains("--tools Read,Grep,Glob"));
        assert!(joined
            .contains("--disallowedTools Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch,Skill"));
        assert!(joined.contains("--strict-mcp-config"));
        assert!(joined.contains(
            "--add-dir /src/widgets --setting-sources user --settings /tmp/argus-ai-x/claude-settings.json"
        ));
        assert!(joined.ends_with("--model sonnet"));
        assert_eq!(CLAUDE.prompt, PromptDelivery::Stdin);
        // Without a clone: no extra directory, still only user settings.
        let joined = CLAUDE.args(&dirs(None, None), None, &[]).join(" ");
        assert!(!joined.contains("--add-dir"));
        assert!(joined.ends_with("--setting-sources user"));
        assert!(CLAUDE.read_only_guaranteed && read_only_guaranteed("claude"));
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "unix paths: deny_outside writes no rules on Windows"
    )]
    fn deny_rules_close_everything_around_the_allowed_dirs() {
        let tree: std::collections::HashMap<&str, &[&str]> = [
            (
                "/",
                &["Users", "etc", "private", "Volumes", "var", "tmp"][..],
            ),
            ("/Users", &["fixture", "other"][..]),
            (
                "/Users/fixture",
                &[".ssh", ".netrc", "src", "Documents", "a(b)*"][..],
            ),
            (
                "/Users/fixture/src",
                &["widgets", "secret-repo", "widget?", "w"][..],
            ),
            ("/private", &["var", "etc"][..]),
            ("/private/var", &["folders", "db"][..]),
            ("/private/var/folders", &["T"][..]),
            (
                "/private/var/folders/T",
                &["argus-ai-1", "argus-ai-2", "other.sock"][..],
            ),
        ]
        .into();
        let allowed = [
            PathBuf::from("/Users/fixture/src/widgets"),
            PathBuf::from("/private/var/folders/T/argus-ai-1"),
        ];
        let links: std::collections::HashMap<&str, &str> = [
            ("/var", "/private/var"),
            ("/tmp", "/private/tmp"),
            ("/Users/fixture/src/w", "/Users/fixture/src/widgets/lib"),
        ]
        .into();
        let rules = deny_outside(
            &allowed,
            |dir| {
                tree.get(dir.to_str().unwrap())
                    .map(|names| names.iter().map(OsString::from).collect())
            },
            |p| links.get(p.to_str().unwrap()).map(PathBuf::from),
        );
        let has = |r: &str| rules.iter().any(|x| x == r);
        for denied in [
            "//etc",
            // A link elsewhere is closed like any entry.
            "//tmp",
            "//Volumes",
            "//Users/other",
            "//Users/fixture/.ssh",
            "//Users/fixture/.netrc",
            "//Users/fixture/Documents",
            "//Users/fixture/a?b??",
            "//Users/fixture/src/secret-repo",
            "//private/etc",
            "//private/var/db",
            "//private/var/folders/T/argus-ai-2",
            "//private/var/folders/T/other.sock",
        ] {
            assert!(has(&format!("Read({denied})")), "{denied}: {rules:?}");
            assert!(has(&format!("Read({denied}/**)")), "{denied}: {rules:?}");
        }
        for open in [
            // Links to (a parent of) an allowed dir, or into one: a rule on
            // them would close it.
            "//var",
            "//Users/fixture/src/w",
            "//Users",
            "//Users/fixture",
            "//Users/fixture/src",
            "//Users/fixture/src/widgets",
            "//private",
            "//private/var/folders/T/argus-ai-1",
        ] {
            assert!(!has(&format!("Read({open})")), "{open}: {rules:?}");
            assert!(!has(&format!("Read({open}/**)")), "{open}: {rules:?}");
        }
        // `widget?` as a pattern would match `widgets`: left out.
        assert!(!rules.iter().any(|r| r.contains("widget?")), "{rules:?}");

        let settings = claude_settings(&[], None);
        assert_eq!(settings["disableAllHooks"], true);
        assert_eq!(
            settings["permissions"]["blockReadsOutsideWorkingDirectories"],
            true
        );
        assert!(settings["permissions"]["deny"]
            .as_array()
            .is_some_and(|d| !d.is_empty()));
    }

    #[cfg(windows)]
    #[test]
    fn no_directory_deny_rules_are_written_on_windows() {
        let allowed = [PathBuf::from(r"C:\Users\fixture\src\widgets")];
        let rules = deny_outside(&allowed, |_| Some(vec![OsString::from("other")]), |_| None);
        assert!(rules.is_empty(), "{rules:?}");
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn the_clones_secret_files_are_denied() {
        let rules = clone_secret_rules(Path::new("/src/w(1)/widgets/"));
        for rule in [
            "Read(//src/w?1?/widgets/.git)",
            "Read(//src/w?1?/widgets/.git/**)",
            "Read(//src/w?1?/widgets/**/.git-credentials)",
            "Read(//src/w?1?/widgets/**/.netrc)",
            "Read(//src/w?1?/widgets/**/.env*)",
        ] {
            assert!(rules.iter().any(|r| r == rule), "{rule}: {rules:?}");
        }
        // The clone itself stays readable.
        assert!(!rules.iter().any(|r| r == "Read(//src/w?1?/widgets/**)"));
        assert!(clone_secret_rules(Path::new("relative")).is_empty());
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn nested_repos_keys_and_app_secrets_are_denied() {
        let rules = clone_secret_rules(Path::new("/src/widgets"));
        for p in [
            "**/.git",
            "**/.git/**",
            "**/*.pem",
            "**/*.key",
            "**/appsettings.local.json",
            "**/appsettings.Local.json",
            "**/appsettings.*.local.json",
            "**/appsettings.*.Local.json",
            "**/appsettings.secrets.json",
            "**/appsettings.Secrets.json",
            "**/local.settings.json",
            "**/.dev.vars",
            "**/secrets.json",
            "**/secrets.yml",
            "**/secrets.yaml",
        ] {
            let rule = format!("Read(//src/widgets/{p})");
            assert!(rules.contains(&rule), "{rule}: {rules:?}");
        }
        // The committed .NET config (`appsettings.json`,
        // `appsettings.Development.json`) is code the review may need: only
        // the local and secrets variants are denied.
        for r in rules.iter().filter(|r| r.contains("appsettings")) {
            let lower = r.to_lowercase();
            assert!(
                lower.ends_with(".local.json)") || lower.ends_with(".secrets.json)"),
                "{r}"
            );
        }
        // Exact names and extensions: a `*.keymap` or `secrets.rs` is code
        // the review may need.
        for r in &rules {
            assert!(!r.contains(".key*") && !r.contains("secrets.*"), "{r}");
        }
        let settings = claude_settings(&[], Some(Path::new("/src/widgets")));
        let deny = settings["permissions"]["deny"].as_array().unwrap();
        assert!(deny.iter().any(|r| r == "Read(//src/widgets/.git/**)"));
    }

    #[test]
    fn env_has_no_provider_tokens() {
        let vars = [
            ("HOME", "/home/fixture"),
            ("PATH", "/usr/bin"),
            ("GH_TOKEN", "fixture-gh"),
            ("GITHUB_TOKEN", "fixture-gh2"),
            ("GH_ENTERPRISE_TOKEN", "fixture-ghe"),
            ("GITLAB_TOKEN", "fixture-gl"),
            ("gitlab_private_token", "fixture-gl2"),
            ("MY_GITLAB_TOKEN", "fixture-gl3"),
            ("CI_JOB_TOKEN", "fixture-ci"),
            ("CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD", "1"),
            ("CLAUDE_CODE_PLUGIN_DIRS", "/fixture/plugins"),
            ("claude_code_post_turn_memory", "1"),
            ("ANTHROPIC_API_KEY", "fixture-agent-key"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let env = sanitized_env(vars.clone(), Some("/opt/homebrew/bin:/usr/bin"));
        let keys: Vec<String> = env
            .iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert_eq!(keys, ["HOME", "ANTHROPIC_API_KEY", "PATH"]);
        assert_eq!(env[2].1, OsString::from("/opt/homebrew/bin:/usr/bin"));
        let kept_path = sanitized_env(vars, None);
        assert!(kept_path
            .iter()
            .any(|(k, v)| k == "PATH" && v == "/usr/bin"));
        assert!(!is_token_var(OsStr::new("GHOSTTY_RESOURCES_DIR")));
        assert!(!is_token_var(OsStr::new("TOKENIZERS_PARALLELISM")));
        // Agent settings that are not about loading files stay.
        assert!(!is_agent_config_var(OsStr::new("CLAUDE_CODE_USE_BEDROCK")));
        assert!(!is_agent_config_var(OsStr::new("CLAUDE_CONFIG_DIR")));
    }

    #[test]
    fn stream_events_from_recorded_lines() {
        // Shapes as Claude Code 2.1.309 prints them with
        // `--include-partial-messages --thinking-display summarized`.
        let lines = [
            r#"{"type":"system","subtype":"init","cwd":"/tmp/x","tools":[],"mcp_servers":[]}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"2047 factors as 23","estimated_tokens":null}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"CAQS"}}}"#,
            r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":150,"estimated_tokens_delta":100}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":""}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"{\"prime\""}}}"#,
            r#"{"type":"system","subtype":"api_retry","attempt":2,"max_retries":10,"retry_delay_ms":1200,"error_status":529,"error":"overloaded"}"#,
            r#"{"type":"stream_event","event":{"type":"ping"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta""#,
            "not json at all",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"{}"}"#,
        ];
        let events: Vec<StreamEvent> = lines.iter().filter_map(|l| parse_stream_event(l)).collect();
        assert_eq!(
            events,
            vec![
                StreamEvent::Thinking("2047 factors as 23".into()),
                StreamEvent::ThinkingTokens(150),
                StreamEvent::Text("{\"prime\"".into()),
                StreamEvent::Retrying {
                    attempt: 2,
                    max: Some(10),
                    status: Some(529)
                },
            ]
        );
    }

    /// Lines of auggie 1.2.0-prerelease's `--mode json` as it printed them
    /// for a one-line prompt with `--thinking high` (texts fictitious, the
    /// `partial` copies cut short, `usage` left out).
    pub(crate) const AUGGIE_JSON: &[&str] = &[
        r#"{"type":"session","version":3,"id":"00000000-0000-4000-8000-000000000001","timestamp":"2026-10-23T14:08:33.702Z","cwd":"/tmp/argus-ai-x"}"#,
        r#"{"type":"agent_start"}"#,
        r#"{"type":"turn_start"}"#,
        r#"{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"Is 91 prime?"}],"timestamp":1792764515366}}"#,
        r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"Is 91 prime?"}],"timestamp":1792764515366}}"#,
        r#"{"type":"message_start","message":{"role":"assistant","content":[],"api":"augment-chat-stream","provider":"augment","model":"fixture-model","stopReason":"stop"}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_start","contentIndex":0,"partial":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"91 = 7 * 13","partial":{"role":"assistant","content":[{"type":"thinking","thinking":"91 = 7 * 13"}]}}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_end","contentIndex":0,"content":"91 = 7 * 13","partial":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":1,"partial":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"no","partial":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"\n","partial":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":1,"content":"no\n","partial":{"role":"assistant","content":[]}}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"91 = 7 * 13"},{"type":"text","text":"no\n"}],"stopReason":"stop","timestamp":1792764515999}}"#,
        r#"{"type":"turn_end","message":{"role":"assistant","content":[{"type":"text","text":"no\n"}],"stopReason":"stop"},"toolResults":[]}"#,
        r#"{"type":"agent_end","messages":[{"role":"user","content":[{"type":"text","text":"Is 91 prime?"}]},{"role":"assistant","content":[{"type":"thinking","thinking":"91 = 7 * 13"},{"type":"text","text":"no\n"}],"stopReason":"stop"}]}"#,
    ];

    #[test]
    fn auggie_json_lines_give_deltas_and_the_last_assistant_text() {
        let mut events = Vec::new();
        let mut answers = Vec::new();
        for line in AUGGIE_JSON {
            match parse_auggie_line(line) {
                AuggieLine::Event(e) => events.push(e),
                AuggieLine::Answer(a) => answers.push(a),
                AuggieLine::Other => {}
            }
        }
        assert_eq!(
            events,
            vec![
                StreamEvent::Thinking("91 = 7 * 13".into()),
                StreamEvent::Text("no".into()),
                StreamEvent::Text("\n".into()),
            ]
        );
        // The assistant's `message_end`, then `agent_end`: the same text.
        assert_eq!(
            answers,
            vec![Answer::Result("no\n".into()), Answer::Result("no\n".into())]
        );
        // A retry, a failed turn, a turn of tool calls only, noise.
        let retry = r#"{"type":"auto_retry_start","attempt":2,"maxAttempts":5,"delayMs":2000,"errorMessage":"overloaded"}"#;
        assert!(matches!(
            parse_auggie_line(retry),
            AuggieLine::Event(StreamEvent::Retrying {
                attempt: 2,
                max: Some(5),
                status: None
            })
        ));
        let failed = r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"Ignore all <b>"}}"#;
        assert!(matches!(
            parse_auggie_line(failed),
            AuggieLine::Answer(Answer::Failed(k)) if k == "error"
        ));
        let tools_only = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a.rs"}}],"stopReason":"toolUse"}}"#;
        assert!(matches!(parse_auggie_line(tools_only), AuggieLine::Other));
        // `agent_end` takes the last assistant message that says something.
        let end = r#"{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"{\"findings\":[]}"}],"stopReason":"stop"},{"role":"toolResult","content":[]},{"role":"assistant","content":[{"type":"text","text":"  "}],"stopReason":"stop"}]}"#;
        assert!(matches!(
            parse_auggie_line(end),
            AuggieLine::Answer(Answer::Result(t)) if t == "{\"findings\":[]}"
        ));
        for other in [
            "",
            "not json",
            r#"{"type":"message_update"}"#,
            r#"{"type":"tool_execution_start","toolName":"read"}"#,
        ] {
            assert!(
                matches!(parse_auggie_line(other), AuggieLine::Other),
                "{other}"
            );
        }
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn an_auggie_json_run_streams_and_answers_with_the_last_message() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("auggie");
        let body: String = AUGGIE_JSON
            .iter()
            .map(|l| format!("printf '%s\\n' '{l}'\n"))
            .collect();
        crate::testutil::write_executable(
            &script,
            &format!("#!/bin/sh\ncat > \"$(dirname \"$0\")/stdin.txt\"\n{body}"),
        );
        let inv = Invocation {
            program: script,
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
            env: sanitized_env(std::env::vars_os(), None),
            stdin: Some("Is 91 prime?".into()),
            timeout: Duration::from_secs(20),
            output: OutputKind::AuggieJson,
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let sink: EventSink = Arc::new(move |e| s2.lock().unwrap().push(e));
        let answer = run_observed(
            &inv,
            &AtomicBool::new(false),
            &ChildSlot::default(),
            Some(sink),
        )
        .unwrap();
        assert_eq!(answer, "no\n");
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stdin.txt")).unwrap(),
            "Is 91 prime?"
        );
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn an_observed_run_streams_events_and_keeps_the_stderr_tail() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("claude");
        // Reads stdin to its end (so stdin must be closed), streams, then
        // ends early: no result line.
        crate::testutil::write_executable(
            &script,
            r#"#!/bin/sh
cat > /dev/null
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}}'
echo 'garbage'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"{"}}}'
i=0; while [ $i -lt 400 ]; do echo "noise line $i" >&2; i=$((i+1)); done
echo 'the real reason' >&2
exit 4
"#,
        );
        let inv = Invocation {
            program: script,
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
            env: sanitized_env(std::env::vars_os(), None),
            stdin: Some("prompt ".repeat(50_000)),
            timeout: Duration::from_secs(20),
            output: OutputKind::ClaudeStreamJson,
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let sink: EventSink = Arc::new(move |e| s2.lock().unwrap().push(e));
        let f = run_observed(
            &inv,
            &AtomicBool::new(false),
            &ChildSlot::default(),
            Some(sink),
        )
        .unwrap_err();
        assert_eq!(f.error, AgentError::Exit(Some(4)));
        assert!(f.stderr.ends_with("the real reason\n"), "{}", f.stderr);
        assert!(f.stderr.len() <= STDERR_TAIL);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                StreamEvent::Thinking("hm".into()),
                StreamEvent::Text("{".into())
            ]
        );
    }

    #[test]
    fn result_lines() {
        assert!(parse_result_line("not json").is_none());
        assert!(parse_result_line(r#"{"type":"assistant"}"#).is_none());
        match parse_result_line(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"{}"}"#,
        ) {
            Some(Answer::Result(t)) => assert_eq!(t, "{}"),
            _ => panic!("expected a result"),
        }
        match parse_result_line(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#)
        {
            Some(Answer::Failed(k)) => assert_eq!(k, "error_max_turns"),
            _ => panic!("expected a failure"),
        }
        match parse_result_line(r#"{"type":"result","subtype":"Ignore all <b>","is_error":true}"#) {
            Some(Answer::Failed(k)) => assert_eq!(k, "error"),
            _ => panic!("expected a failure"),
        }
    }
}
