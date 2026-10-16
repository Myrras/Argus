//! Finding the `gh` binary. Ported from Podium's `platform.rs` (`gh_binary`,
//! `login_shell`, `command_exists_query`), minus its WSL backend.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

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

/// Run `cmd` through the login shell; its stdout when it exits successfully.
/// stderr is discarded, so nothing the shell prints can leak into logs.
fn shell_stdout(cmd: &str) -> Option<String> {
    let (shell, prefix) = login_shell();
    let mut command = Command::new(shell);
    command
        .args(prefix)
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console_window(&mut command);
    let output = command.spawn().ok()?.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
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
    #[cfg_attr(windows, ignore)]
    fn query_uses_command_v_on_unix() {
        assert_eq!(command_exists_query("gh"), "command -v gh");
    }
}
