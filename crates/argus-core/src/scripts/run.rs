//! Running one submit script as a real process.
//!
//! The command is spawned directly (no shell, so nothing in a name or an
//! argument is ever interpreted), with its arguments, in the review's clone
//! (else a fresh temp dir, removed afterwards). Its environment is Argus'
//! own minus every variable that can hold a GitHub or GitLab token (as for
//! the AI pass' agents), with the login shell's `PATH` (a Finder-launched
//! app's `PATH` lacks Homebrew's `glab`, `gh` and `jq`) and the `ARGUS_*`
//! variables; stdin gets one JSON object, then closes. stdout and stderr
//! are kept (their tails), never logged. On Unix the script gets its own
//! process group, so a timeout kills what it started too.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use super::{ScriptJob, ScriptResult, ScriptStatus, MAX_OUTPUT};
use crate::ai::adapter::is_token_var;

const POLL: Duration = Duration::from_millis(20);

/// How long the output may still come in after the script exited: a
/// background process it left behind can hold the pipes open; what was
/// read by then is kept.
const READER_GRACE: Duration = Duration::from_secs(2);

/// Prefix of a script's temp dir (no clone set).
const TEMP_PREFIX: &str = "argus-script-";

/// The first line of output that lost its start to the cap.
const CUT_MARK: &str = "[… earlier output cut]\n";

/// Run `job` with Argus' environment (blocking).
pub(crate) fn execute(job: &ScriptJob, cancel: &AtomicBool) -> ScriptResult {
    execute_with(
        job,
        cancel,
        std::env::vars_os().collect(),
        crate::platform::login_path(),
        job.script.timeout(),
    )
}

/// [`execute`] over the environment `base` and the login shell's `PATH`
/// `login_path`, stopped after `timeout` (tests pass their own).
pub(crate) fn execute_with(
    job: &ScriptJob,
    cancel: &AtomicBool,
    base: Vec<(OsString, OsString)>,
    login_path: Option<&str>,
    timeout: Duration,
) -> ScriptResult {
    let script = &job.script;
    let name = script.display_name();
    let not_run =
        |status, message: String| ScriptResult::without_run(&script.id, &name, status, message);
    let env = script_env(base, login_path, &job.context.env());
    let path_var = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| v.clone());
    let home = env
        .iter()
        .find(|(k, _)| k == "HOME")
        .map(|(_, v)| PathBuf::from(v));
    let program = match resolve_command(&script.command, path_var.as_deref(), home.as_deref()) {
        Ok(p) => p,
        Err((status, message)) => return not_run(status, message),
    };

    // The clone when set and there, else a temp dir of its own.
    let temp;
    let cwd = match job.clone_dir.as_deref().filter(|d| d.is_dir()) {
        Some(dir) => dir.to_path_buf(),
        None => match TempDir::new() {
            Ok(t) => {
                temp = t;
                temp.0.clone()
            }
            Err(_) => {
                return not_run(
                    ScriptStatus::Failed,
                    "Argus could not make a working folder for it.".into(),
                )
            }
        },
    };

    let mut cmd = Command::new(&program);
    cmd.args(&script.args)
        .current_dir(&cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_process_group(&mut cmd);
    hide_console_window(&mut cmd);

    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let (status, message) = spawn_failure(&e, &program);
            return not_run(status, message);
        }
    };
    // Written from its own thread (a script that never reads stdin must
    // not hold up the run), then closed by the drop.
    if let Some(mut stdin) = child.stdin.take() {
        let json = job.context.stdin_json();
        std::thread::spawn(move || {
            let _ = stdin.write_all(json.as_bytes());
        });
    }
    let stdout = Arc::new(Mutex::new(Tail::default()));
    let stderr = Arc::new(Mutex::new(Tail::default()));
    let readers = [
        child.stdout.take().map(|s| read_into(s, stdout.clone())),
        child.stderr.take().map(|s| read_into(s, stderr.clone())),
    ];

    let deadline = started + timeout;
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(_) => {
                kill_tree(&mut child);
                break Err((ScriptStatus::Failed, "Argus lost track of it.".to_string()));
            }
        }
        if cancel.load(Ordering::Relaxed) {
            kill_tree(&mut child);
            break Err((
                ScriptStatus::Failed,
                "Stopped: Argus quit before it finished.".into(),
            ));
        }
        if Instant::now() >= deadline {
            kill_tree(&mut child);
            break Err((
                ScriptStatus::TimedOut,
                format!(
                    "Took longer than {} and was stopped. Raise its timeout in Settings → Scripts if it needs longer.",
                    seconds(timeout)
                ),
            ));
        }
        std::thread::sleep(POLL);
    };
    let grace = Instant::now() + READER_GRACE;
    for r in readers.into_iter().flatten() {
        while !r.is_finished() && Instant::now() < grace {
            std::thread::sleep(POLL);
        }
    }
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (status, exit_code, message) = match ended {
        Ok(exit) if exit.success() => (ScriptStatus::Ok, exit.code(), None),
        Ok(exit) => match exit.code() {
            Some(code) => (
                ScriptStatus::Failed,
                Some(code),
                Some(format!("Exited with status {code}.")),
            ),
            None => (
                ScriptStatus::Failed,
                None,
                Some("Stopped by a signal.".into()),
            ),
        },
        Err((status, message)) => (status, None, Some(message)),
    };
    let text = |t: &Arc<Mutex<Tail>>| t.lock().expect("output lock poisoned").text();
    ScriptResult {
        script_id: script.id.clone(),
        name,
        status,
        exit_code,
        stdout: text(&stdout),
        stderr: text(&stderr),
        duration_ms,
        message,
        finished_at: Utc::now(),
    }
}

/// `d` for a message: whole seconds, or a fraction below one.
fn seconds(d: Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{}s", d.as_secs())
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// The script's environment: `base` without the token variables (see
/// [`is_token_var`]) and without `ARGUS_*` ones (only Argus' own go in),
/// `PATH` from the login shell when known, then `argus`.
pub(crate) fn script_env(
    base: Vec<(OsString, OsString)>,
    login_path: Option<&str>,
    argus: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = base
        .into_iter()
        .filter(|(k, _)| !is_token_var(k))
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .filter(|(k, _)| !k.to_ascii_uppercase().starts_with("ARGUS_"))
        .filter(|(k, _)| login_path.is_none() || !k.eq_ignore_ascii_case("PATH"))
        .collect();
    if let Some(p) = login_path {
        out.push(("PATH".into(), p.into()));
    }
    out.extend(argus.iter().cloned());
    out
}

type NotRun = (ScriptStatus, String);

/// The program to spawn for `command`: an absolute path, `~/…` (under
/// `home`), or a bare name found on `path_var`. A relative path with a
/// folder in it is refused (relative to what?).
pub(crate) fn resolve_command(
    command: &str,
    path_var: Option<&str>,
    home: Option<&Path>,
) -> Result<PathBuf, NotRun> {
    let command = command.trim();
    if command.is_empty() {
        return Err((
            ScriptStatus::NotFound,
            "No command is set: add the script's path in Settings → Scripts.".into(),
        ));
    }
    let path = if let Some(rest) = command.strip_prefix("~/") {
        match home {
            Some(h) => h.join(rest),
            None => {
                return Err((
                    ScriptStatus::NotFound,
                    format!("{command} was not found (no home folder is known)."),
                ))
            }
        }
    } else if Path::new(command).is_absolute() {
        PathBuf::from(command)
    } else if command.contains('/') || command.contains('\\') {
        return Err((
            ScriptStatus::NotFound,
            format!("{command} is a relative path: use the script's full path."),
        ));
    } else {
        return find_on_path(command, path_var).ok_or_else(|| {
            (
                ScriptStatus::NotFound,
                format!("{command} was not found on your PATH: use its full path."),
            )
        });
    };
    check_runnable(&path)?;
    Ok(path)
}

/// The first `dir/name` on `path_var` that is a runnable file.
fn find_on_path(name: &str, path_var: Option<&str>) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .map(|dir| dir.join(name))
        .find(|p| check_runnable(p).is_ok())
}

/// `path` is a file the user may run: there, not a folder, executable.
fn check_runnable(path: &Path) -> Result<(), NotRun> {
    let shown = path.display();
    let meta = std::fs::metadata(path)
        .map_err(|_| (ScriptStatus::NotFound, format!("{shown} was not found.")))?;
    if meta.is_dir() {
        return Err((
            ScriptStatus::NotExecutable,
            format!("{shown} is a folder, not a script."),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err((ScriptStatus::NotExecutable, not_executable(path)));
        }
    }
    Ok(())
}

fn not_executable(path: &Path) -> String {
    let p = path.display();
    format!("{p} is not executable — run chmod +x {p}")
}

/// Why the spawn failed, in Argus' words.
fn spawn_failure(e: &std::io::Error, program: &Path) -> NotRun {
    use std::io::ErrorKind;
    // ENOEXEC: no `#!` line and not a binary.
    if e.raw_os_error() == Some(8) {
        return (
            ScriptStatus::NotExecutable,
            format!(
                "{} could not start: a script needs a first line such as #!/bin/sh",
                program.display()
            ),
        );
    }
    match e.kind() {
        ErrorKind::NotFound => (
            ScriptStatus::NotFound,
            format!(
                "{} could not start: it or the interpreter on its #! line was not found.",
                program.display()
            ),
        ),
        ErrorKind::PermissionDenied => (ScriptStatus::NotExecutable, not_executable(program)),
        _ => (
            ScriptStatus::Failed,
            format!("{} could not start.", program.display()),
        ),
    }
}

#[cfg(unix)]
fn own_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn own_process_group(_cmd: &mut Command) {}

#[cfg(windows)]
fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_console_window(_cmd: &mut Command) {}

/// Kill the script and (Unix) its process group: what it started too.
/// `kill(1)` rather than a syscall, as the crate forbids `unsafe`.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let _ = Command::new("/bin/kill")
            .args(["-s", "KILL", "--", &group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The last [`MAX_OUTPUT`] bytes of a stream.
#[derive(Default)]
pub(crate) struct Tail {
    buf: Vec<u8>,
    cut: bool,
}

impl Tail {
    pub(crate) fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        // Trimmed in steps, not on every chunk.
        if self.buf.len() > 2 * MAX_OUTPUT {
            let extra = self.buf.len() - MAX_OUTPUT;
            self.buf.drain(..extra);
            self.cut = true;
        }
    }

    pub(crate) fn text(&self) -> String {
        let mut bytes: &[u8] = &self.buf;
        let mut cut = self.cut;
        if bytes.len() > MAX_OUTPUT {
            bytes = &bytes[bytes.len() - MAX_OUTPUT..];
            cut = true;
        }
        if cut {
            // Not from the middle of a UTF-8 character.
            while bytes
                .first()
                .is_some_and(|b| b & 0b1100_0000 == 0b1000_0000)
            {
                bytes = &bytes[1..];
            }
        }
        let text = String::from_utf8_lossy(bytes);
        if cut {
            format!("{CUT_MARK}{text}")
        } else {
            text.into_owned()
        }
    }
}

fn read_into(
    mut stream: impl Read + Send + 'static,
    into: Arc<Mutex<Tail>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => into.lock().expect("output lock poisoned").push(&chunk[..n]),
            }
        }
    })
}

/// A fresh folder under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path.canonicalize().unwrap_or(path)))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
