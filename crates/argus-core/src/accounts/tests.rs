//! What gh's and glab's `auth status` output becomes, and the account list
//! and providers made of it. Integration tests (`tests/accounts.rs`) run the
//! same through fake CLIs and a core.

use super::cli::{parse_gh_json, parse_gh_text, parse_glab_json, parse_glab_text};
use super::*;
use crate::settings::AccountChoice;

const GH_JSON: &str = r#"{"hosts":{
  "github.com":[
    {"state":"success","active":true,"host":"github.com","login":"rev-fixture","tokenSource":"keyring","scopes":"gist, read:org, repo","gitProtocol":"https"},
    {"state":"error","error":"The token in keyring is invalid.","active":false,"host":"github.com","login":"old-fixture","tokenSource":"keyring","gitProtocol":"https","token":"gho_fixtureNotARealToken0000000000000000"}
  ],
  "ghe.example.test":[
    {"state":"timeout","error":"timeout trying to log in","active":true,"host":"ghe.example.test","login":"work-fixture","tokenSource":"GH_ENTERPRISE_TOKEN","gitProtocol":"ssh"}
  ]}}"#;

const GLAB_JSON: &str = r#"{"hosts":[
  {"host":"gitlab.com","state":"error","error":"GET https://gitlab.com/api/v4/user: 401 {message: 401 Unauthorized}","token_source":"none","git_protocol":"ssh","api_protocol":"https","api_endpoint":"https://gitlab.com/api/v4/","graphql_endpoint":"https://gitlab.com/api/graphql/"},
  {"host":"gitlab.vpn.test","state":"success","user":"rev-fixture","token_source":"keyring","git_protocol":"ssh","api_protocol":"http","api_endpoint":"http://gitlab.vpn.test/api/v4/","graphql_endpoint":"http://gitlab.vpn.test/api/graphql/"},
  {"host":"gitlab.odd.test","state":"error","error":"glab exploded with glpat-fixtureNotARealToken00000 in hand","token_source":"keyring","api_protocol":"gopher"}
]}"#;

fn gh_hosts() -> Vec<HostLogin> {
    parse_gh_json(GH_JSON).unwrap()
}

fn glab_hosts() -> Vec<HostLogin> {
    parse_glab_json(GLAB_JSON).unwrap()
}

fn found() -> Discovered {
    Discovered {
        github: CliReport::Hosts(gh_hosts()),
        gitlab: CliReport::Hosts(glab_hosts()),
    }
}

#[test]
fn reads_ghs_json_with_several_hosts_and_accounts() {
    let hosts = gh_hosts();
    assert_eq!(hosts.len(), 2);
    let com = &hosts[1];
    assert_eq!(com.host, "github.com");
    assert_eq!(com.user.as_deref(), Some("rev-fixture"));
    assert_eq!(com.state, AccountState::LoggedIn);
    assert_eq!(
        com.logins
            .iter()
            .map(|l| l.login.as_str())
            .collect::<Vec<_>>(),
        ["rev-fixture", "old-fixture"]
    );
    assert!(matches!(
        com.logins[1].state,
        AccountState::NotLoggedIn { .. }
    ));
    let ghe = &hosts[0];
    assert!(
        matches!(ghe.state, AccountState::Error { ref message } if message.contains("could not reach ghe.example.test"))
    );
    // Nothing of gh's own text or its token field is kept.
    let all = format!("{hosts:?}");
    assert!(!all.contains("gho_") && !all.contains("keyring"), "{all}");
    // An empty answer is no hosts; anything else is not gh's JSON.
    assert_eq!(parse_gh_json(r#"{"hosts":{}}"#), Some(Vec::new()));
    for garbage in ["", "not json", "[]", r#"{"hosts":[]}"#, "{\"error\":1}"] {
        assert_eq!(parse_gh_json(garbage), None, "{garbage}");
    }
}

#[test]
fn reads_ghs_text_old_and_new() {
    // 2.40+: several accounts, an active one.
    let text = "github.com\n  \u{2713} Logged in to github.com account rev-fixture (keyring)\n  - Active account: false\n  - Token: gho_****\n  \u{2713} Logged in to github.com account work-fixture (keyring)\n  - Active account: true\n  X Failed to log in to github.com account old-fixture (keyring)\n  - Active account: false\n  - The token in keyring is invalid.\n\nghe.example.test\n  X Timeout trying to log in to ghe.example.test account work-fixture (keyring)\n";
    let hosts = parse_gh_text(text).unwrap();
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].host, "github.com");
    assert_eq!(hosts[0].user.as_deref(), Some("work-fixture"));
    assert_eq!(hosts[0].logins.len(), 3);
    assert!(matches!(hosts[1].state, AccountState::Error { .. }));
    // Before multiple accounts: `as <login>`.
    let old = "github.com\n  \u{2713} Logged in to github.com as rev-fixture (oauth_token)\n  \u{2713} Git operations for github.com configured to use https protocol.\n";
    let hosts = parse_gh_text(old).unwrap();
    assert_eq!(hosts[0].user.as_deref(), Some("rev-fixture"));
    assert_eq!(hosts[0].state, AccountState::LoggedIn);
    // Logged out everywhere.
    assert_eq!(
        parse_gh_text("You are not logged into any GitHub hosts. To log in, run: gh auth login\n"),
        Some(Vec::new())
    );
    assert_eq!(parse_gh_text("panic: something else\n"), None);
}

#[test]
fn reads_glabs_json_and_sorts_its_errors() {
    let hosts = glab_hosts();
    assert_eq!(hosts.len(), 3);
    assert!(matches!(hosts[0].state, AccountState::NotLoggedIn { .. }));
    assert_eq!(hosts[0].scheme.as_deref(), Some("https"));
    assert_eq!(hosts[1].state, AccountState::LoggedIn);
    assert_eq!(hosts[1].user.as_deref(), Some("rev-fixture"));
    assert_eq!(hosts[1].scheme.as_deref(), Some("http"));
    // Anything else: glab's text, its token redacted; no odd scheme.
    let AccountState::Error { message } = &hosts[2].state else {
        panic!("{:?}", hosts[2].state)
    };
    assert!(
        message.contains("exploded") && !message.contains("glpat-"),
        "{message}"
    );
    assert_eq!(hosts[2].scheme, None);
    assert_eq!(parse_glab_json(r#"{"hosts":[]}"#), Some(Vec::new()));
    for garbage in [
        "",
        "x",
        r#"{"error":{"message":"x has not been authenticated with glab"}}"#,
    ] {
        assert_eq!(parse_glab_json(garbage), None, "{garbage}");
    }
}

#[test]
fn reads_glabs_text() {
    let text = "gitlab.com\n  x gitlab.com: API call failed: GET https://gitlab.com/api/v4/user: 401 {message: 401 Unauthorized}\n  \u{2713} Git operations for gitlab.com configured to use ssh protocol.\n  \u{2713} API calls for gitlab.com are made over https protocol.\n  ! No token found (checked config file, keyring, and environment variables).\ngitlab.vpn.test\n  \u{2713} Logged in to gitlab.vpn.test as rev-fixture (keyring)\n  \u{2713} API calls for gitlab.vpn.test are made over http protocol.\n  \u{2713} Token found in operating system keyring: **************************\n";
    let hosts = parse_glab_text(text).unwrap();
    assert_eq!(hosts.len(), 2);
    assert!(matches!(hosts[0].state, AccountState::NotLoggedIn { .. }));
    assert_eq!(hosts[1].user.as_deref(), Some("rev-fixture"));
    assert_eq!(hosts[1].scheme.as_deref(), Some("http"));
    assert_eq!(parse_glab_text("ERROR something\n"), None);
}

#[test]
fn a_host_is_on_when_logged_in_unless_switched_off() {
    let settings = Settings::default();
    let all = list(&settings, &found());
    assert_eq!(
        all.iter()
            .map(|a| (a.key.as_str(), a.enabled))
            .collect::<Vec<_>>(),
        [
            ("github:ghe.example.test", false),
            ("github:github.com", true),
            ("gitlab:gitlab.com", false),
            ("gitlab:gitlab.odd.test", false),
            ("gitlab:gitlab.vpn.test", true),
        ]
    );
    // Providers: only the hosts on by default (logged in); not the ones
    // that are not logged in or could not be checked, until switched on.
    let specs = provider_specs(&settings, &found());
    assert_eq!(
        specs
            .iter()
            .map(|s| (s.host.as_str(), s.base_url.as_str()))
            .collect::<Vec<_>>(),
        [
            ("github.com", ""),
            ("gitlab.vpn.test", "http://gitlab.vpn.test"),
        ]
    );

    // Switched on (an entry), a host that could not be checked is polled
    // and one that is not logged in warns.
    let mut on = Settings::default();
    for key in ["github:ghe.example.test", "gitlab:gitlab.com"] {
        on.accounts.insert(key.into(), AccountChoice::default());
    }
    let rows = list(&on, &found());
    assert!(rows
        .iter()
        .filter(|a| a.host == "gitlab.com" || a.host == "ghe.example.test")
        .all(|a| a.enabled));
    let hosts: Vec<String> = provider_specs(&on, &found())
        .into_iter()
        .map(|s| s.host)
        .collect();
    assert_eq!(hosts, ["ghe.example.test", "github.com", "gitlab.vpn.test"]);
}

#[test]
fn a_host_that_logs_in_later_turns_on_without_an_entry() {
    let mut hosts = glab_hosts();
    hosts
        .iter_mut()
        .find(|h| h.host == "gitlab.com")
        .unwrap()
        .state = AccountState::LoggedIn;
    let d = Discovered {
        github: CliReport::Hosts(Vec::new()),
        gitlab: CliReport::Hosts(hosts),
    };
    let list = list(&Settings::default(), &d);
    assert!(
        list.iter()
            .find(|a| a.host == "gitlab.com")
            .unwrap()
            .enabled
    );
}

#[test]
fn choices_switch_hosts_off_and_pick_gh_accounts() {
    let mut settings = Settings::default();
    settings.accounts.insert(
        "gitlab:gitlab.vpn.test".into(),
        AccountChoice {
            enabled: false,
            account: None,
        },
    );
    settings.accounts.insert(
        "github:github.com".into(),
        AccountChoice {
            enabled: true,
            account: Some("old-fixture".into()),
        },
    );
    // Hosts only the settings name: listed, not logged in.
    settings.accounts.insert(
        "gitlab:gitlab.gone.test:8443".into(),
        AccountChoice::default(),
    );
    let list = list(&settings, &found());
    let com = list.iter().find(|a| a.host == "github.com").unwrap();
    assert_eq!(com.user.as_deref(), Some("old-fixture"));
    assert_eq!(com.active.as_deref(), Some("rev-fixture"));
    assert!(matches!(com.state, AccountState::NotLoggedIn { .. }));
    let gone = list
        .iter()
        .find(|a| a.host == "gitlab.gone.test:8443")
        .unwrap();
    assert_eq!(gone.state, AccountState::NotLoggedIn { message: None });
    assert_eq!(
        gone.login_command,
        "glab auth login --hostname gitlab.gone.test:8443"
    );
    let specs: Vec<String> = provider_specs(&settings, &found())
        .into_iter()
        .map(|s| s.host)
        .collect();
    // github.com's chosen account is logged out; gitlab.vpn.test is off;
    // ghe and odd could not be checked and have no entry: off by default.
    assert!(specs.is_empty(), "{specs:?}");

    // An account gh does not have on the host.
    settings.accounts.insert(
        "github:github.com".into(),
        AccountChoice {
            enabled: true,
            account: Some("nobody-fixture".into()),
        },
    );
    let com = list_of(&settings)
        .into_iter()
        .find(|a| a.host == "github.com")
        .unwrap();
    assert!(
        matches!(com.state, AccountState::NotLoggedIn { message: Some(ref m) } if m.contains("nobody-fixture"))
    );
}

fn list_of(settings: &Settings) -> Vec<Account> {
    list(settings, &found())
}

#[test]
fn a_missing_or_failed_cli_is_one_row() {
    let mut settings = Settings::default();
    settings
        .accounts
        .insert("github:github.com".into(), AccountChoice::default());
    let d = Discovered {
        github: CliReport::Missing,
        gitlab: CliReport::Failed("it did not answer in time".into()),
    };
    let list = list(&settings, &d);
    assert_eq!(list.len(), 2);
    assert!(
        matches!(list[0].state, AccountState::CliMissing { ref hint } if hint.contains("gh auth login"))
    );
    assert!(
        matches!(list[1].state, AccountState::Error { ref message } if message.contains("did not answer"))
    );
    assert!(provider_specs(&settings, &d).is_empty());
    // gh was set up before (an enabled entry): its missing CLI is on; the
    // failed glab row is on; a missing CLI of an unused provider is off.
    assert!(list[0].enabled);
    let none = Discovered {
        github: CliReport::Missing,
        gitlab: CliReport::Missing,
    };
    assert!(super::list(&Settings::default(), &none)
        .iter()
        .all(|a| !a.enabled));
}

#[test]
fn a_fixed_book_never_asks() {
    let book = AccountBook::new(Source::Fixed(found()));
    assert_eq!(book.get(true), found());
    assert_eq!(book.cached(), Some(found()));
    assert_eq!(book.programs(), None);
}

#[test]
fn calls_that_go_through_mark_a_host_logged_in() {
    let book = AccountBook::new(Source::Cli {
        gh: Some("/nonexistent/argus-fixture/gh".into()),
        glab: Some("/nonexistent/argus-fixture/glab".into()),
    });
    assert_eq!(book.cached(), None);
    let d = book.get(false);
    assert_eq!(d.github, CliReport::Missing);
    book.mark_stale();
    assert_eq!(book.get(false), d);

    // ghe.example.test could not be checked; its calls went through.
    book.known.lock().unwrap().discovered = Some(found());
    book.note_reachable(ProviderId::Github, "ghe.example.test");
    let cached = book.cached().unwrap();
    let CliReport::Hosts(hosts) = &cached.github else {
        panic!()
    };
    assert_eq!(hosts[0].state, AccountState::LoggedIn);
}
