//! The user's accounts: what `gh` and `glab` are logged in to, and which of
//! those hosts Argus polls (Settings → Accounts).
//!
//! Argus has no host settings of its own any more: every host the CLIs know
//! is one [`Account`], on unless the user switched it off
//! ([`Settings::accounts`]). One provider is built per enabled host that
//! has a login ([`provider_specs`]); reviews of a host switched off stay
//! stored, unpolled and out of the list.
//!
//! Asking the CLIs costs a request per host, so the answer is kept
//! ([`AccountBook`]) and asked again only when it matters: the first time
//! (startup), on Check again (`recheck`), after a settings change of the
//! accounts, and after a provider call refused for the login or one that
//! found no CLI. A refresh whose calls to a host went through marks that
//! host logged in without asking.

mod cli;
#[cfg(test)]
mod tests;

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use crate::provider::ProviderId;
use crate::settings::{account_key, parse_account_key, Settings};

/// One host of the user's, as Settings → Accounts shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// `provider:host`, the key of [`Settings::accounts`]; `provider:` for
    /// a missing CLI.
    pub key: String,
    pub provider: ProviderId,
    /// The host (`github.com`, a GHE or GitLab host, a port included);
    /// empty for a missing CLI.
    pub host: String,
    /// The login Argus uses on the host: the chosen gh account, else gh's
    /// active one; glab's user.
    pub user: Option<String>,
    pub state: AccountState,
    /// gh's active account on the host.
    pub active: Option<String>,
    /// gh: every account logged in on the host (for the picker); empty for
    /// GitLab.
    pub accounts: Vec<String>,
    /// gh: the account chosen in settings; `None` follows gh's active one.
    pub account: Option<String>,
    /// Argus polls the host (on unless switched off).
    pub enabled: bool,
    /// glab's API protocol for the host (`https`, `http`), when it said.
    pub scheme: Option<String>,
    /// What logs the CLI in to the host (`gh auth login --hostname <host>`).
    pub login_command: String,
}

/// Whether a host can be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountState {
    /// The CLI has a login the host accepted.
    LoggedIn,
    /// No login, or one the host refused; `message` says which when known.
    NotLoggedIn { message: Option<String> },
    /// The login could not be checked (the host was unreachable); Argus
    /// keeps polling it. Argus-owned text.
    Error { message: String },
    /// The CLI is not installed (or not where Argus looked).
    CliMissing { hint: String },
}

impl AccountState {
    /// Whether a provider is built for the host: a login that works, or
    /// one that could not be checked (offline, VPN down) and may well.
    fn usable(&self) -> bool {
        matches!(self, AccountState::LoggedIn | AccountState::Error { .. })
    }
}

/// One account of gh's on a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GhLogin {
    pub login: String,
    pub active: bool,
    pub state: AccountState,
}

/// What a CLI reported for one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostLogin {
    pub provider: ProviderId,
    pub host: String,
    /// The active account's state (gh), the host's (glab).
    pub state: AccountState,
    /// gh's active account; glab's user.
    pub user: Option<String>,
    /// gh: every account on the host.
    pub logins: Vec<GhLogin>,
    /// glab's API protocol.
    pub scheme: Option<String>,
}

/// What one CLI answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliReport {
    Hosts(Vec<HostLogin>),
    /// Not installed.
    Missing,
    /// It ran but gave no answer Argus could read (Argus-owned text).
    Failed(String),
}

/// What both CLIs answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Discovered {
    pub github: CliReport,
    pub gitlab: CliReport,
}

impl Discovered {
    pub(crate) fn empty() -> Self {
        Self {
            github: CliReport::Hosts(Vec::new()),
            gitlab: CliReport::Hosts(Vec::new()),
        }
    }

    fn report(&self, provider: ProviderId) -> &CliReport {
        match provider {
            ProviderId::Github => &self.github,
            ProviderId::Gitlab => &self.gitlab,
        }
    }

    fn report_mut(&mut self, provider: ProviderId) -> &mut CliReport {
        match provider {
            ProviderId::Github => &mut self.github,
            ProviderId::Gitlab => &mut self.gitlab,
        }
    }
}

/// What logs `provider`'s CLI in to `host`.
pub fn login_command(provider: ProviderId, host: &str) -> String {
    match (provider, host.is_empty()) {
        (ProviderId::Github, false) => format!("gh auth login --hostname {host}"),
        (ProviderId::Github, true) => "gh auth login".into(),
        (ProviderId::Gitlab, false) => format!("glab auth login --hostname {host}"),
        (ProviderId::Gitlab, true) => "glab auth login".into(),
    }
}

/// How to get a missing CLI.
fn install_hint(provider: ProviderId) -> String {
    match provider {
        ProviderId::Github => {
            "Install the GitHub CLI (https://cli.github.com, `brew install gh`), \
             then run `gh auth login`."
        }
        ProviderId::Gitlab => {
            "Install the GitLab CLI (https://gitlab.com/gitlab-org/cli, `brew install glab`), \
             then run `glab auth login`."
        }
    }
    .into()
}

/// The accounts Settings → Accounts lists: every host the CLIs reported,
/// then every host the settings name that they did not (not logged in);
/// a missing CLI is one row for its provider. GitHub first, then by host.
pub(crate) fn list(settings: &Settings, discovered: &Discovered) -> Vec<Account> {
    let mut out = Vec::new();
    for provider in [ProviderId::Github, ProviderId::Gitlab] {
        let mut rows: Vec<Account> = Vec::new();
        match discovered.report(provider) {
            CliReport::Missing => {
                out.push(Account {
                    key: format!("{}:", provider.as_str()),
                    provider,
                    host: String::new(),
                    user: None,
                    state: AccountState::CliMissing {
                        hint: install_hint(provider),
                    },
                    active: None,
                    accounts: Vec::new(),
                    account: None,
                    // Warn about a missing CLI only for a user who set the
                    // provider up before (an enabled entry).
                    enabled: settings.accounts.iter().any(|(k, c)| {
                        c.enabled && parse_account_key(k).is_some_and(|(p, _)| p == provider)
                    }),
                    scheme: None,
                    login_command: login_command(provider, ""),
                });
                continue;
            }
            CliReport::Failed(why) => {
                let cli = match provider {
                    ProviderId::Github => "gh",
                    ProviderId::Gitlab => "glab",
                };
                rows.push(Account {
                    key: format!("{}:", provider.as_str()),
                    provider,
                    host: String::new(),
                    user: None,
                    state: AccountState::Error {
                        message: format!("Could not ask {cli} for its logins: {why}."),
                    },
                    active: None,
                    accounts: Vec::new(),
                    account: None,
                    enabled: true,
                    scheme: None,
                    login_command: login_command(provider, ""),
                });
            }
            CliReport::Hosts(hosts) => {
                rows.extend(hosts.iter().map(|h| account(settings, h)));
            }
        }
        // Hosts only the settings name: switched off before, or migrated
        // from an older Argus, and not logged in now.
        for (key, choice) in &settings.accounts {
            let Some((p, host)) = parse_account_key(key) else {
                continue;
            };
            if p != provider || rows.iter().any(|r| r.host == host) {
                continue;
            }
            rows.push(Account {
                key: key.clone(),
                provider,
                login_command: login_command(provider, &host),
                host,
                user: choice.account.clone(),
                state: AccountState::NotLoggedIn { message: None },
                active: None,
                accounts: Vec::new(),
                account: choice.account.clone(),
                enabled: choice.enabled,
                scheme: None,
            });
        }
        rows.sort_by(|a, b| a.host.cmp(&b.host));
        out.extend(rows);
    }
    out
}

/// One reported host with the user's choice applied.
fn account(settings: &Settings, h: &HostLogin) -> Account {
    let choice = settings.account_choice(h.provider, &h.host);
    // No entry yet: on only while the host is logged in (a host the user
    // never uses, like gitlab.com in glab's list, stays quiet). An entry
    // (the user's switch, or the migration's) decides from then on.
    let enabled = if settings
        .accounts
        .contains_key(&account_key(h.provider, &h.host.to_ascii_lowercase()))
    {
        choice.enabled
    } else {
        h.state == AccountState::LoggedIn
    };
    let active = h.logins.iter().find(|l| l.active).map(|l| l.login.clone());
    let (user, state) = match (h.provider, &choice.account) {
        // A host reported without its accounts (a core over explicit
        // providers) takes the choice as it is.
        (ProviderId::Github, Some(chosen)) if h.logins.is_empty() => {
            (Some(chosen.clone()), h.state.clone())
        }
        (ProviderId::Github, Some(chosen)) => match h.logins.iter().find(|l| &l.login == chosen) {
            Some(l) => (Some(l.login.clone()), l.state.clone()),
            None => (
                Some(chosen.clone()),
                AccountState::NotLoggedIn {
                    message: Some(format!("gh has no login for {chosen} on {}.", h.host)),
                },
            ),
        },
        _ => (h.user.clone(), h.state.clone()),
    };
    Account {
        key: account_key(h.provider, &h.host),
        provider: h.provider,
        host: h.host.clone(),
        user,
        state,
        active: match h.provider {
            ProviderId::Github => active.or_else(|| h.user.clone()),
            ProviderId::Gitlab => None,
        },
        accounts: h.logins.iter().map(|l| l.login.clone()).collect(),
        account: match h.provider {
            ProviderId::Github => choice.account,
            ProviderId::Gitlab => None,
        },
        enabled,
        scheme: h.scheme.clone(),
        login_command: login_command(h.provider, &h.host),
    }
}

/// What one provider is built from: its platform and host, plus gh's
/// account (GitHub) or the instance URL (GitLab).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSpec {
    pub provider: ProviderId,
    pub host: String,
    /// GitHub: the gh account to use; `None` is gh's active one.
    pub account: Option<String>,
    /// GitLab: `<scheme>://<host>` (glab's API protocol, else https), for
    /// web links; glab is asked for the host alone.
    pub base_url: String,
}

/// The providers `settings` and `discovered` call for: one per enabled
/// host whose login works or could not be checked.
pub(crate) fn provider_specs(settings: &Settings, discovered: &Discovered) -> Vec<ProviderSpec> {
    list(settings, discovered)
        .into_iter()
        .filter(|a| a.enabled && !a.host.is_empty() && a.state.usable())
        .map(|a| ProviderSpec {
            base_url: match a.provider {
                ProviderId::Github => String::new(),
                ProviderId::Gitlab => {
                    format!("{}://{}", a.scheme.as_deref().unwrap_or("https"), a.host)
                }
            },
            account: a.account,
            provider: a.provider,
            host: a.host,
        })
        .collect()
}

/// Where the accounts come from.
pub(crate) enum Source {
    /// The real CLIs; `None` finds them on first use (the login shell, so
    /// never on the UI thread).
    Cli {
        gh: Option<String>,
        glab: Option<String>,
    },
    /// A fixed answer (a core over explicit providers: tests).
    Fixed(Discovered),
}

#[derive(Default)]
struct Known {
    discovered: Option<Discovered>,
    /// Ask the CLIs next time.
    stale: bool,
}

/// The CLIs' last answer, asked only when it matters (see the module).
pub(crate) struct AccountBook {
    source: Mutex<Source>,
    known: Mutex<Known>,
    /// Held while the CLIs are asked, so reads that come together ask once.
    asking: Mutex<()>,
    timeout: Duration,
}

impl AccountBook {
    pub(crate) fn new(source: Source) -> Self {
        Self {
            source: Mutex::new(source),
            known: Mutex::new(Known::default()),
            asking: Mutex::new(()),
            timeout: cli::AUTH_TIMEOUT,
        }
    }

    /// Ask from `source` from now on; the next read asks it.
    pub(crate) fn set_source(&self, source: Source) {
        *self.source.lock().expect("account source lock poisoned") = source;
        let mut known = self.known.lock().expect("accounts lock poisoned");
        known.discovered = None;
        known.stale = false;
    }

    /// The programs the providers run (`gh`, `glab`), when they are the
    /// CLIs'; found once here when not given.
    pub(crate) fn programs(&self) -> Option<(String, String)> {
        let mut source = self.source.lock().expect("account source lock poisoned");
        match &mut *source {
            Source::Fixed(_) => None,
            Source::Cli { gh, glab } => {
                let gh = gh
                    .get_or_insert_with(|| crate::platform::gh_binary().unwrap_or("gh").to_string())
                    .clone();
                let glab = glab
                    .get_or_insert_with(|| {
                        crate::platform::glab_binary().unwrap_or("glab").to_string()
                    })
                    .clone();
                Some((gh, glab))
            }
        }
    }

    /// The last answer, without asking; `None` before the first.
    pub(crate) fn cached(&self) -> Option<Discovered> {
        if let Source::Fixed(d) = &*self.source.lock().expect("account source lock poisoned") {
            return Some(d.clone());
        }
        self.known
            .lock()
            .expect("accounts lock poisoned")
            .discovered
            .clone()
    }

    /// The accounts, asking the CLIs the first time, when `recheck`, and
    /// after [`mark_stale`](Self::mark_stale); otherwise the last answer.
    /// Blocking (one request per host).
    pub(crate) fn get(&self, recheck: bool) -> Discovered {
        let _one = self.asking.lock().expect("accounts ask lock poisoned");
        if let Source::Fixed(d) = &*self.source.lock().expect("account source lock poisoned") {
            return d.clone();
        }
        {
            let known = self.known.lock().expect("accounts lock poisoned");
            if let (false, false, Some(d)) = (recheck, known.stale, &known.discovered) {
                return d.clone();
            }
        }
        let Some((gh, glab)) = self.programs() else {
            return Discovered::empty();
        };
        let timeout = self.timeout;
        let (github, gitlab) = std::thread::scope(|s| {
            let gh = s.spawn(|| cli::ask_gh(&gh, timeout));
            let gl = s.spawn(|| cli::ask_glab(&glab, timeout));
            let failed = || CliReport::Failed("the check crashed".into());
            (
                gh.join().unwrap_or_else(|_| failed()),
                gl.join().unwrap_or_else(|_| failed()),
            )
        });
        let mut known = self.known.lock().expect("accounts lock poisoned");
        let mut next = Discovered { github, gitlab };
        let mut stale = false;
        // A CLI that gave no readable answer this time keeps its last one
        // (a slow link must not switch every host off), and is asked again
        // next time.
        if let Some(prev) = &known.discovered {
            for p in [ProviderId::Github, ProviderId::Gitlab] {
                if matches!(next.report(p), CliReport::Failed(_))
                    && matches!(prev.report(p), CliReport::Hosts(_))
                {
                    *next.report_mut(p) = prev.report(p).clone();
                    stale = true;
                }
            }
        }
        known.discovered = Some(next.clone());
        known.stale = stale;
        next
    }

    /// Ask the CLIs on the next read (a settings change of the accounts, a
    /// call refused for the login, a CLI gone).
    pub(crate) fn mark_stale(&self) {
        self.known.lock().expect("accounts lock poisoned").stale = true;
    }

    /// A refresh's calls to `provider`'s `host` went through: a login that
    /// could not be checked works after all.
    pub(crate) fn note_reachable(&self, provider: ProviderId, host: &str) {
        let mut known = self.known.lock().expect("accounts lock poisoned");
        let Some(d) = known.discovered.as_mut() else {
            return;
        };
        if let CliReport::Hosts(hosts) = d.report_mut(provider) {
            for h in hosts.iter_mut().filter(|h| h.host == host) {
                if matches!(h.state, AccountState::Error { .. }) {
                    h.state = AccountState::LoggedIn;
                }
                for l in h.logins.iter_mut() {
                    if l.active && matches!(l.state, AccountState::Error { .. }) {
                        l.state = AccountState::LoggedIn;
                    }
                }
            }
        }
    }
}

/// The hosts known for the Repositories tab's provider inference: the
/// CLIs' last answer and the settings' entries, enabled ones only.
pub(crate) fn enabled_hosts(
    settings: &Settings,
    discovered: Option<&Discovered>,
) -> Vec<(ProviderId, String)> {
    let discovered = discovered.cloned().unwrap_or_else(Discovered::empty);
    list(settings, &discovered)
        .into_iter()
        .filter(|a| a.enabled && !a.host.is_empty())
        .map(|a| (a.provider, a.host))
        .collect()
}
