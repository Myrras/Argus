//! Writing a fake CLI that a test then runs. Shared by the integration
//! tests (`#[path = "support/script.rs"] mod script;`) and, through
//! `src/testutil.rs`, the unit tests.
//!
//! A test thread that forks while another is still writing a script holds
//! its write fd until it execs, so running the script then fails with "text
//! file busy" (ETXTBSY), which the code under test reports as a missing or
//! broken CLI. The script is therefore run once right after writing, until
//! it starts; a line after the shebang makes that run exit at once.

#![allow(dead_code)]

use std::path::Path;

/// Set only for the warm-up run.
const WARMUP: &str = "ARGUS_TEST_WARMUP";

/// Write `contents` to `path` as an executable (Unix) and make sure it can
/// be run. A script (`#!` first line) is guarded for the warm-up run; other
/// contents are written as they are.
pub fn write_executable(path: &Path, contents: &str) {
    let guarded = contents
        .split_once('\n')
        .filter(|_| contents.starts_with("#!"));
    let text = match guarded {
        Some((shebang, rest)) => format!("{shebang}\n[ -z \"${WARMUP}\" ] || exit 0\n{rest}"),
        None => contents.to_string(),
    };
    std::fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        if guarded.is_some() {
            wait_until_runnable(path);
        }
    }
}

#[cfg(unix)]
fn wait_until_runnable(path: &Path) {
    use std::process::{Command, Stdio};
    for _ in 0..400 {
        let run = Command::new(path)
            .env(WARMUP, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match run {
            // ETXTBSY is 26 on Linux and macOS (its ErrorKind needs Rust 1.83).
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            _ => return,
        }
    }
    panic!("{} stayed busy for 2 s", path.display());
}
