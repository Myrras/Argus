//! Auggie (the Augment CLI) in the two shapes Argus knows, told apart by
//! what its `--help` lists, never by its version string:
//!
//! - **Legacy** (0.x, checked with 0.37): `--print --quiet --ask
//!   --dont-save-session`, the prompt in `--instruction-file`, the answer as
//!   plain text. No allow-list: `auggie tools list` names the tools and every
//!   one but the read set is removed (`--remove-tool`, one exact name each).
//! - **Current** (1.x, checked with 1.2.0-prerelease): `--print --mode
//!   json`, the prompt on stdin, the answer and its thinking/text deltas as
//!   JSON lines (see [`super::adapter::parse_auggie_line`]). `--tools`
//!   allow-lists the built-in tools (`read,grep,find,ls` for a review,
//!   `--no-tools` for the script writer), `--mcp-config` with no server
//!   replaces the user's MCP servers (theirs write: merge requests, SQL, …),
//!   and `--remove-tool` takes what extensions add (web search, sub-agents,
//!   …). `--no-session` keeps the run out of the user's sessions, and
//!   `--no-extensions`, `--no-skills`, `--no-rules` and
//!   `--no-prompt-templates` keep their own setup out of a one-shot run.
//!   It has no `tools list` any more: that line is a prompt to it, run with
//!   every tool, so it is never sent to this shape.
//!
//! The review worktree is not an `--add-workspace` for the current shape:
//! with Augment's multi-repo context on (it was, 2026-10), an added
//! workspace's `AGENTS.md`/`CLAUDE.md` and those of its parent directories
//! load as instructions, and the worktree's are the head's, which the
//! author controls (rules come from the target branch, see
//! [`super::context`]). Its read tools take absolute paths, and the prompt
//! names the worktree's.
//!
//! A binary whose `--help` fits neither shape does not run: the caller gets
//! [`AgentError::Unsupported`]. A flag Argus passes that the CLI does not
//! know fails the run (both shapes refuse unknown options), never runs it
//! with less protection.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use super::adapter::{self, AgentError, ChildSlot, Invocation, OutputKind, PromptDelivery};

/// Most `auggie --help` may take (a Node start).
pub(crate) const HELP_TIMEOUT: Duration = Duration::from_secs(30);

/// Most of a help text read from stderr (1.2.0's is 9 KB).
const HELP_CAP: usize = 256 * 1024;

/// Which auggie a binary is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuggieKind {
    Legacy,
    Current,
}

/// What one auggie binary can do: its shape and the flags its `--help`
/// lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuggieCli {
    pub kind: AuggieKind,
    flags: BTreeSet<String>,
}

/// Flags a current auggie must list: print mode, JSON output (checked on
/// `--mode`'s line), the built-in allow-list, MCP replaced, tool removal.
const CURRENT_REQUIRED: &[&str] = &[
    "--print",
    "--mode",
    "--tools",
    "--no-tools",
    "--mcp-config",
    "--remove-tool",
    "--workspace-root",
];

/// Flags a legacy auggie must list.
const LEGACY_REQUIRED: &[&str] = &[
    "--print",
    "--instruction-file",
    "--remove-tool",
    "--workspace-root",
];

/// A current auggie's run with no MCP server at all.
const NO_MCP: &str = r#"{"mcpServers":{}}"#;

/// The built-in tools a current auggie's review keeps (`--tools`): reading,
/// searching, listing. No shell, no edits.
pub(crate) const CURRENT_BUILTIN_READ: &str = "read,grep,find,ls";

/// Every tool a current auggie's review keeps: the built-in read tools,
/// the codebase search and the views of a long tool output it cut.
pub(crate) const CURRENT_READ_TOOLS: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "codebase-retrieval",
    "view-range-untruncated",
    "search-untruncated",
];

/// The tools of a current auggie that extensions or its runtime add on top
/// of `--tools` (1.2.0: the bundle's tool names), removed by name; an
/// unknown name is ignored. The built-in writing ones are listed too, in
/// case a user setting adds them back.
pub(crate) const CURRENT_REMOVED_TOOLS: &[&str] = &[
    "bash",
    "edit",
    "write",
    "multi-edit",
    "terminal",
    "web-search",
    "web-fetch",
    "agent",
    "get-subagent-result",
    "steer-subagent",
    "session-retrieval",
    "view-session",
    "ask-user",
    "find-tool",
    "execute-tool",
];

/// The tools a legacy run keeps: viewing and searching the workspace and
/// its history, rendering a diagram, viewing its task list.
pub(crate) const LEGACY_READ_TOOLS: &[&str] = &[
    "view",
    "view-range-untruncated",
    "grep-search",
    "search-untruncated",
    "codebase-retrieval",
    "git-commit-retrieval",
    "render-mermaid",
    "view_tasklist",
];

/// Every other built-in legacy tool (0.37: `auggie tools list`, and the
/// names in its bundle: the integrations, which only list once
/// configured, the alternate tool set, tool search). Removed on every run
/// whatever `auggie tools list` says; an unknown name is ignored.
pub(crate) const LEGACY_REMOVED_TOOLS: &[&str] = &[
    // Files.
    "remove-files",
    "save-file",
    "apply_patch",
    "apply-patch",
    "str-replace-editor",
    "edit",
    "write",
    // Processes and shells.
    "launch-process",
    "kill-process",
    "read-process",
    "write-process",
    "list-processes",
    "read-terminal",
    "shell",
    "bash",
    "setup-script",
    // The web and the integrations (`github-api` takes a method: it writes).
    "web-search",
    "web-fetch",
    "open-browser",
    "github-api",
    "linear",
    "jira",
    "confluence",
    "notion",
    "supabase",
    "glean",
    // Sub-agents (user-defined ones are `sub-agent-<name>`: listed).
    "sub-agent",
    "sub-agent-explore",
    "sub-agent-plan",
    "sub-agent-auggie-guide",
    "sub-agent-general-purpose",
    "sub-agent-research",
    "sub-agent-code",
    "sub-agent-validate",
    "report-to-parent",
    "finish-subagent-context-gathering",
    // Tool search can run a tool the run does not list.
    "find-tool",
    "execute-tool",
    // Task list edits, prompts, plan mode.
    "reorganize_tasklist",
    "update_tasks",
    "add_tasks",
    "ask-user",
    "enter-plan-mode",
    "exit-plan-mode",
    // Reads outside the read set: other sessions, raw retrieval, the
    // alternate tool set, diagnostics.
    "conversation-retrieval",
    "view-session",
    "codebase-retrieval-raw",
    "read",
    "grep",
    "find",
    "ls",
    "diagnostics",
];

/// The tool names in a legacy `auggie tools list` output (`  ✓ name
/// enabled`, or `✗ … disabled`), enabled or not: a disabled one is removed
/// again, in case the user's settings change before the run.
pub(crate) fn parse_tools_list(listing: &str) -> Vec<String> {
    strip_ansi(listing)
        .lines()
        .filter_map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                [.., name, "enabled" | "disabled"] if words.len() >= 3 => Some(name.to_string()),
                _ => None,
            }
        })
        .collect()
}

/// `s` without ANSI CSI sequences (`ESC [ … final byte`): help and listings
/// are coloured when auggie thinks it writes to a terminal.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The tools a legacy review removes: [`LEGACY_REMOVED_TOOLS`] and every
/// `listed` one, minus [`LEGACY_READ_TOOLS`]. Sorted, each once.
pub(crate) fn legacy_removed_tools(listed: &[String]) -> Vec<String> {
    let mut out: BTreeSet<String> = LEGACY_REMOVED_TOOLS.iter().map(|t| t.to_string()).collect();
    out.extend(listed.iter().filter(|t| !t.is_empty()).cloned());
    out.retain(|t| !LEGACY_READ_TOOLS.contains(&t.as_str()));
    out.into_iter().collect()
}

/// The tools a current review removes: every name Argus knows, of either
/// shape (a name the CLI does not have is ignored), minus
/// [`CURRENT_READ_TOOLS`]. Sorted, each once.
pub(crate) fn current_removed_tools() -> Vec<String> {
    let mut out: BTreeSet<String> = LEGACY_REMOVED_TOOLS
        .iter()
        .chain(CURRENT_REMOVED_TOOLS)
        .map(|t| t.to_string())
        .collect();
    out.retain(|t| !CURRENT_READ_TOOLS.contains(&t.as_str()));
    out.into_iter().collect()
}

/// Every tool name Argus knows plus `listed`: what the script writer, which
/// has no tools at all, removes. Sorted, each once.
pub(crate) fn all_tools(listed: &[String]) -> Vec<String> {
    let mut out: BTreeSet<String> = LEGACY_REMOVED_TOOLS
        .iter()
        .chain(LEGACY_READ_TOOLS)
        .chain(CURRENT_REMOVED_TOOLS)
        .chain(CURRENT_READ_TOOLS)
        .map(|t| t.to_string())
        .collect();
    out.extend(listed.iter().filter(|t| !t.is_empty()).cloned());
    out.into_iter().collect()
}

/// The long options a help text names (`--name`, up to a space, `=`, `[`,
/// `<` or `,`), ANSI colours stripped.
fn help_flags(help: &str) -> BTreeSet<String> {
    let text = strip_ansi(help);
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let starts = bytes[i] == b'-'
            && bytes[i + 1] == b'-'
            && bytes[i + 2].is_ascii_lowercase()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-'));
        if !starts {
            i += 1;
            continue;
        }
        let mut end = i + 2;
        while end < bytes.len()
            && (bytes[end].is_ascii_lowercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'-')
        {
            end += 1;
        }
        out.insert(text[i..end].trim_end_matches('-').to_string());
        i = end;
    }
    out
}

/// Which auggie `help` (its `--help` output) describes, or why it can run
/// neither way.
pub(crate) fn detect(help: &str) -> Result<AuggieCli, String> {
    let flags = help_flags(help);
    let missing = |required: &[&str]| -> Vec<String> {
        required
            .iter()
            .filter(|f| !flags.contains(**f))
            .map(|f| format!("`{f}`"))
            .collect()
    };
    // JSON output is a value of `--mode`, on its line.
    let json_mode = strip_ansi(help)
        .lines()
        .any(|l| l.contains("--mode") && l.contains("json"));
    let mut current = missing(CURRENT_REQUIRED);
    if !json_mode {
        current.push("`--mode json`".into());
    }
    if current.is_empty() {
        return Ok(AuggieCli {
            kind: AuggieKind::Current,
            flags,
        });
    }
    let legacy = missing(LEGACY_REQUIRED);
    if legacy.is_empty() {
        return Ok(AuggieCli {
            kind: AuggieKind::Legacy,
            flags,
        });
    }
    // The shape it is closer to says what is missing.
    let gaps = if flags.contains("--mode") || current.len() <= legacy.len() {
        current
    } else {
        legacy
    };
    Err(format!("its --help lists no {}", gaps.join(", ")))
}

impl AuggieCli {
    fn has(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }

    /// `flag` when the CLI lists it: the optional ones only.
    fn optional<'a>(&self, flags: &[&'a str]) -> impl Iterator<Item = String> + 'a {
        let present: Vec<&'a str> = flags.iter().copied().filter(|f| self.has(f)).collect();
        present.into_iter().map(str::to_string)
    }

    /// How the prompt goes in: a file for a legacy auggie (a long prompt
    /// never hits the argv limit), stdin for a current one.
    pub fn prompt(&self) -> PromptDelivery {
        match self.kind {
            AuggieKind::Legacy => PromptDelivery::File("--instruction-file"),
            AuggieKind::Current => PromptDelivery::Stdin,
        }
    }

    pub fn output(&self) -> OutputKind {
        match self.kind {
            AuggieKind::Legacy => OutputKind::Text,
            AuggieKind::Current => OutputKind::AuggieJson,
        }
    }

    /// Whether the tools must be listed first (`auggie tools list`): only a
    /// legacy auggie, which has no allow-list.
    pub fn lists_tools(&self) -> bool {
        self.kind == AuggieKind::Legacy
    }

    /// The flags every run starts with: print mode, the answer format, no
    /// saved session, the workspace root (the run's temp dir, so no
    /// `.augment/` of a checkout loads); for a current auggie also no MCP
    /// server and none of the user's extensions, skills, rules or prompt
    /// templates.
    fn base(&self, temp: &Path) -> Vec<String> {
        let mut out = vec!["--print".to_string()];
        match self.kind {
            AuggieKind::Legacy => {
                out.extend(self.optional(&["--quiet", "--ask", "--dont-save-session"]));
            }
            AuggieKind::Current => {
                out.extend(["--mode".into(), "json".into()]);
                out.extend(self.optional(&["--no-session"]));
            }
        }
        out.extend([
            "--workspace-root".into(),
            temp.to_string_lossy().into_owned(),
        ]);
        if self.kind == AuggieKind::Current {
            out.extend(["--mcp-config".into(), NO_MCP.into()]);
            out.extend(self.optional(&[
                "--no-extensions",
                "--no-skills",
                "--no-rules",
                "--no-prompt-templates",
            ]));
        }
        out
    }

    /// The argv of a review: [`Self::base`], the read tools only, the clone
    /// (legacy: an extra workspace; current: named in the prompt only, see
    /// the module docs), the prompt file (legacy), then the user's args.
    /// `listed` is what a legacy `tools list` named.
    pub fn review_args(
        &self,
        temp: &Path,
        clone: Option<&Path>,
        listed: &[String],
        prompt_file: Option<&Path>,
        extra: &[String],
    ) -> Vec<String> {
        let mut out = self.base(temp);
        let removed = match self.kind {
            AuggieKind::Legacy => {
                if let Some(clone) = clone {
                    out.extend([
                        "--add-workspace".into(),
                        clone.to_string_lossy().into_owned(),
                    ]);
                }
                legacy_removed_tools(listed)
            }
            AuggieKind::Current => {
                out.extend(["--tools".into(), CURRENT_BUILTIN_READ.into()]);
                current_removed_tools()
            }
        };
        // `=`-joined: a listed name that starts with `-` stays a value,
        // never a flag of its own.
        out.extend(removed.into_iter().map(|t| format!("--remove-tool={t}")));
        self.finish(out, prompt_file, extra)
    }

    /// The argv of a "Write with AI" turn: [`Self::base`] with no tools at
    /// all, `system` appended to a current auggie's system prompt, the
    /// prompt file (legacy), then the user's args.
    pub fn writer_args(
        &self,
        temp: &Path,
        listed: &[String],
        system: &str,
        prompt_file: Option<&Path>,
        extra: &[String],
    ) -> Vec<String> {
        let mut out = self.base(temp);
        if self.kind == AuggieKind::Current {
            out.push("--no-tools".into());
            if self.has("--append-system-prompt") {
                out.extend(["--append-system-prompt".into(), system.into()]);
            }
        }
        out.extend(
            all_tools(listed)
                .into_iter()
                .map(|t| format!("--remove-tool={t}")),
        );
        self.finish(out, prompt_file, extra)
    }

    fn finish(
        &self,
        mut out: Vec<String>,
        prompt_file: Option<&Path>,
        extra: &[String],
    ) -> Vec<String> {
        if let (PromptDelivery::File(flag), Some(file)) = (self.prompt(), prompt_file) {
            out.extend([flag.to_string(), file.to_string_lossy().into_owned()]);
        }
        out.extend(extra.iter().cloned());
        out
    }
}

/// A binary as it is on disk: a rebuilt or upgraded one (a new file behind
/// Homebrew's symlink) is asked again.
type Key = (PathBuf, u64, Option<SystemTime>);

fn cache() -> &'static Mutex<HashMap<Key, AuggieCli>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, AuggieCli>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn key(program: &Path) -> Option<Key> {
    let real = program.canonicalize().ok()?;
    let meta = std::fs::metadata(&real).ok()?;
    Some((real, meta.len(), meta.modified().ok()))
}

/// What the auggie at `program` is, from its `--help` (run in `cwd` with
/// `env`, at most `timeout`), cached per binary. Fails with
/// [`AgentError::Unsupported`] when it fits neither shape or `--help`
/// fails; only a fitting answer is cached.
pub(crate) fn probe(
    program: &Path,
    cwd: &Path,
    env: Vec<(OsString, OsString)>,
    timeout: Duration,
    cancel: &AtomicBool,
    slot: &ChildSlot,
) -> Result<AuggieCli, AgentError> {
    let key = key(program);
    if let Some(cli) = key.as_ref().and_then(|k| {
        cache()
            .lock()
            .expect("auggie cache poisoned")
            .get(k)
            .cloned()
    }) {
        return Ok(cli);
    }
    let inv = Invocation {
        program: program.to_path_buf(),
        args: vec!["--help".into()],
        cwd: cwd.to_path_buf(),
        env,
        stdin: None,
        timeout,
        output: OutputKind::Text,
    };
    // Auggie 1.2 prints its help to stderr when stdout is a pipe: a
    // successful run with nothing on stdout answers there.
    let help = match adapter::run_capped(&inv, cancel, slot, None, HELP_CAP) {
        Ok(help) => help,
        Err(f) if f.error == AgentError::NoAnswer => f.stderr,
        Err(f) if f.error == AgentError::Cancelled => return Err(AgentError::Cancelled),
        Err(f) => {
            return Err(AgentError::Unsupported(format!(
                "`auggie --help` failed ({})",
                f.error.message()
            )))
        }
    };
    let cli = detect(&help).map_err(AgentError::Unsupported)?;
    if let Some(k) = key {
        cache()
            .lock()
            .expect("auggie cache poisoned")
            .insert(k, cli.clone());
    }
    Ok(cli)
}

/// Recorded help texts for tests (the fake auggies print them).
#[cfg(test)]
pub(crate) mod fixtures {
    /// `auggie --help` of 1.2.0-prerelease, cut to the lines around the
    /// flags Argus reads (the real text, extension flags and all).
    pub(crate) const CURRENT_HELP: &str = "auggie - AI coding assistant with read, bash, edit, write tools

Usage:
  auggie [options] [@files...] [messages...]

Commands:
  auggie mcp <command>             Manage MCP servers (list/add/add-json/remove)
  auggie <command> --help          Show command-specific help

Options:
  --model <pattern>              Model pattern or ID (supports \"provider/id\" and optional \":<thinking>\")
  --append-system-prompt <text>  Append text or file contents to the system prompt
  --mode <mode>                  Output mode: text (default), stream, json, rpc, or acp
  --print, -p                    Non-interactive mode: process prompt and exit
  --workspace-root, -w <path>    Workspace root directory (auto-detects git root if absent)
  --add-workspace <path>         Additional workspace folder to index (repeatable)
  --discover-workspaces[=depth]  Force git-repo discovery under a non-git workspace root
  --no-session                   Don't save session (ephemeral)
  --no-tools                     Disable all built-in tools
  --tools <tools>                Comma-separated list of tools to enable (default: read,bash,edit,write)
                             Available: read, bash, edit, write, grep, find, ls
  --remove-tool <tool>           Remove a tool by name (repeatable)
  --no-extensions, -ne           Disable extension discovery (explicit -e paths still work)
  --mcp-config <cfg>             MCP server config as inline JSON or a JSON file path (repeatable); overrides settings.json MCP servers
  --permission <rule>            Tool-permission rule in 'tool-name:policy' format (repeatable)
  --no-skills, -ns               Disable skills discovery and loading
  --no-rules                     Disable rule discovery and loading
  --no-prompt-templates, -np     Disable prompt template discovery and loading
  --help, -h                     Show this help

Examples:
  auggie -p \"List all .ts files in src/\"
";

    /// A 0.x help in the shape Argus relied on (fictitious text, the flags
    /// the old adapter passed).
    pub(crate) const LEGACY_HELP: &str = "Usage: auggie [options] [command] [prompt]

Options:
  -p, --print                   Print the final answer and exit
  --quiet                       Only print the final message
  --ask                         Ask mode: retrieval and non-editing tools only
  --dont-save-session           Do not save the session
  --instruction-file <path>     Read the instruction from a file
  --workspace-root <path>       Workspace root
  --add-workspace <path>        Additional workspace
  --remove-tool <name>          Remove a tool
  --permission <rule>           Tool permission rule

Commands:
  tools list                    List the available tools
";
}

#[cfg(test)]
mod tests {
    use super::fixtures::{CURRENT_HELP, LEGACY_HELP};
    use super::*;

    #[test]
    fn the_help_tells_the_two_shapes_apart() {
        let current = detect(CURRENT_HELP).unwrap();
        assert_eq!(current.kind, AuggieKind::Current);
        assert!(current.has("--no-extensions") && current.has("--append-system-prompt"));
        // `[=depth]` and `, -w` end a flag; `-p` is no long option.
        assert!(current.has("--discover-workspaces") && current.has("--workspace-root"));
        assert!(!current.has("-p"));
        let legacy = detect(LEGACY_HELP).unwrap();
        assert_eq!(legacy.kind, AuggieKind::Legacy);
        // Coloured help reads the same.
        let coloured = CURRENT_HELP.replace("--mcp-config", "\u{1b}[36m--mcp-config\u{1b}[39m");
        assert_eq!(detect(&coloured).unwrap().kind, AuggieKind::Current);
    }

    #[test]
    fn a_help_that_fits_neither_shape_says_what_is_missing() {
        // A current auggie without the MCP override cannot keep the user's
        // MCP servers (some write) away.
        let no_mcp = CURRENT_HELP.replace("--mcp-config", "--mcp-file");
        assert_eq!(
            detect(&no_mcp).unwrap_err(),
            "its --help lists no `--mcp-config`"
        );
        let no_json = CURRENT_HELP.replace("stream, json, rpc", "stream, rpc");
        assert_eq!(
            detect(&no_json).unwrap_err(),
            "its --help lists no `--mode json`"
        );
        let old_without_removal = LEGACY_HELP.replace("--remove-tool", "--drop");
        assert_eq!(
            detect(&old_without_removal).unwrap_err(),
            "its --help lists no `--remove-tool`"
        );
        assert!(detect("")
            .unwrap_err()
            .starts_with("its --help lists no `--print`"));
    }

    #[test]
    fn current_review_args_allow_only_read_tools_and_no_mcp() {
        let cli = detect(CURRENT_HELP).unwrap();
        let args = cli.review_args(
            Path::new("/tmp/argus-ai-x"),
            Some(Path::new("/src/widgets/.argus/worktrees/w")),
            &[],
            None,
            &["--model".into(), "fixture-model".into()],
        );
        assert_eq!(
            args[..13],
            [
                "--print",
                "--mode",
                "json",
                "--no-session",
                "--workspace-root",
                "/tmp/argus-ai-x",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--no-extensions",
                "--no-skills",
                "--no-rules",
                "--no-prompt-templates",
                "--tools",
            ]
        );
        assert_eq!(args[13], "read,grep,find,ls");
        // The worktree is named in the prompt, not added: its AGENTS.md
        // would load.
        assert!(!args.iter().any(|a| a == "--add-workspace"), "{args:?}");
        for tool in [
            "bash",
            "write",
            "web-search",
            "agent",
            "terminal",
            "save-file",
        ] {
            assert!(args.contains(&format!("--remove-tool={tool}")), "{tool}");
        }
        for tool in CURRENT_READ_TOOLS {
            assert!(!args.contains(&format!("--remove-tool={tool}")), "{tool}");
        }
        assert!(!args.iter().any(|a| a == "--instruction-file"));
        assert!(args.ends_with(&["--model".into(), "fixture-model".into()]));
        assert_eq!(cli.prompt(), PromptDelivery::Stdin);
        assert_eq!(cli.output(), OutputKind::AuggieJson);
        assert!(!cli.lists_tools());
    }

    #[test]
    fn optional_flags_go_only_when_listed() {
        let bare = CURRENT_HELP
            .replace("--no-session", "--ephemeral")
            .replace("--no-extensions", "--x-ext")
            .replace("--append-system-prompt", "--system-extra");
        let cli = detect(&bare).unwrap();
        let args = cli.writer_args(Path::new("/tmp/t"), &[], "SYSTEM", None, &[]);
        for absent in ["--no-session", "--no-extensions", "--append-system-prompt"] {
            assert!(!args.iter().any(|a| a == absent), "{absent}: {args:?}");
        }
        assert!(args.iter().any(|a| a == "--no-skills"));
    }

    #[test]
    fn the_writer_has_no_tools_in_either_shape() {
        let cli = detect(CURRENT_HELP).unwrap();
        let args = cli.writer_args(Path::new("/tmp/t"), &[], "Answer in JSON.", None, &[]);
        assert!(args.iter().any(|a| a == "--no-tools"));
        assert!(!args.iter().any(|a| a == "--tools"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--append-system-prompt" && w[1] == "Answer in JSON."));
        for tool in CURRENT_READ_TOOLS.iter().chain(LEGACY_READ_TOOLS) {
            assert!(args.contains(&format!("--remove-tool={tool}")), "{tool}");
        }

        let legacy = detect(LEGACY_HELP).unwrap();
        let listed = vec!["add_todo_fixture".to_string()];
        let args = legacy.writer_args(
            Path::new("/tmp/t"),
            &listed,
            "ignored",
            Some(Path::new("/tmp/t/prompt.md")),
            &[],
        );
        assert_eq!(
            args[..6],
            [
                "--print",
                "--quiet",
                "--ask",
                "--dont-save-session",
                "--workspace-root",
                "/tmp/t"
            ]
        );
        assert!(args.contains(&"--remove-tool=add_todo_fixture".to_string()));
        assert!(args.contains(&"--remove-tool=view".to_string()));
        assert!(!args
            .iter()
            .any(|a| a == "--append-system-prompt" || a == "--mode"));
        assert!(args.ends_with(&["--instruction-file".into(), "/tmp/t/prompt.md".into()]));
    }

    #[test]
    fn legacy_review_args_are_as_before() {
        let cli = detect(LEGACY_HELP).unwrap();
        let listed = vec!["add_todo_fixture".to_string(), "-x".to_string()];
        let args = cli.review_args(
            Path::new("/tmp/argus-ai-x"),
            Some(Path::new("/src/widgets")),
            &listed,
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
        for tool in [
            "save-file",
            "launch-process",
            "web-fetch",
            "github-api",
            "sub-agent-code",
            // Listed for this user (an MCP tool), and one that looks like
            // a flag: still a value.
            "add_todo_fixture",
            "-x",
        ] {
            assert!(args.contains(&format!("--remove-tool={tool}")), "{tool}");
        }
        for tool in LEGACY_READ_TOOLS {
            assert!(!args.contains(&format!("--remove-tool={tool}")), "{tool}");
        }
        assert!(args
            .join(" ")
            .ends_with("--instruction-file /tmp/x/prompt.md"));
        assert_eq!(cli.prompt(), PromptDelivery::File("--instruction-file"));
        assert_eq!(cli.output(), OutputKind::Text);
        assert!(cli.lists_tools());
    }

    /// `auggie tools list` of auggie 0.37 (one user's MCP server's tools
    /// cut to two), in colour as on a terminal.
    const TOOLS_LIST: &str = "\nAvailable Tools:\n\n\u{1b}[36mCore Tools:\u{1b}[39m\n  \
\u{1b}[32m\u{2713}\u{1b}[39m codebase-retrieval               \u{1b}[32menabled\u{1b}[39m\n  \
\u{2713} remove-files                     enabled\n  \u{2713} save-file                        enabled\n  \
\u{2713} apply_patch                      enabled\n  \u{2713} str-replace-editor               enabled\n  \
\u{2713} view                             enabled\n\nProcess Tools:\n  \
\u{2713} launch-process                   enabled\n  \u{2713} kill-process                     enabled\n  \
\u{2713} read-process                     enabled\n  \u{2713} write-process                    enabled\n  \
\u{2713} list-processes                   enabled\n\nIntegration Tools:\n  \
\u{2713} add_todo_fixture                 enabled\n  \u{2713} spawn_agent_fixture              enabled\n  \
\u{2713} web-search                       enabled\n  \u{2713} web-fetch                        enabled\n  \
\u{2713} view-session                     enabled\n  \u{2713} codebase-retrieval-raw           enabled\n  \
\u{2713} sub-agent-explore                enabled\n  \u{2713} sub-agent-plan                   enabled\n  \
\u{2713} sub-agent-auggie-guide           enabled\n  \u{2713} sub-agent-general-purpose        enabled\n  \
\u{2713} sub-agent-research               enabled\n  \u{2713} sub-agent-code                   enabled\n  \
\u{2713} sub-agent-validate               enabled\n  \u{2713} sub-agent-fixture-reviewer       enabled\n  \
\u{2713} ask-user                         enabled\n  \u{2713} exit-plan-mode                   enabled\n  \
\u{2713} enter-plan-mode                  enabled\n\nTask Management:\n  \
\u{2713} view_tasklist                    enabled\n  \u{2713} reorganize_tasklist              enabled\n  \
\u{2713} update_tasks                     enabled\n  \u{2713} add_tasks                        enabled\n\n\
Advanced Tools:\n  \u{2713} grep-search                      enabled\n  \
\u{2713} view-range-untruncated           enabled\n  \u{2717} search-untruncated               disabled\n\n\
Total: 35 tools (34 enabled, 1 disabled)\n";

    #[test]
    fn a_legacy_run_keeps_only_its_read_tools() {
        let listed = parse_tools_list(TOOLS_LIST);
        assert_eq!(listed.len(), 35, "{listed:?}");
        assert!(listed.iter().any(|t| t == "codebase-retrieval"));
        assert!(listed.iter().any(|t| t == "search-untruncated"));
        let removed = legacy_removed_tools(&listed);
        // Every listed tool outside the read set goes, and with no list at
        // all every built-in one still does.
        for tool in &listed {
            let read = LEGACY_READ_TOOLS.contains(&tool.as_str());
            assert_eq!(removed.contains(tool), !read, "{tool}");
        }
        let fallback = legacy_removed_tools(&[]);
        // (The user's own tools, `*fixture*`, only the listing can name.)
        for tool in listed.iter().filter(|t| !t.contains("fixture")) {
            let read = LEGACY_READ_TOOLS.contains(&tool.as_str());
            assert!(read || fallback.contains(tool), "{tool}");
        }
        for tool in [
            "github-api",
            "open-browser",
            "linear",
            "jira",
            "confluence",
            "notion",
            "supabase",
            "glean",
            "shell",
            "bash",
            "execute-tool",
            "read-terminal",
        ] {
            assert!(fallback.iter().any(|t| t == tool), "{tool}");
        }
        for tool in LEGACY_READ_TOOLS {
            assert!(!LEGACY_REMOVED_TOOLS.contains(tool), "{tool}");
            assert!(!removed.iter().any(|t| t == tool), "{tool}");
        }
        assert!(
            parse_tools_list("no tools here\nTotal: 0 tools (0 enabled, 0 disabled)").is_empty()
        );
        // The current read set is never removed from a current review,
        // though the legacy list removes `read`, `grep`, `find` and `ls`
        // (legacy's alternate tool set).
        let current = current_removed_tools();
        for tool in CURRENT_READ_TOOLS {
            assert!(!current.iter().any(|t| t == tool), "{tool}");
        }
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn the_help_is_read_once_per_binary_and_again_after_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("auggie");
        let script = |help: &str| {
            format!("#!/bin/sh\necho x >> \"$(dirname \"$0\")/calls\"\ncat <<'EOF'\n{help}EOF\n")
        };
        crate::testutil::write_executable(&bin, &script(CURRENT_HELP));
        let ask = || {
            probe(
                &bin,
                dir.path(),
                adapter::sanitized_env(std::env::vars_os(), None),
                Duration::from_secs(20),
                &AtomicBool::new(false),
                &ChildSlot::default(),
            )
        };
        let calls = || {
            std::fs::read_to_string(dir.path().join("calls"))
                .unwrap_or_default()
                .lines()
                .count()
        };
        assert_eq!(ask().unwrap().kind, AuggieKind::Current);
        assert_eq!(ask().unwrap().kind, AuggieKind::Current);
        assert_eq!(calls(), 1);
        // An upgrade (a new file, a new size) is read again.
        crate::testutil::write_executable(&bin, &script(LEGACY_HELP));
        assert_eq!(ask().unwrap().kind, AuggieKind::Legacy);
        assert_eq!(calls(), 2);
        // A binary that fits neither is refused, and asked again next time.
        crate::testutil::write_executable(&bin, &script("Usage: auggie [prompt]\n  --print\n"));
        let e = ask().unwrap_err();
        assert!(
            e.message()
                .starts_with("this auggie version can't run read-only: its --help lists no "),
            "{}",
            e.message()
        );
        assert!(ask().is_err());
        assert_eq!(calls(), 4);
        // A help on stderr (auggie 1.2 when stdout is a pipe) reads the
        // same.
        crate::testutil::write_executable(
            &bin,
            &format!("#!/bin/sh\necho x >> \"$(dirname \"$0\")/calls\"\ncat >&2 <<'EOF'\n{CURRENT_HELP}EOF\n"),
        );
        assert_eq!(ask().unwrap().kind, AuggieKind::Current);
        // A failing --help is refused too.
        crate::testutil::write_executable(&bin, "#!/bin/sh\nexit 3\n");
        assert_eq!(
            ask().unwrap_err().message(),
            "this auggie version can't run read-only: `auggie --help` failed (the agent exited with status 3)"
        );
    }
}
