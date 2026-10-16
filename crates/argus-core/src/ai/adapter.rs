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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Always passed, before the user's extra args.
    pub args: &'static [&'static str],
    pub prompt: PromptDelivery,
    pub output: OutputKind,
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
};

/// Auggie. `--print --quiet` prints only the final message; `--ask` is its
/// retrieval-and-non-editing-tools-only mode; `--dont-save-session` as
/// above. Its instruction comes from a file, so a long prompt never hits
/// the argv limit.
pub(crate) const AUGGIE: Adapter = Adapter {
    id: "auggie",
    binary: "auggie",
    args: &["--print", "--quiet", "--ask", "--dont-save-session"],
    prompt: PromptDelivery::File("--instruction-file"),
    output: OutputKind::Text,
};

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

    /// The argv after the binary.
    pub fn args(&self, prompt_file: Option<&Path>, extra: &[String]) -> Vec<String> {
        let mut out: Vec<String> = self.args.iter().map(|a| a.to_string()).collect();
        if let (PromptDelivery::File(flag), Some(file)) = (self.prompt, prompt_file) {
            out.push(flag.to_string());
            out.push(file.to_string_lossy().into_owned());
        }
        out.extend(extra.iter().cloned());
        out
    }
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

/// Run `inv` to completion (or until `cancel` is set or it times out, which
/// kills it) and return its answer text.
pub(crate) fn run(inv: &Invocation, cancel: &AtomicBool) -> Result<String, AgentError> {
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
    let mut child = cmd.spawn().map_err(|_| AgentError::Spawn)?;

    // Written from its own thread: a prompt larger than the pipe buffer
    // would otherwise deadlock against an agent already writing stdout.
    if let (Some(prompt), Some(mut stdin)) = (inv.stdin.clone(), child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
    }
    let stdout = child.stdout.take().expect("stdout piped");
    let kind = inv.output;
    let reader = std::thread::spawn(move || read_answer(stdout, kind));

    let deadline = Instant::now() + inv.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
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
            let _ = child.kill();
            let _ = child.wait();
            // The reader is not joined: a grandchild could hold the pipe.
            return Err(e);
        }
        std::thread::sleep(POLL);
    };
    let answer = reader.join().unwrap_or(Answer::None);
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

fn read_answer(stdout: impl Read, kind: OutputKind) -> Answer {
    match kind {
        OutputKind::ClaudeStreamJson => {
            let mut last = Answer::None;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(a) = parse_result_line(&line) {
                    last = a;
                }
            }
            last
        }
        OutputKind::Text => {
            let mut buf = Vec::new();
            let _ = stdout.take(MAX_TEXT_OUTPUT as u64).read_to_end(&mut buf);
            Answer::Text(String::from_utf8_lossy(&buf).into_owned())
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

    #[test]
    fn claude_args_are_print_stream_json_and_read_only() {
        let args = CLAUDE.args(None, &["--model".into(), "sonnet".into()]);
        let joined = args.join(" ");
        assert!(joined.starts_with("--print --output-format stream-json --verbose"));
        assert!(joined.contains("--tools Read,Grep,Glob"));
        assert!(
            joined.contains("--disallowedTools Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch")
        );
        assert!(joined.contains("--strict-mcp-config"));
        assert!(joined.ends_with("--model sonnet"));
        assert_eq!(CLAUDE.prompt, PromptDelivery::Stdin);
    }

    #[test]
    fn auggie_reads_its_prompt_from_a_file() {
        let args = AUGGIE.args(Some(Path::new("/tmp/x/prompt.md")), &[]);
        assert_eq!(
            args,
            [
                "--print",
                "--quiet",
                "--ask",
                "--dont-save-session",
                "--instruction-file",
                "/tmp/x/prompt.md"
            ]
        );
        assert_eq!(adapter(" auggie ").unwrap().id, "auggie");
        assert!(adapter("copilot").is_none());
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
