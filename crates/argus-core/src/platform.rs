//! Finding CLI binaries (`gh`, the AI agents). Ported from Podium's
//! `platform.rs` (`gh_binary`, `login_shell`, `command_exists_query`), minus
//! its WSL backend.

use std::io::{Read, Write as _};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Where `gh` usually lives when the login shell cannot tell us (Homebrew on
/// Apple Silicon and Intel, MacPorts, a plain Linux install).
#[cfg(unix)]
const WELL_KNOWN_GH: &[&str] = &[
    "/opt/homebrew/bin/gh",
    "/usr/local/bin/gh",
    "/opt/local/bin/gh",
    "/usr/bin/gh",
];

/// The `gh` binary, resolved once; `None` when it is not installed (so
/// callers can report that instead of a spawn failure).
///
/// A macOS app launched from Finder/Dock inherits a minimal `PATH` without
/// Homebrew, so a bare `gh` is not found. Like Podium, ask the user's login
/// shell (`command -v gh`) so profile PATH edits are honoured, and use the
/// absolute path it prints. If the shell cannot answer (broken profile, no
/// `$SHELL`), fall back to the well-known install locations.
/// Windows PATH is process-wide, so `where` is enough there.
pub fn gh_binary() -> Option<&'static str> {
    static GH_PATH: OnceLock<Option<String>> = OnceLock::new();
    GH_PATH
        .get_or_init(|| {
            #[cfg(unix)]
            {
                shell_stdout(&command_exists_query("gh"))
                    .and_then(|out| parse_binary_path(&out))
                    .or_else(|| first_existing(WELL_KNOWN_GH.iter().map(Path::new)))
            }
            #[cfg(windows)]
            {
                shell_ok(&command_exists_query("gh")).then(|| "gh".to_string())
            }
        })
        .as_deref()
}

/// A shell command line that exits 0 iff `binary` resolves on `PATH`, and
/// (Unix) prints where. Native Windows uses `where`.
fn command_exists_query(binary: &str) -> String {
    if cfg!(windows) {
        format!("where {binary}")
    } else {
        format!("command -v {binary}")
    }
}

/// The login shell plus the args that precede a command string.
///
/// Unix: `$SHELL -lic <cmd>` — login *and* interactive, so both
/// `.zprofile`/`.bash_profile` and `.zshrc`/`.bashrc` PATH edits (Homebrew
/// shellenv, nvm, …) apply; some installers only touch the interactive file.
/// Windows: `%COMSPEC% /C <cmd>`.
fn login_shell() -> (String, Vec<String>) {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "/bin/zsh".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        (shell, vec!["-lic".to_string()])
    }
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        (shell, vec!["/C".to_string()])
    }
}

/// How long the login shell may take to answer. A profile that waits on
/// the network or a prompt would otherwise hang whoever asks (building the
/// providers, so the core itself); past this the well-known paths answer.
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// Run `cmd` through the login shell; its stdout when it exits successfully
/// within [`SHELL_TIMEOUT`]. stderr is discarded, so nothing the shell
/// prints can leak into logs.
fn shell_stdout(cmd: &str) -> Option<String> {
    let (shell, prefix) = login_shell();
    run_shell(&shell, &prefix, cmd, SHELL_TIMEOUT)
}

/// `shell <prefix…> cmd`, killed after `timeout`; its stdout on success.
fn run_shell(shell: &str, prefix: &[String], cmd: &str, timeout: Duration) -> Option<String> {
    let mut command = Command::new(shell);
    command
        .args(prefix)
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console_window(&mut command);
    let output = run_with_timeout(&mut command, None, timeout).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Why a child process gave no output.
#[derive(Debug)]
pub(crate) enum ExecError {
    /// The program is not there.
    Missing,
    /// It could not be started (or waited on).
    Failed,
    /// Killed after the timeout.
    TimedOut,
}

/// Run `cmd` to completion with `input` on stdin, or kill it once `timeout`
/// has passed. Both pipes are drained on their own threads so a large output
/// never stalls the child; on a timeout those threads are left to end when
/// the pipes close.
pub(crate) fn run_with_timeout(
    cmd: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Output, ExecError> {
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ExecError::Missing,
        _ => ExecError::Failed,
    })?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        // A program like gh reads all of stdin before it sends anything. A
        // write error means it exited early; its status says why.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::debug!("a child process timed out after {timeout:?}");
                return Err(ExecError::TimedOut);
            }
            Ok(None) => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(25));
            }
            Err(_) => return Err(ExecError::Failed),
        }
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

#[cfg(windows)]
fn shell_ok(cmd: &str) -> bool {
    shell_stdout(cmd).is_some()
}

#[cfg(windows)]
fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_console_window(_cmd: &mut Command) {}

/// The binary path in a login shell's `command -v` output. Takes the last
/// line (a chatty shell profile may print first) and only accepts an
/// absolute path, which rejects `alias gh=…` and function names.
#[cfg(any(unix, test))]
fn parse_binary_path(stdout: &str) -> Option<String> {
    let line = stdout.lines().last()?.trim();
    Path::new(line).is_absolute().then(|| line.to_string())
}

/// The first of `candidates` that is an existing file.
#[cfg(any(unix, test))]
fn first_existing<'a>(candidates: impl IntoIterator<Item = &'a Path>) -> Option<String> {
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Where agent CLIs usually live when the login shell cannot tell us.
/// `~` is the user's home.
#[cfg(unix)]
pub(crate) const WELL_KNOWN_CLAUDE: &[&str] = &[
    "~/.local/bin/claude",
    "~/.claude/local/claude",
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
];
#[cfg(unix)]
pub(crate) const WELL_KNOWN_AUGGIE: &[&str] =
    &["/opt/homebrew/bin/auggie", "/usr/local/bin/auggie"];

/// Resolve `binary` like [`gh_binary`] does (login shell `command -v`, then
/// `well_known`), for the AI agents. Not cached: the user may install the
/// CLI while Argus runs, and a run spawns rarely.
#[cfg(unix)]
pub(crate) fn resolve_binary(binary: &str, well_known: &[&str]) -> Option<String> {
    shell_stdout(&command_exists_query(binary))
        .and_then(|out| parse_binary_path(&out))
        .or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            let expanded: Vec<std::path::PathBuf> = well_known
                .iter()
                .map(|p| match p.strip_prefix("~/") {
                    Some(rest) if !home.is_empty() => Path::new(&home).join(rest),
                    _ => Path::new(p).to_path_buf(),
                })
                .collect();
            first_existing(expanded.iter().map(|p| p.as_path()))
        })
}

/// Windows: the first path `where` prints.
#[cfg(windows)]
pub(crate) fn resolve_binary(binary: &str, _well_known: &[&str]) -> Option<String> {
    shell_stdout(&command_exists_query(binary)).and_then(|out| {
        out.lines()
            .map(str::trim)
            .find(|l| Path::new(l).is_absolute())
            .map(str::to_string)
    })
}

/// The login shell's `PATH`, so an agent CLI started from a Finder-launched
/// Argus still finds what its shebang needs (`node` under nvm/Homebrew).
/// `None` on Windows (its PATH is process-wide) or when the shell fails.
/// Read once, like [`gh_binary`].
pub(crate) fn login_path() -> Option<&'static str> {
    static LOGIN_PATH: OnceLock<Option<String>> = OnceLock::new();
    LOGIN_PATH
        .get_or_init(|| {
            #[cfg(unix)]
            {
                shell_stdout("printf '%s\\n' \"$PATH\"")
                    .and_then(|out| out.lines().last().map(|l| l.trim().to_string()))
                    .filter(|p| !p.is_empty())
            }
            #[cfg(windows)]
            {
                None
            }
        })
        .as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(windows, ignore)]
    fn parse_takes_last_line_after_profile_noise() {
        let out = "Welcome back!\n/opt/homebrew/bin/gh\n";
        assert_eq!(
            parse_binary_path(out).as_deref(),
            Some("/opt/homebrew/bin/gh")
        );
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn parse_rejects_aliases_and_empty_output() {
        assert_eq!(parse_binary_path("alias gh='echo no'"), None);
        assert_eq!(parse_binary_path("gh"), None);
        assert_eq!(parse_binary_path(""), None);
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn first_existing_picks_first_real_file_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing/gh");
        let first = dir.path().join("a-gh");
        let second = dir.path().join("b-gh");
        std::fs::write(&first, "").unwrap();
        std::fs::write(&second, "").unwrap();
        let found = first_existing([missing.as_path(), first.as_path(), second.as_path()]);
        assert_eq!(found, Some(first.to_string_lossy().into_owned()));
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn first_existing_is_none_without_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gh");
        assert_eq!(first_existing([missing.as_path()]), None);
        // A directory is not a binary.
        assert_eq!(first_existing([dir.path()]), None);
    }

    #[test]
    fn times_out_a_hung_process() {
        #[cfg(unix)]
        {
            let start = Instant::now();
            let mut cmd = Command::new("sleep");
            cmd.arg("5").stdout(Stdio::piped()).stderr(Stdio::piped());
            let r = run_with_timeout(&mut cmd, None, Duration::from_millis(100));
            assert!(matches!(r, Err(ExecError::TimedOut)));
            assert!(start.elapsed() < Duration::from_secs(3));
        }
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn a_hung_login_shell_is_given_up_on() {
        let sh = ["-c".to_string()];
        let start = Instant::now();
        assert_eq!(
            run_shell(
                "/bin/sh",
                &sh,
                "sleep 5; echo /late/gh",
                Duration::from_millis(200)
            ),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(
            run_shell("/bin/sh", &sh, "echo /opt/homebrew/bin/gh", SHELL_TIMEOUT).as_deref(),
            Some("/opt/homebrew/bin/gh\n")
        );
        assert_eq!(run_shell("/bin/sh", &sh, "exit 1", SHELL_TIMEOUT), None);
    }

    #[test]
    #[cfg_attr(windows, ignore)]
    fn query_uses_command_v_on_unix() {
        assert_eq!(command_exists_query("gh"), "command -v gh");
    }
}
