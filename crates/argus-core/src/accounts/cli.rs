//! Asking `gh` and `glab` which hosts they are logged in to.
//!
//! `gh auth status --json hosts` (gh 2.8x+; an older gh's text output is
//! parsed instead) and `glab auth status --all --output json` (an older
//! glab without `--all` lists every host anyway; one without `--output`
//! is read as text). Both check every login against its host, so each is
//! one request per host, killed after [`AUTH_TIMEOUT`]. Never with
//! `--show-token`: neither prints a token then, and only the fields named
//! here are read from what they print. Error texts are never forwarded as
//! they are: gh's are replaced by Argus' own words, glab's pass
//! [`redact_tokens`](crate::provider::gitlab::redact_tokens) first.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;

use super::{AccountState, CliReport, GhLogin, HostLogin};
use crate::platform::{run_with_timeout, ExecError};
use crate::post::sanitize;
use crate::provider::gitlab::redact_tokens;
use crate::provider::ProviderId;

/// How long one `auth status` may take: it checks every login with its
/// host, so a few requests on a slow link.
pub(crate) const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

/// `gh auth status` for every host.
pub(crate) fn ask_gh(program: &str, timeout: Duration) -> CliReport {
    let json = run(
        gh_command(program, &["auth", "status", "--json", "hosts"]),
        timeout,
    );
    let output = match json {
        Err(report) => return report,
        Ok(o) => o,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(hosts) = parse_gh_json(&stdout) {
        return CliReport::Hosts(hosts);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    if stderr.contains("unknown flag") {
        // A gh before `auth status --json`: its text output, which it
        // prints on stderr when a login has a problem.
        return match run(gh_command(program, &["auth", "status"]), timeout) {
            Err(report) => report,
            Ok(o) => {
                let text = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                );
                parse_gh_text(&text).map_or_else(
                    || CliReport::Failed("gh's answer could not be read".into()),
                    CliReport::Hosts,
                )
            }
        };
    }
    CliReport::Failed("gh's answer could not be read".into())
}

/// `glab auth status` for every host.
pub(crate) fn ask_glab(program: &str, timeout: Duration) -> CliReport {
    let mut args: Vec<&str> = vec!["auth", "status", "--all", "--output", "json"];
    for _ in 0..3 {
        let output = match run(glab_command(program, &args), timeout) {
            Err(report) => return report,
            Ok(o) => o,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(hosts) = parse_glab_json(&stdout) {
            return CliReport::Hosts(hosts);
        }
        let lower = stderr.to_lowercase();
        if lower.contains("unknown flag: --all") {
            // An older glab: `auth status` lists every host on its own.
            args.retain(|a| *a != "--all");
            continue;
        }
        if lower.contains("unknown flag: --output") || lower.contains("unknown shorthand") {
            args.retain(|a| *a != "--output" && *a != "json");
            continue;
        }
        let text = format!("{stdout}\n{stderr}");
        if let Some(hosts) = parse_glab_text(&text) {
            return CliReport::Hosts(hosts);
        }
        if no_glab_login(&text) {
            return CliReport::Hosts(Vec::new());
        }
        return CliReport::Failed("glab's answer could not be read".into());
    }
    CliReport::Failed("glab's answer could not be read".into())
}

/// The program's output, or the report for a program that could not run.
fn run(mut cmd: Command, timeout: Duration) -> Result<std::process::Output, CliReport> {
    match run_with_timeout(&mut cmd, None, timeout) {
        Ok(o) => Ok(o),
        Err(ExecError::Missing) => Err(CliReport::Missing),
        Err(ExecError::TimedOut) => Err(CliReport::Failed("it did not answer in time".into())),
        Err(ExecError::Failed) => Err(CliReport::Failed("it could not be started".into())),
    }
}

fn gh_command(program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        // Every host, not the one a variable picks.
        .env_remove("GH_HOST");
    hide_window(&mut cmd);
    cmd
}

fn glab_command(program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Not a repository: glab would read the git remotes of wherever
        // Argus runs.
        .current_dir(std::env::temp_dir())
        .env("NO_COLOR", "1")
        .env("GLAB_NO_PROMPT", "true")
        .env("GLAB_CHECK_UPDATE", "false")
        .env("GLAB_SEND_TELEMETRY", "false")
        // glab's HTTP debugging prints its `PRIVATE-TOKEN` header.
        .env_remove("GLAB_DEBUG_HTTP")
        .env_remove("DEBUG");
    hide_window(&mut cmd);
    cmd
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_window(_cmd: &mut Command) {}

/// One account in `gh auth status --json hosts`. Only these fields are
/// read (a `token` field, which gh fills only with `--show-token`, never).
#[derive(Deserialize)]
struct GhEntry {
    #[serde(default)]
    state: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct GhStatus {
    hosts: BTreeMap<String, Vec<GhEntry>>,
}

/// gh's JSON: `{"hosts": {"<host>": [{"state", "active", "login", …}]}}`;
/// `None` when it is not that.
pub(crate) fn parse_gh_json(stdout: &str) -> Option<Vec<HostLogin>> {
    let status: GhStatus = serde_json::from_str(stdout.trim()).ok()?;
    let hosts = status
        .hosts
        .into_iter()
        .filter_map(|(host, entries)| {
            let logins: Vec<GhLogin> = entries
                .into_iter()
                .filter(|e| !e.login.trim().is_empty())
                .map(|e| GhLogin {
                    state: gh_state(&e.state, &host, e.login.trim()),
                    login: e.login.trim().to_string(),
                    active: e.active,
                })
                .collect();
            gh_host(&host, logins)
        })
        .collect();
    Some(hosts)
}

/// The text output of a gh without `--json` (2.40+: `Logged in to <host>
/// account <login>`, `Active account: true`; older: `Logged in to <host> as
/// <login>`); `None` when nothing in it reads as gh's.
pub(crate) fn parse_gh_text(text: &str) -> Option<Vec<HostLogin>> {
    let mut hosts: Vec<(String, Vec<GhLogin>)> = Vec::new();
    let mut understood = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("not logged into any") {
            understood = true;
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            // A host heading: no spaces in a host.
            if !trimmed.contains(' ') && trimmed.contains('.') {
                hosts.push((trimmed.to_ascii_lowercase(), Vec::new()));
            }
            continue;
        }
        let Some((host, logins)) = hosts.last_mut() else {
            continue;
        };
        let body = unmarked(trimmed);
        let lower = body.to_lowercase();
        let state = if lower.starts_with("logged in to ") {
            Some("success")
        } else if lower.starts_with("failed to log in to ") {
            Some("error")
        } else if lower.starts_with("timeout trying to log in to ") {
            Some("timeout")
        } else {
            None
        };
        if let Some(state) = state {
            let login = word_after(body, " account ").or_else(|| word_after(body, " as "));
            if let Some(login) = login {
                understood = true;
                let state = gh_state(state, host, &login);
                // A gh without multiple accounts has one, the active one.
                let active = logins.is_empty();
                logins.push(GhLogin {
                    login,
                    active,
                    state,
                });
            }
            continue;
        }
        if let Some(rest) = lower.strip_prefix("active account: ") {
            if let Some(last) = logins.last_mut() {
                last.active = rest.trim() == "true";
            }
        }
    }
    if !understood {
        return None;
    }
    // With several accounts gh names the active one; if it named none,
    // the first stands.
    Some(
        hosts
            .into_iter()
            .filter_map(|(host, mut logins)| {
                if logins.iter().filter(|l| l.active).count() > 1 {
                    let first = logins.iter().position(|l| l.active).unwrap_or(0);
                    for (i, l) in logins.iter_mut().enumerate() {
                        l.active = i == first;
                    }
                }
                gh_host(&host, logins)
            })
            .collect(),
    )
}

/// A status line without its mark (`✓`, `X`, `x`, `-`, `!`).
fn unmarked(line: &str) -> &str {
    let body = line.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    body.strip_prefix("X ")
        .or_else(|| body.strip_prefix("x "))
        .unwrap_or(body)
        .trim_start()
}

/// The word after `marker` in `text` (`… account alice (keyring)` → alice).
fn word_after(text: &str, marker: &str) -> Option<String> {
    let at = text.find(marker)? + marker.len();
    let word = text[at..].split_whitespace().next()?;
    let word = word.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    (!word.is_empty()).then(|| word.to_string())
}

/// One gh host from its logins; `None` for a host name Argus would not use.
fn gh_host(host: &str, logins: Vec<GhLogin>) -> Option<HostLogin> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(['/', ' ', '@']) {
        return None;
    }
    let active = logins
        .iter()
        .find(|l| l.active)
        .or_else(|| logins.first())
        .cloned();
    Some(HostLogin {
        provider: ProviderId::Github,
        state: active
            .as_ref()
            .map_or(AccountState::NotLoggedIn { message: None }, |a| {
                a.state.clone()
            }),
        user: active.map(|a| a.login),
        logins,
        scheme: None,
        host,
    })
}

/// gh's state of one login, in Argus' words (gh's own error text is never
/// shown: it is gh's, and may name where the token is kept).
fn gh_state(state: &str, host: &str, login: &str) -> AccountState {
    match state {
        "success" => AccountState::LoggedIn,
        "timeout" => AccountState::Error {
            message: format!("gh could not reach {host} to check the login of {login}."),
        },
        "error" => AccountState::NotLoggedIn {
            message: Some(format!(
                "gh's login of {login} on {host} is no longer valid."
            )),
        },
        _ => AccountState::Error {
            message: format!("gh could not check the login of {login} on {host}."),
        },
    }
}

/// One host in `glab auth status --output json`. Only these fields are
/// read (never `token_source` or a token).
#[derive(Deserialize)]
struct GlabEntry {
    #[serde(default)]
    host: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    api_protocol: Option<String>,
}

#[derive(Deserialize)]
struct GlabStatus {
    hosts: Vec<GlabEntry>,
}

/// glab's JSON: `{"hosts": [{"host", "state", "user", "error",
/// "api_protocol", …}]}`; `None` when it is not that (glab's `{"error":
/// …}` included).
pub(crate) fn parse_glab_json(stdout: &str) -> Option<Vec<HostLogin>> {
    let status: GlabStatus = serde_json::from_str(stdout.trim()).ok()?;
    Some(
        status
            .hosts
            .into_iter()
            .filter_map(|e| {
                let state = if e.state == "success" {
                    AccountState::LoggedIn
                } else {
                    glab_state(&e.host, e.error.as_deref().unwrap_or_default())
                };
                glab_host(
                    &e.host,
                    state,
                    e.user.filter(|u| !u.trim().is_empty()),
                    e.api_protocol,
                )
            })
            .collect(),
    )
}

/// The text output of a glab without `--output json`: a host heading, then
/// `✓ Logged in to <host> as <user>` or `x <host>: API call failed: …`,
/// and `API calls for <host> are made over <protocol> protocol`.
pub(crate) fn parse_glab_text(text: &str) -> Option<Vec<HostLogin>> {
    struct Found {
        host: String,
        user: Option<String>,
        error: Option<String>,
        scheme: Option<String>,
        understood: bool,
    }
    let mut found: Vec<Found> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            if !trimmed.contains(' ') && trimmed.contains(['.', ':']) {
                found.push(Found {
                    host: trimmed.to_ascii_lowercase(),
                    user: None,
                    error: None,
                    scheme: None,
                    understood: false,
                });
            }
            continue;
        }
        let Some(f) = found.last_mut() else {
            continue;
        };
        let body = unmarked(trimmed);
        let lower = body.to_lowercase();
        if lower.starts_with("logged in to ") {
            f.user = word_after(body, " as ");
            f.understood = true;
        } else if lower.contains("api call failed") || lower.contains("has not been authenticated")
        {
            f.error = Some(body.to_string());
            f.understood = true;
        } else if lower.starts_with("api calls for ") {
            f.scheme = word_after(body, " over ");
        }
    }
    let hosts: Vec<HostLogin> = found
        .into_iter()
        .filter(|f| f.understood)
        .filter_map(|f| {
            let state = match (&f.user, &f.error) {
                (Some(_), None) => AccountState::LoggedIn,
                (_, error) => glab_state(&f.host, error.as_deref().unwrap_or_default()),
            };
            glab_host(&f.host, state, f.user, f.scheme)
        })
        .collect();
    (!hosts.is_empty()).then_some(hosts)
}

/// Whether glab says it has no login at all.
fn no_glab_login(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("no gitlab instances") || lower.contains("glab auth login")
}

/// One glab host; `None` for a host name Argus would not use.
fn glab_host(
    host: &str,
    state: AccountState,
    user: Option<String>,
    scheme: Option<String>,
) -> Option<HostLogin> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(['/', ' ', '@']) {
        return None;
    }
    let scheme = scheme
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s == "https" || s == "http");
    Some(HostLogin {
        provider: ProviderId::Gitlab,
        host,
        state,
        user,
        logins: Vec::new(),
        scheme,
    })
}

/// glab's error for a host, sorted: a login the instance refused (or none)
/// is not logged in; a host glab could not reach is an error that says so;
/// anything else carries glab's text, tokens redacted.
fn glab_state(host: &str, error: &str) -> AccountState {
    let lower = error.to_lowercase();
    if lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("no token")
        || lower.contains("has not been authenticated")
    {
        AccountState::NotLoggedIn {
            message: Some(format!("glab has no working login for {host}.")),
        }
    } else if lower.contains("dial tcp")
        || lower.contains("no such host")
        || lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("x509")
        || lower.contains("eof")
    {
        AccountState::Error {
            message: format!("glab could not reach {host}."),
        }
    } else {
        let detail = sanitize(&redact_tokens(error), &[]);
        AccountState::Error {
            message: if detail.is_empty() {
                format!("glab could not check the login for {host}.")
            } else {
                format!("glab could not check the login for {host}: {detail}")
            },
        }
    }
}
