//! A fake `gh` for the account tests: `auth status --json hosts` prints
//! what the test wrote ([`FakeGh::set_hosts`]; no hosts without it), and a
//! gh without `--json` can be played ([`FakeGh::text_only`]). Every call's
//! arguments are logged. Anything else fails as gh does for an unknown
//! host. Unix-only (a shell script).

#![allow(dead_code)]

use std::path::PathBuf;

const SCRIPT: &str = r##"#!/bin/sh
DIR='@DIR@'
printf '%s\n' "$*" >> "$DIR/calls.log"
if [ "$1 $2" = "auth status" ]; then
  if [ "$3" = "--json" ]; then
    if [ -e "$DIR/text-only" ]; then
      echo 'unknown flag: --json' >&2
      echo 'Usage:  gh auth status [flags]' >&2
      exit 1
    fi
    if [ -e "$DIR/hosts" ]; then cat "$DIR/hosts"; else echo '{"hosts":{}}'; fi
    exit 0
  fi
  if [ -e "$DIR/text" ]; then cat "$DIR/text" >&2; exit 1; fi
  echo 'You are not logged into any GitHub hosts. To log in, run: gh auth login' >&2
  exit 1
fi
echo "gh: Not Found (HTTP 404)" >&2
exit 1
"##;

pub struct FakeGh {
    _dir: tempfile::TempDir,
    dir: PathBuf,
    pub path: String,
}

impl FakeGh {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let path = dir.join("gh");
        super::script::write_executable(&path, &SCRIPT.replace("@DIR@", &dir.to_string_lossy()));
        Self {
            _dir: tmp,
            dir,
            path: path.to_string_lossy().into_owned(),
        }
    }

    /// What `auth status --json hosts` prints from now on.
    pub fn set_hosts(&self, json: &str) {
        std::fs::write(self.dir.join("hosts"), json).unwrap();
    }

    /// Play a gh without `--json`, whose `auth status` prints `text`.
    pub fn text_only(&self, text: &str) {
        std::fs::write(self.dir.join("text-only"), "").unwrap();
        std::fs::write(self.dir.join("text"), text).unwrap();
    }

    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// How often gh was asked for its logins.
    pub fn auth_calls(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| c.starts_with("auth status"))
            .count()
    }
}
