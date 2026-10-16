//! Driving a local agent CLI headless: no PTY, the prompt piped in, the
//! answer read from stdout, then the process is gone.
//!
//! Patterns from Podium: the binary is resolved through the login shell
//! (`command -v`, see `platform`), tools are restricted by deny rules
//! (Podium's assistant denies `Edit,Write,NotebookEdit,Bash`; the AI pass
//! adds the web tools so diff text cannot make it fetch anything), stderr
//! is dropped (child output never reaches the logs), and the child we hold
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
    /// The local clone, given as a read-only extra directory (never the
    /// working directory: a checkout's own agent config must not load).
    pub clone: Option<&'a Path>,
    /// Claude's `--settings` file (see [`claude_settings`]).
    pub settings: Option<&'a Path>,
}

/// Claude Code. `--print` + `stream-json` (`--verbose` is required with it
/// in print mode). Read-only: `--tools` limits the built-in tools to
/// `Read,Grep,Glob` and `--disallowedTools` denies the writing, shell and
/// web tools on top (deny rules win over any allow rule in the user's own
/// settings). `--strict-mcp-config` without `--mcp-config` loads no MCP
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
        "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch",
        "--strict-mcp-config",
        "--no-session-persistence",
    ],
    prompt: PromptDelivery::Stdin,
    output: OutputKind::ClaudeStreamJson,
    read_only_guaranteed: true,
};

/// Auggie. `--print --quiet` prints only the final message; `--ask` asks
/// for retrieval and non-editing tools only; `--dont-save-session` as
/// above. Its instruction comes from a file, so a long prompt never hits
/// the argv limit.
///
/// Not guaranteed read-only: `--ask` only adds an instruction to the prompt
/// (auggie 0.37), and auggie has no switch to drop the user's MCP servers,
/// whose tools are named by the user's config. Per run Argus removes every
/// built-in tool that writes, runs a process, reaches the web or starts a
/// sub-agent ([`AUGGIE_REMOVED_TOOLS`]); the workspace root is the temp dir
/// (so no `.augment/` settings of the clone load) and the clone is an extra
/// workspace.
pub(crate) const AUGGIE: Adapter = Adapter {
    id: "auggie",
    binary: "auggie",
    args: &["--print", "--quiet", "--ask", "--dont-save-session"],
    prompt: PromptDelivery::File("--instruction-file"),
    output: OutputKind::Text,
    read_only_guaranteed: false,
};

/// Auggie's built-in tools (`auggie tools list`, 0.37) that write files,
/// run or drive processes, reach the web, start sub-agents or edit its task
/// list; each goes out with `--remove-tool` (an unknown name is ignored).
pub(crate) const AUGGIE_REMOVED_TOOLS: &[&str] = &[
    "remove-files",
    "save-file",
    "apply_patch",
    "str-replace-editor",
    "launch-process",
    "kill-process",
    "read-process",
    "write-process",
    "list-processes",
    "web-search",
    "web-fetch",
    "sub-agent-explore",
    "sub-agent-plan",
    "sub-agent-general-purpose",
    "sub-agent-research",
    "sub-agent-code",
    "sub-agent-validate",
    "reorganize_tasklist",
    "update_tasks",
    "add_tasks",
    "ask-user",
    "enter-plan-mode",
    "exit-plan-mode",
];

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
    /// `dirs`, the prompt file, then the user's extra args.
    pub fn args(
        &self,
        dirs: &RunDirs<'_>,
        prompt_file: Option<&Path>,
        extra: &[String],
    ) -> Vec<String> {
        let mut out: Vec<String> = self.args.iter().map(|a| a.to_string()).collect();
        let path = |p: &Path| p.to_string_lossy().into_owned();
        match self.id {
            "claude" => {
                // `--add-dir` takes several values: it goes before the
                // other flags, never last.
                if let Some(clone) = dirs.clone {
                    out.extend(["--add-dir".into(), path(clone)]);
                }
                out.extend(["--setting-sources".into(), "user".into()]);
                if let Some(settings) = dirs.settings {
                    out.extend(["--settings".into(), path(settings)]);
                }
            }
            "auggie" => {
                out.extend(["--workspace-root".into(), path(dirs.temp)]);
                if let Some(clone) = dirs.clone {
                    out.extend(["--add-workspace".into(), path(clone)]);
                }
                for tool in AUGGIE_REMOVED_TOOLS {
                    out.extend(["--remove-tool".into(), tool.to_string()]);
                }
            }
            _ => {}
        }
        if let (PromptDelivery::File(flag), Some(file)) = (self.prompt, prompt_file) {
            out.push(flag.to_string());
            out.push(file.to_string_lossy().into_owned());
        }
        out.extend(extra.iter().cloned());
        out
    }
}

/// Claude's `--settings` for one run: hooks off, and `Read` deny rules for
/// every path outside `allowed` (the temp dir and the clone, absolute and
/// canonical). Read rules cover Grep and Glob too. Reads outside the
/// working directories are refused in print mode anyway; the rules make
/// that hold against allow rules in the user's own settings as well.
pub(crate) fn claude_settings(allowed: &[PathBuf]) -> serde_json::Value {
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
    serde_json::json!({
        "disableAllHooks": true,
        "permissions": { "deny": deny }
    })
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

/// The agent's environment: Argus' own minus every token variable, with the
/// login shell's `PATH` when there is one.
pub(crate) fn sanitized_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    login_path: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = vars
        .into_iter()
        .filter(|(k, _)| !is_token_var(k))
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
        // Dropped, as in Podium: child output never goes to the logs.
        .stderr(Stdio::null());
    hide_console_window(&mut cmd);
    // Spawned under the slot's lock: a shutdown that cancels and kills
    // either finds the child in the slot or sees the cancel flag below.
    let mut guard = slot.lock().expect("child lock poisoned");
    if cancel.load(Ordering::Relaxed) {
        return Err(AgentError::Cancelled);
    }
    let mut child = cmd.spawn().map_err(|_| AgentError::Spawn)?;

    // Written from its own thread: a prompt larger than the pipe buffer
    // would otherwise deadlock against an agent already writing stdout.
    if let (Some(prompt), Some(mut stdin)) = (inv.stdin.clone(), child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
    }
    let stdout = child.stdout.take().expect("stdout piped");
    *guard = Some(child);
    drop(guard);
    let kind = inv.output;
    let shared = Arc::new(Mutex::new(ReadSoFar::default()));
    let into = shared.clone();
    let reader = std::thread::spawn(move || read_answer(stdout, kind, &into));

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
                return Err(AgentError::Spawn);
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
            return Err(e);
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
    match (answer, status.success()) {
        (Answer::Result(text), _) => Ok(text),
        (Answer::Failed(kind), _) => Err(AgentError::Failed(kind)),
        (Answer::Text(text), true) if !text.trim().is_empty() => Ok(text),
        (_, false) => Err(AgentError::Exit(status.code())),
        _ => Err(AgentError::NoAnswer),
    }
}

enum Answer {
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
            OutputKind::ClaudeStreamJson => self.last.take().unwrap_or(Answer::None),
            OutputKind::Text => {
                Answer::Text(String::from_utf8_lossy(&std::mem::take(&mut self.text)).into_owned())
            }
        }
    }
}

fn read_answer(stdout: impl Read, kind: OutputKind, into: &Mutex<ReadSoFar>) {
    match kind {
        OutputKind::ClaudeStreamJson => {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(a) = parse_result_line(&line) {
                    into.lock().expect("answer lock poisoned").last = Some(a);
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
        assert!(
            joined.contains("--disallowedTools Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch")
        );
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
    fn auggie_reads_its_prompt_from_a_file_with_its_writing_tools_removed() {
        let args = AUGGIE.args(
            &dirs(Some(Path::new("/src/widgets")), None),
            Some(Path::new("/tmp/x/prompt.md")),
            &[],
        );
        assert_eq!(
            args[..8],
            [
                "--print",
                "--quiet",
                "--ask",
                "--dont-save-session",
                "--workspace-root",
                "/tmp/argus-ai-x",
                "--add-workspace",
                "/src/widgets",
            ]
        );
        let joined = args.join(" ");
        for tool in [
            "save-file",
            "launch-process",
            "web-fetch",
            "web-search",
            "sub-agent-code",
        ] {
            assert!(joined.contains(&format!("--remove-tool {tool}")), "{tool}");
        }
        assert!(joined.ends_with("--instruction-file /tmp/x/prompt.md"));
        assert_eq!(adapter(" auggie ").unwrap().id, "auggie");
        assert!(adapter("copilot").is_none());
        // MCP tools of the user's own config cannot be turned off.
        assert!(!read_only_guaranteed("auggie"));
        assert!(read_only_guaranteed("copilot"));
    }

    #[test]
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

        let settings = claude_settings(&[]);
        assert_eq!(settings["disableAllHooks"], true);
        assert!(settings["permissions"]["deny"]
            .as_array()
            .is_some_and(|d| !d.is_empty()));
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
