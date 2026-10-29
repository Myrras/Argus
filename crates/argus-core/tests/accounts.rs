//! Settings → Accounts end to end: what a fake `gh` and a fake `glab` say
//! they are logged in to becomes the account list and one provider per
//! enabled host; the CLIs are asked only when it matters. GitLab hosts are
//! local doubles (`support::gitlab`), reached through the fake glab. No
//! network, no real token. Unix-only (the fakes are shell scripts).

#![cfg_attr(windows, allow(dead_code, unused_imports))]

mod support;

use argus_core::provider::ProviderId;
use argus_core::store::Store;
use argus_core::{Account, AccountChoice, AccountState, Core, Settings};

use support::gh::FakeGh;
use support::gitlab::{instance, reply, serve, FakeGlab, Log};

/// `base` (`http://127.0.0.1:<port>`) as glab names the host.
fn host_of(base: &str) -> String {
    base.trim_start_matches("http://").to_string()
}

/// glab's `auth status --all --output json` for `hosts` (host, state,
/// user, error).
fn glab_hosts(hosts: &[(&str, &str, Option<&str>, Option<&str>)]) -> String {
    let entries: Vec<serde_json::Value> = hosts
        .iter()
        .map(|(host, state, user, error)| {
            let mut e = serde_json::json!({
                "host": host,
                "state": state,
                "token_source": "keyring",
                "git_protocol": "ssh",
                "api_protocol": "http",
                "api_endpoint": format!("http://{host}/api/v4/"),
            });
            if let Some(u) = user {
                e["user"] = (*u).into();
            }
            if let Some(err) = error {
                e["error"] = (*err).into();
            }
            e
        })
        .collect();
    serde_json::json!({ "hosts": entries }).to_string()
}

fn row<'a>(accounts: &'a [Account], host: &str) -> &'a Account {
    accounts
        .iter()
        .find(|a| a.host == host)
        .unwrap_or_else(|| panic!("no row for {host}: {accounts:?}"))
}

fn switched(core: &Core, provider: ProviderId, host: &str, on: bool) -> Settings {
    let mut s = core.settings();
    s.accounts.insert(
        argus_core::settings::account_key(provider, host),
        AccountChoice {
            enabled: on,
            account: None,
        },
    );
    s
}

/// Settings with the GitLab host `host` switched on (an entry): a host glab
/// could not check is off without one.
fn on_for(host: &str) -> Settings {
    let mut s = Settings::default();
    s.accounts.insert(
        argus_core::settings::account_key(ProviderId::Gitlab, host),
        AccountChoice::default(),
    );
    s
}

fn requests(log: &Log) -> usize {
    log.lock().unwrap().len()
}

/// Real-shaped answers: gh with two accounts on github.com (the second
/// active), a GHE host whose token gh no longer accepts; glab with
/// gitlab.com without a token (glab 1.124's 401), a working host over
/// http and one it could not reach.
#[test]
#[cfg_attr(windows, ignore)]
fn every_host_of_both_clis_is_an_account() {
    let gh = FakeGh::new();
    gh.set_hosts(
        r#"{"hosts":{
          "github.com":[
            {"state":"success","active":false,"host":"github.com","login":"rev-fixture","tokenSource":"keyring","scopes":"repo","gitProtocol":"https"},
            {"state":"success","active":true,"host":"github.com","login":"work-fixture","tokenSource":"keyring","scopes":"repo","gitProtocol":"https"}
          ],
          "ghe.example.test":[
            {"state":"error","error":"The token in keyring is invalid.","active":true,"host":"ghe.example.test","login":"rev-fixture","tokenSource":"keyring","gitProtocol":"https"}
          ]}}"#,
    );
    let glab = FakeGlab::new();
    glab.set_hosts(&glab_hosts(&[
        (
            "gitlab.com",
            "error",
            None,
            Some("GET https://gitlab.com/api/v4/user: 401 {message: 401 Unauthorized}"),
        ),
        ("gitlab.example.test", "success", Some("rev-fixture"), None),
        (
            "gitlab.down.test",
            "error",
            None,
            Some("Get \"http://gitlab.down.test/api/v4/user\": dial tcp: lookup gitlab.down.test: no such host"),
        ),
    ]));
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        &gh.path,
        &glab.path,
    );
    let accounts = core.list_accounts(false);
    let hosts: Vec<(&str, &str)> = accounts
        .iter()
        .map(|a| (a.provider.as_str(), a.host.as_str()))
        .collect();
    assert_eq!(
        hosts,
        [
            ("github", "ghe.example.test"),
            ("github", "github.com"),
            ("gitlab", "gitlab.com"),
            ("gitlab", "gitlab.down.test"),
            ("gitlab", "gitlab.example.test"),
        ]
    );

    let gh_com = row(&accounts, "github.com");
    assert_eq!(gh_com.key, "github:github.com");
    assert_eq!(gh_com.state, AccountState::LoggedIn);
    assert_eq!(gh_com.user.as_deref(), Some("work-fixture"));
    assert_eq!(gh_com.active.as_deref(), Some("work-fixture"));
    assert_eq!(gh_com.accounts, ["rev-fixture", "work-fixture"]);
    assert_eq!(gh_com.account, None);
    assert!(gh_com.enabled, "a new host is on");
    assert_eq!(gh_com.login_command, "gh auth login --hostname github.com");

    let ghe = row(&accounts, "ghe.example.test");
    assert!(matches!(ghe.state, AccountState::NotLoggedIn { .. }));
    // gh's own words never pass.
    assert!(!format!("{:?}", ghe.state).contains("keyring"));

    let com = row(&accounts, "gitlab.com");
    assert!(matches!(com.state, AccountState::NotLoggedIn { .. }));
    assert_eq!(com.login_command, "glab auth login --hostname gitlab.com");
    let down = row(&accounts, "gitlab.down.test");
    assert!(
        matches!(down.state, AccountState::Error { ref message } if message.contains("could not reach"))
    );
    let ok = row(&accounts, "gitlab.example.test");
    assert_eq!(ok.state, AccountState::LoggedIn);
    assert_eq!(ok.user.as_deref(), Some("rev-fixture"));
    assert_eq!(ok.scheme.as_deref(), Some("http"));
    assert!(ok.accounts.is_empty());

    // The other gh account: picked in settings, it is the one shown.
    let mut s = core.settings();
    s.accounts.insert(
        "github:github.com".into(),
        AccountChoice {
            enabled: true,
            account: Some("rev-fixture".into()),
        },
    );
    core.update_settings(s).unwrap();
    let gh_com = core
        .list_accounts(false)
        .into_iter()
        .find(|a| a.host == "github.com")
        .unwrap();
    assert_eq!(gh_com.user.as_deref(), Some("rev-fixture"));
    assert_eq!(gh_com.account.as_deref(), Some("rev-fixture"));
    assert_eq!(gh_com.active.as_deref(), Some("work-fixture"));

    // The wire shape the webview reads.
    let json = serde_json::to_value(&accounts).unwrap();
    assert_eq!(json[0]["state"]["kind"], "notLoggedIn");
    assert_eq!(
        json[1]["loginCommand"],
        "gh auth login --hostname github.com"
    );
}

/// A gh before `auth status --json`: its text output is read.
#[test]
#[cfg_attr(windows, ignore)]
fn an_older_gh_is_read_from_its_text() {
    let gh = FakeGh::new();
    gh.text_only(
        "github.com\n  \u{2713} Logged in to github.com account rev-fixture (keyring)\n  - Active account: false\n  - Git operations protocol: https\n  - Token: gho_************************************\n  \u{2713} Logged in to github.com account work-fixture (keyring)\n  - Active account: true\n\nghe.example.test\n  X Failed to log in to ghe.example.test account rev-fixture (keyring)\n  - Active account: true\n  - The token in keyring is invalid.\n",
    );
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        &gh.path,
        "/nonexistent/argus-fixture/glab",
    );
    let accounts = core.list_accounts(false);
    let com = row(&accounts, "github.com");
    assert_eq!(com.state, AccountState::LoggedIn);
    assert_eq!(com.active.as_deref(), Some("work-fixture"));
    assert_eq!(com.accounts, ["rev-fixture", "work-fixture"]);
    assert!(matches!(
        row(&accounts, "ghe.example.test").state,
        AccountState::NotLoggedIn { .. }
    ));
    // glab is not installed: one row that says so.
    let gl: Vec<&Account> = accounts
        .iter()
        .filter(|a| a.provider == ProviderId::Gitlab)
        .collect();
    assert_eq!(gl.len(), 1);
    assert!(matches!(gl[0].state, AccountState::CliMissing { ref hint } if hint.contains("glab")));
    assert_eq!(gl[0].host, "");
    assert_eq!(
        gh.calls(),
        ["auth status --json hosts", "auth status"],
        "never --show-token"
    );
}

/// Neither CLI installed: one row per provider, nothing polled.
#[test]
fn missing_clis_are_one_row_each() {
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        "/nonexistent/argus-fixture/gh",
        "/nonexistent/argus-fixture/glab",
    );
    let accounts = core.list_accounts(false);
    assert_eq!(accounts.len(), 2);
    assert!(accounts
        .iter()
        .all(|a| matches!(a.state, AccountState::CliMissing { .. })));
    assert_eq!(accounts[0].login_command, "gh auth login");
    assert!(core.refresh().errors.is_empty());
}

/// Garbage from a CLI: an error row, and the last good answer is kept.
#[test]
#[cfg_attr(windows, ignore)]
fn a_cli_answer_argus_cannot_read_keeps_the_last_one() {
    let gh = FakeGh::new();
    let glab = FakeGlab::new();
    glab.set_hosts("not json at all");
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        &gh.path,
        &glab.path,
    );
    let accounts = core.list_accounts(false);
    assert_eq!(accounts.len(), 1);
    assert!(
        matches!(accounts[0].state, AccountState::Error { ref message } if message.contains("glab"))
    );

    glab.set_hosts(&glab_hosts(&[(
        "gitlab.example.test",
        "success",
        Some("rev-fixture"),
        None,
    )]));
    assert_eq!(core.list_accounts(true)[0].host, "gitlab.example.test");
    glab.set_hosts("{\"garbage\": true}");
    let accounts = core.list_accounts(true);
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].state, AccountState::LoggedIn);
    // Asked again next time, as it gave no answer.
    let before = glab.auth_calls();
    core.list_accounts(false);
    assert_eq!(glab.auth_calls(), before + 1);
}

/// Two GitLab hosts, each its own provider: both polled, independently; a
/// host switched off is not polled and its reviews leave the list but stay
/// stored, back when it is switched on again.
#[test]
#[cfg_attr(windows, ignore)]
fn every_enabled_host_is_polled_on_its_own() {
    let (a, a_log) = serve(instance);
    let (b, b_log) = serve(instance);
    let (ha, hb) = (host_of(&a), host_of(&b));
    let gh = FakeGh::new();
    let glab = FakeGlab::new();
    glab.set_hosts(&glab_hosts(&[
        (&ha, "success", Some("rev-fixture"), None),
        (&hb, "success", Some("rev-fixture"), None),
    ]));
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        &gh.path,
        &glab.path,
    );
    // The first refresh asks the CLIs itself.
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut hosts: Vec<String> = core
        .reviews()
        .unwrap()
        .into_iter()
        .map(|r| r.review.host)
        .collect();
    hosts.sort();
    let mut both = vec![ha.clone(), hb.clone()];
    both.sort();
    assert_eq!(hosts, both);
    assert!(requests(&a_log) > 0 && requests(&b_log) > 0);
    let b_review = core
        .reviews()
        .unwrap()
        .into_iter()
        .find(|r| r.review.host == hb)
        .unwrap()
        .id;

    // B off: not polled, out of the list, still stored.
    core.update_settings(switched(&core, ProviderId::Gitlab, &hb, false))
        .unwrap();
    let off = core.list_accounts(false);
    assert!(!row(&off, &hb).enabled);
    let (before_a, before_b) = (requests(&a_log), requests(&b_log));
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(requests(&a_log) > before_a);
    assert_eq!(requests(&b_log), before_b);
    let listed: Vec<String> = core
        .reviews()
        .unwrap()
        .into_iter()
        .map(|r| r.review.host)
        .collect();
    assert_eq!(listed, std::slice::from_ref(&ha));
    assert_eq!(core.review(b_review).unwrap().review.host, hb);

    // B on again: listed and polled again.
    core.update_settings(switched(&core, ProviderId::Gitlab, &hb, true))
        .unwrap();
    assert_eq!(core.reviews().unwrap().len(), 2);
    core.refresh();
    assert!(requests(&b_log) > before_b);

    // One host failing leaves the other alone, with its host named.
    drop(b_log);
    let (c, _c_log) = serve(|_| reply("500 Internal Server Error", "{}"));
    let hc = host_of(&c);
    glab.set_hosts(&glab_hosts(&[
        (&ha, "success", Some("rev-fixture"), None),
        (&hc, "success", Some("rev-fixture"), None),
    ]));
    // A host new to glab is on by default.
    let accounts = core.list_accounts(true);
    assert!(row(&accounts, &hc).enabled);
    let report = core.refresh();
    assert!(
        report.errors.iter().all(|e| e.host == hc),
        "{:?}",
        report.errors
    );
    assert!(!report.errors.is_empty());
}

/// The CLIs are asked the first time, on Check again, after a change of
/// the accounts and after a call refused for the login or without its
/// CLI; never on a plain refresh or another settings change.
#[test]
#[cfg_attr(windows, ignore)]
fn the_clis_are_asked_only_when_it_matters() {
    // Anonymous calls (glab without a login) get GitLab's 401.
    let (a, _log) = serve(|req| {
        if req.token {
            instance(req)
        } else {
            reply("401 Unauthorized", r#"{"message":"401 Unauthorized"}"#)
        }
    });
    let ha = host_of(&a);
    let gh = FakeGh::new();
    let glab = FakeGlab::new();
    glab.set_hosts(&glab_hosts(&[(&ha, "success", Some("rev-fixture"), None)]));
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        Settings::default(),
        &gh.path,
        &glab.path,
    );
    let asked = || (gh.auth_calls(), glab.auth_calls());

    core.list_accounts(false);
    assert_eq!(asked(), (1, 1));
    // A plain refresh, and the list after it: not asked.
    assert!(core.refresh().errors.is_empty());
    core.list_accounts(false);
    core.refresh();
    assert_eq!(asked(), (1, 1));

    // Check again always asks.
    core.list_accounts(true);
    assert_eq!(asked(), (2, 2));

    // Another setting: not asked. The accounts: asked on the next read.
    let mut s = core.settings();
    s.poll_interval_secs = 600;
    core.update_settings(s).unwrap();
    core.list_accounts(false);
    assert_eq!(asked(), (2, 2));
    core.update_settings(switched(
        &core,
        ProviderId::Github,
        "ghe.example.test",
        false,
    ))
    .unwrap();
    core.list_accounts(false);
    assert_eq!(asked(), (3, 3));
    core.list_accounts(false);
    assert_eq!(asked(), (3, 3));

    // glab loses its login: the refresh's 401 has the next read ask.
    glab.log_out();
    glab.set_hosts(&glab_hosts(&[(
        &ha,
        "error",
        None,
        Some("401 {message: 401 Unauthorized}"),
    )]));
    let report = core.refresh();
    assert!(report.errors.iter().any(|e| e.unavailable && e.host == ha));
    let accounts = core.list_accounts(false);
    assert_eq!(asked(), (4, 4));
    assert!(matches!(
        row(&accounts, &ha).state,
        AccountState::NotLoggedIn { .. }
    ));
    // Not logged in: no provider, nothing polled, nothing asked.
    assert!(core.refresh().errors.is_empty());
    core.list_accounts(false);
    assert_eq!(asked(), (4, 4));

    // Logged in again (in a terminal), then Check again: polled again.
    glab.log_in();
    glab.set_hosts(&glab_hosts(&[(&ha, "success", Some("rev-fixture"), None)]));
    core.list_accounts(true);
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    // glab uninstalled: the call finds no glab, the next read asks again.
    std::fs::remove_file(&glab.path).unwrap();
    let report = core.refresh();
    assert!(report.errors.iter().any(|e| e.unavailable));
    let accounts = core.list_accounts(false);
    assert!(accounts
        .iter()
        .any(|a| matches!(a.state, AccountState::CliMissing { .. })));
}

/// A host glab could not reach when asked is polled once switched on; once its calls
/// go through it shows logged in, without asking glab again.
#[test]
#[cfg_attr(windows, ignore)]
fn a_host_whose_calls_go_through_is_logged_in() {
    let (a, _log) = serve(instance);
    let ha = host_of(&a);
    let gh = FakeGh::new();
    let glab = FakeGlab::new();
    glab.set_hosts(&glab_hosts(&[(
        &ha,
        "error",
        None,
        Some("dial tcp: connect: connection refused"),
    )]));
    let core = Core::with_cli(
        Store::in_memory().unwrap(),
        on_for(&ha),
        &gh.path,
        &glab.path,
    );
    assert!(matches!(
        row(&core.list_accounts(false), &ha).state,
        AccountState::Error { .. }
    ));
    let report = core.refresh();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        row(&core.list_accounts(false), &ha).state,
        AccountState::LoggedIn
    );
    assert_eq!(glab.auth_calls(), 1);
}

/// Settings from an older Argus name a host neither CLI knows: listed, not
/// logged in, with the command that logs in to it.
#[test]
#[cfg_attr(windows, ignore)]
fn a_host_only_the_settings_name_shows_how_to_log_in() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.json");
    std::fs::write(
        &file,
        r#"{"github":{"enabled":true,"host":"github.com","account":"work-fixture"},"gitlab":{"baseUrl":"https://gitlab.example.test:8443"}}"#,
    )
    .unwrap();
    let settings = Settings::load(&file);
    let gh = FakeGh::new();
    let glab = FakeGlab::new();
    let core = Core::with_cli(Store::in_memory().unwrap(), settings, &gh.path, &glab.path);
    let accounts = core.list_accounts(false);
    let gl = row(&accounts, "gitlab.example.test:8443");
    assert!(gl.enabled);
    assert_eq!(gl.state, AccountState::NotLoggedIn { message: None });
    assert_eq!(
        gl.login_command,
        "glab auth login --hostname gitlab.example.test:8443"
    );
    let com = row(&accounts, "github.com");
    assert_eq!(com.account.as_deref(), Some("work-fixture"));
    assert!(core.refresh().errors.is_empty());
}
