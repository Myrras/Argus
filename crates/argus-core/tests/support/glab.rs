//! A fake `glab`: a bash script that answers `glab api` the way glab 1.124
//! does, by forwarding the request to a local GitLab double (a plain
//! `TcpListener` in the test) with `curl`. The double keeps its routes, its
//! request log and its answers; the fake adds what glab adds:
//!
//! - its own login: `PRIVATE-TOKEN: <token>` on every request, REST and
//!   GraphQL alike (glab sends a personal access token that way to both),
//!   so the token reaches GitLab without Argus ever holding it;
//! - `--include` output: status line, headers, a blank line, the body, for
//!   any status; exit 1 for a non-2xx (a 304 included) with GitLab's body
//!   echoed on stderr (`glab: <body> (HTTP <status>)`), as glab echoes the
//!   message;
//! - a request that cannot connect prints no response and Go's
//!   `dial tcp … connection refused` on stderr;
//! - `auth status --all --output json`: the JSON glab prints for every
//!   host it knows, as the test wrote it ([`FakeGlab::set_hosts`]).
//!
//! Each call's arguments are logged one per line to `calls.log`, plus an
//! `env-token=<n>` line counting how often the token shows up in glab's
//! environment (always 0: Argus never puts one there). Unix-only (bash and
//! curl): tests that use it are `#[cfg_attr(windows, ignore)]`.
//!
//! Files next to the log steer it: `hosts` holds the `auth status` answer
//! (no hosts without the file), and `logged-out` makes it send no token
//! (glab without a login for the host calls anonymously).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use argus_core::provider::GitlabProvider;

/// glab's own login in the fake (fictitious).
pub const TOKEN: &str = "glpat-fixture-not-a-real-token";

const SCRIPT: &str = r##"#!/bin/bash
DIR='@DIR@'
TOKEN='@TOKEN@'
printf '%s\n' "$*" >> "$DIR/calls.log"
printf 'env-token=%s\n' "$(env | grep -c -F -- "$TOKEN")" >> "$DIR/calls.log"
if [ "$1" = auth ]; then
  # `auth status --all --output json`: every host glab knows, as the test
  # wrote them (none without the file); glab exits 1 when one failed.
  if [ -e "$DIR/hosts" ]; then cat "$DIR/hosts"; else echo '{"hosts":[]}'; fi
  grep -q '"state":"error"' "$DIR/hosts" 2>/dev/null && exit 1
  exit 0
fi
[ "$1" = api ] || { echo "glab: unknown command \"$1\"" >&2; exit 1; }
shift
host=""; method=GET; input=""; endpoint=""
headers=()
while [ $# -gt 0 ]; do
  case "$1" in
    --hostname) host=$2; shift 2 ;;
    --include|-i) shift ;;
    --method|-X) method=$2; shift 2 ;;
    --header|-H) headers+=(-H "$2"); shift 2 ;;
    --input) input=$2; shift 2 ;;
    *) endpoint=$1; shift ;;
  esac
done
case "$endpoint" in
  graphql) url="http://$host/api/graphql" ;;
  /*) url="http://$host/api/v4$endpoint" ;;
  *) url="http://$host/api/v4/$endpoint" ;;
esac
auth=(-H "PRIVATE-TOKEN: $TOKEN")
[ -e "$DIR/logged-out" ] && auth=()
data=()
[ "$input" = "-" ] && data=(--data-binary @- -H "Content-Type: application/json; charset=utf-8")
out=$(mktemp "$DIR/out.XXXXXX")
curl -sS -i --max-time 20 -X "$method" -H 'Expect:' "${auth[@]}" "${headers[@]}" "${data[@]}" -o "$out" "$url" 2>/dev/null
if [ $? -ne 0 ] || [ ! -s "$out" ]; then
  rm -f "$out"
  echo "$method \"$url\": dial tcp $host: connect: connection refused" >&2
  exit 1
fi
cat "$out"
status=$(head -1 "$out" | cut -d' ' -f2)
case "$status" in
  2??) rm -f "$out"; exit 0 ;;
esac
body=$(sed '1,/^\r*$/d' "$out")
rm -f "$out"
echo "glab: $body (HTTP $status)" >&2
exit 1
"##;

/// One fake glab in its own directory.
pub struct FakeGlab {
    /// Kept for the test's length; `None` for the shared one.
    _dir: Option<tempfile::TempDir>,
    dir: PathBuf,
    pub path: String,
}

impl FakeGlab {
    /// A fake logged in with [`TOKEN`].
    pub fn new() -> Self {
        Self::with_token(TOKEN)
    }

    /// A fake whose login is `token`.
    pub fn with_token(token: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let mut fake = Self::write(&dir, token);
        fake._dir = Some(tmp);
        fake
    }

    fn write(dir: &Path, token: &str) -> Self {
        let path = dir.join("glab");
        let script = SCRIPT
            .replace("@DIR@", &dir.to_string_lossy())
            .replace("@TOKEN@", token);
        super::script::write_executable(&path, &script);
        Self {
            _dir: None,
            dir: dir.to_path_buf(),
            path: path.to_string_lossy().into_owned(),
        }
    }

    /// One fake for the whole test binary, logged in with [`TOKEN`], for
    /// tests that never look at its log (its directory stays under
    /// Cargo's target tmp dir).
    pub fn shared() -> &'static FakeGlab {
        static SHARED: std::sync::OnceLock<FakeGlab> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| {
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("fake-glab-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self::write(&dir, TOKEN)
        })
    }

    /// A provider for the double at `base` (`http://127.0.0.1:<port>`)
    /// through this fake.
    pub fn provider(&self, base: &str) -> GitlabProvider {
        GitlabProvider::with_binary(&self.path, base).unwrap()
    }

    /// The argument lines of every call so far, without the env lines.
    pub fn calls(&self) -> Vec<String> {
        self.log()
            .lines()
            .filter(|l| !l.starts_with("env-token="))
            .map(str::to_string)
            .collect()
    }

    /// The whole log, env lines included.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("calls.log")).unwrap_or_default()
    }

    /// What `auth status --all --output json` prints from now on.
    pub fn set_hosts(&self, json: &str) {
        std::fs::write(self.dir.join("hosts"), json).unwrap();
    }

    /// How often glab was asked for its logins.
    pub fn auth_calls(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| c.starts_with("auth status"))
            .count()
    }

    /// Send no token from now on (glab without a login calls anonymously).
    pub fn log_out(&self) {
        std::fs::write(self.dir.join("logged-out"), "").unwrap();
    }

    /// Send the token again (after [`log_out`](Self::log_out)).
    pub fn log_in(&self) {
        let _ = std::fs::remove_file(self.dir.join("logged-out"));
    }
}
