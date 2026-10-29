//! Submit scripts: the user's own programs that run after a successful
//! Submit review, filtered by verdict and repository.
//!
//! The one way a script runs is [`Core::submit_review_with_scripts`]: the
//! review is submitted first, and only when that succeeded do the scripts
//! the user ticked for this submit run (each still has to match the
//! verdict and the repository; one that does not, or is gone from the
//! settings, is reported as skipped). Nothing else starts one: no refresh,
//! poll, AI run or settings change.
//!
//! They run in the background, one after the other in the order of the
//! settings, so the submit's reply never waits on them; each batch ends in
//! one [`ScriptsDone`] through [`Core::on_scripts_done`] (the shell's
//! `scripts:done` event). A script is spawned directly (no shell) with
//! its arguments, in the review's local clone when one is set (else a
//! fresh temp dir), with the user's environment and login shell `PATH`
//! minus every token variable, plus the `ARGUS_*` variables that describe
//! the review; the same as one JSON object on stdin, with the summary. See
//! [`run`]. Argus holds no platform token of its own (`gh` and `glab`
//! keep theirs), so none can go near a script.
//!
//! [`drafts`] writes a script with the AI pass' agent CLI (no tools) as a
//! conversation in the background ([`generate`] is one turn), and
//! [`Core::save_submit_script`] stores one in Argus' own `scripts/`
//! folder. Neither runs anything.

pub(crate) mod attach;
pub(crate) mod drafts;
pub(crate) mod generate;
pub(crate) mod run;
pub(crate) mod runtime;
mod save;

pub(crate) use save::SCRIPTS_DIR;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

pub use drafts::{
    DraftStage, DraftTurn, ScriptDraft, ScriptDraftCallback, ScriptDraftProgress,
    ScriptDraftProgressCallback, ScriptDraftProgressEvent, ScriptDraftStatus,
};
pub use generate::{GeneratedScript, ScriptGenerationStatus};

use crate::ids::ReviewId;
use crate::post::PostError;
use crate::provider::ReviewVerdict;
use crate::settings::repo_key;
use crate::store::ReviewRecord;
use crate::Core;

/// What a new script waits at most, and what a missing value is.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
/// The shortest timeout a script can have.
pub const MIN_TIMEOUT_SECS: u64 = 5;
/// The longest timeout a script can have.
pub const MAX_TIMEOUT_SECS: u64 = 600;
/// Most of a script's stdout and of its stderr Argus keeps (the tail).
pub const MAX_OUTPUT: usize = 16 * 1024;

/// One script of the user's, as `settings.json` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubmitScript {
    /// A UUID string; what the submit dialog ticks.
    pub id: String,
    pub name: String,
    /// The executable: an absolute path, `~/…`, or a bare name found on
    /// the login shell's `PATH`.
    pub command: String,
    pub args: Vec<String>,
    /// The verdicts it is offered for; none means never.
    #[serde(deserialize_with = "lenient_verdicts")]
    pub verdicts: Vec<ReviewVerdict>,
    /// `host/repo` keys (as [`crate::settings::repo_key`]); empty means
    /// every repository.
    pub repos: Vec<String>,
    /// Ticked in the submit dialog unless the user unticks it.
    pub default_on: bool,
    pub timeout_secs: u64,
}

impl Default for SubmitScript {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            verdicts: vec![ReviewVerdict::Approve],
            repos: Vec::new(),
            default_on: false,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

impl SubmitScript {
    /// Whether the script is offered for `verdict`.
    pub fn matches_verdict(&self, verdict: ReviewVerdict) -> bool {
        self.verdicts.contains(&verdict)
    }

    /// Whether the script is offered for the repository `repo` (`host/repo`):
    /// every one without a list, else one on it (case and a trailing slash
    /// do not count: GitLab paths are case-insensitive).
    pub fn matches_repo(&self, repo: &str) -> bool {
        let norm = |r: &str| r.trim().trim_end_matches('/').to_ascii_lowercase();
        let repo = norm(repo);
        self.repos.is_empty() || self.repos.iter().any(|r| norm(r) == repo)
    }

    /// The timeout the script runs with, within the bounds.
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS))
    }

    /// The name to show: the user's, else the command's file name.
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        let command = self.command.trim();
        std::path::Path::new(command)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Untitled script".into())
    }
}

/// `submitScripts` read leniently: anything but a list is none, and an
/// entry that cannot be read is left out (the rest stay). An entry without
/// an id gets one, so the submit dialog can tick it.
pub(crate) fn lenient_scripts<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<SubmitScript>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let serde_json::Value::Array(items) = value else {
        return Ok(Vec::new());
    };
    let scripts = items
        .into_iter()
        .filter(serde_json::Value::is_object)
        .filter_map(|item| serde_json::from_value::<SubmitScript>(item).ok())
        .collect();
    Ok(with_ids(scripts))
}

/// A script's `verdicts` read leniently: values Argus does not know (a
/// newer Argus, a typo) are left out, each kept once.
fn lenient_verdicts<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ReviewVerdict>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let serde_json::Value::Array(items) = value else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for v in items
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
    {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(out)
}

/// Every script with an id of its own: a blank or repeated one gets a
/// fresh UUID.
fn with_ids(scripts: Vec<SubmitScript>) -> Vec<SubmitScript> {
    let mut seen = std::collections::HashSet::new();
    scripts
        .into_iter()
        .map(|mut s| {
            s.id = s.id.trim().to_string();
            if s.id.is_empty() || !seen.insert(s.id.clone()) {
                s.id = uuid::Uuid::new_v4().to_string();
                seen.insert(s.id.clone());
            }
            s
        })
        .collect()
}

/// Tidy scripts before a save: ids as in [`lenient_scripts`]; name and
/// command trimmed; verdicts each once; repositories trimmed (a trailing
/// slash too), blank ones dropped, each once; the timeout within bounds.
/// The arguments stay exactly as given (spaces can matter there). A
/// script without a command is kept: the Scripts tab adds one blank first.
pub(crate) fn tidy(scripts: Vec<SubmitScript>) -> Vec<SubmitScript> {
    with_ids(scripts)
        .into_iter()
        .map(|mut s| {
            s.name = s.name.trim().to_string();
            s.command = s.command.trim().to_string();
            let mut verdicts = Vec::new();
            for v in s.verdicts {
                if !verdicts.contains(&v) {
                    verdicts.push(v);
                }
            }
            s.verdicts = verdicts;
            let mut repos: Vec<String> = Vec::new();
            for r in s.repos {
                let r = r.trim().trim_end_matches('/').to_string();
                if !r.is_empty() && !repos.iter().any(|k| k.eq_ignore_ascii_case(&r)) {
                    repos.push(r);
                }
            }
            s.repos = repos;
            s.timeout_secs = s.timeout_secs.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
            s
        })
        .collect()
}

/// How one script's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptStatus {
    /// Exited with status 0.
    Ok,
    /// Exited with another status, was killed, or could not start.
    Failed,
    /// Ran longer than its timeout and was killed.
    TimedOut,
    /// Not run: gone from the settings, or not set for this verdict or
    /// repository.
    Skipped,
    /// The command is not there (or not set).
    NotFound,
    /// The command is there but cannot be run (`chmod +x`, a folder).
    NotExecutable,
}

/// One script's outcome after a submit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResult {
    pub script_id: String,
    pub name: String,
    pub status: ScriptStatus,
    /// `None` when it did not run to an exit (not started, killed).
    pub exit_code: Option<i32>,
    /// The tail of what it printed, at most [`MAX_OUTPUT`] bytes each.
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    /// Argus' own explanation, e.g. "not executable — run chmod +x …".
    pub message: Option<String>,
    pub finished_at: DateTime<Utc>,
}

impl ScriptResult {
    /// A result for a script that did not run.
    pub(crate) fn without_run(
        script_id: &str,
        name: &str,
        status: ScriptStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            script_id: script_id.to_string(),
            name: name.to_string(),
            status,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
            message: Some(message.into()),
            finished_at: Utc::now(),
        }
    }
}

/// What one submit's scripts did: the `scripts:done` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptsDone {
    pub review_id: ReviewId,
    /// One per ticked id: the configured order, then unknown ids.
    pub results: Vec<ScriptResult>,
}

/// What [`Core::on_scripts_done`] registers: called (on the scripts'
/// thread) once a submit's scripts have all ended.
pub type ScriptsDoneCallback = Arc<dyn Fn(&ScriptsDone) + Send + Sync>;

/// One of a submit's ticked scripts: run it, or report why not.
#[derive(Debug, Clone)]
pub(crate) enum Planned {
    Run(SubmitScript),
    Skip(ScriptResult),
}

/// The verdict as people say it, for messages.
fn verdict_label(v: ReviewVerdict) -> &'static str {
    match v {
        ReviewVerdict::Approve => "approve",
        ReviewVerdict::RequestChanges => "request changes",
        ReviewVerdict::Comment => "comment",
    }
}

/// What to do with the ticked `chosen` ids after a submit of `verdict` on
/// `repo`: the scripts that match run, in the settings' order; a ticked one
/// that does not match the verdict or repository, and an id the settings
/// do not have (any more), is skipped with the reason. Each id once.
pub(crate) fn plan(
    scripts: &[SubmitScript],
    chosen: &[String],
    verdict: ReviewVerdict,
    repo: &str,
) -> Vec<Planned> {
    let mut wanted: Vec<&str> = Vec::new();
    for id in chosen
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
    {
        if !wanted.contains(&id) {
            wanted.push(id);
        }
    }
    let mut out: Vec<Planned> = scripts
        .iter()
        .filter(|s| wanted.contains(&s.id.as_str()))
        .map(|s| {
            let skip = |why: String| {
                Planned::Skip(ScriptResult::without_run(
                    &s.id,
                    &s.display_name(),
                    ScriptStatus::Skipped,
                    why,
                ))
            };
            if !s.matches_verdict(verdict) {
                skip(format!(
                    "Not set to run when you {}.",
                    verdict_label(verdict)
                ))
            } else if !s.matches_repo(repo) {
                skip(format!("Not set to run for {repo}."))
            } else {
                Planned::Run(s.clone())
            }
        })
        .collect();
    for id in wanted {
        if !scripts.iter().any(|s| s.id == id) {
            out.push(Planned::Skip(ScriptResult::without_run(
                id,
                "Unknown script",
                ScriptStatus::Skipped,
                "This script is no longer in Settings → Scripts.",
            )));
        }
    }
    out
}

/// What a script is told about the review: the `ARGUS_*` variables, and
/// (with the summary) the JSON object on its stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScriptContext {
    /// `gitlab` or `github`.
    pub provider: String,
    pub host: String,
    /// `group/project` (GitLab) or `owner/repo` (GitHub).
    pub repo: String,
    /// The MR iid or PR number.
    pub number: u64,
    pub url: String,
    pub title: String,
    /// `approve`, `requestChanges` or `comment`.
    pub verdict: String,
    pub head_sha: String,
    /// The local clone's folder; empty without one.
    pub clone: String,
    /// The review summary; empty without one. Stdin only.
    pub summary: String,
}

impl ScriptContext {
    pub(crate) fn new(
        record: &ReviewRecord,
        verdict: ReviewVerdict,
        head: &str,
        summary: Option<&str>,
        clone: Option<&std::path::Path>,
    ) -> Self {
        let r = &record.review;
        Self {
            provider: r.provider.as_str().to_string(),
            host: r.host.clone(),
            repo: r.repo.clone(),
            number: r.number,
            url: r.url.clone(),
            title: r.title.clone(),
            verdict: verdict.as_str().to_string(),
            head_sha: head.to_string(),
            clone: clone
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            summary: summary.map(str::trim).unwrap_or_default().to_string(),
        }
    }

    /// The `ARGUS_*` variables (the summary is stdin only: it can be long
    /// and hold anything).
    pub(crate) fn env(&self) -> Vec<(String, String)> {
        [
            ("ARGUS_PROVIDER", self.provider.clone()),
            ("ARGUS_HOST", self.host.clone()),
            ("ARGUS_REPO", self.repo.clone()),
            ("ARGUS_NUMBER", self.number.to_string()),
            ("ARGUS_URL", self.url.clone()),
            ("ARGUS_TITLE", self.title.clone()),
            ("ARGUS_VERDICT", self.verdict.clone()),
            ("ARGUS_HEAD_SHA", self.head_sha.clone()),
            ("ARGUS_CLONE", self.clone.clone()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    /// The JSON object written to stdin.
    pub(crate) fn stdin_json(&self) -> String {
        serde_json::to_string(self).expect("a script context always encodes")
    }
}

/// One script to run, with what it is told.
#[derive(Debug, Clone)]
pub(crate) struct ScriptJob {
    pub script: SubmitScript,
    pub context: ScriptContext,
    /// The working directory: the review's clone, when set and there.
    pub clone_dir: Option<PathBuf>,
}

impl Core {
    /// [`submit_review`](Self::submit_review), then, only once it
    /// succeeded, the scripts of `script_ids` (what the user ticked in the
    /// submit dialog) in the background. A failed or refused submit runs
    /// nothing. Ticked ids the settings lack, or whose script is not set
    /// for this verdict or repository, are reported as skipped. Returns
    /// as soon as the review is submitted: the scripts report through
    /// [`on_scripts_done`](Self::on_scripts_done).
    ///
    /// Blocking (network, `gh`): call it from `spawn_blocking`.
    pub fn submit_review_with_scripts(
        &self,
        id: ReviewId,
        verdict: ReviewVerdict,
        seen_head: &str,
        summary: Option<&str>,
        script_ids: &[String],
    ) -> Result<ReviewRecord, PostError> {
        let record = self.submit_review(id, verdict, seen_head, summary)?;
        if script_ids.is_empty() {
            return Ok(record);
        }
        let settings = self.settings();
        let planned = plan(
            &settings.submit_scripts,
            script_ids,
            verdict,
            &repo_key(&record.review),
        );
        if planned.is_empty() {
            return Ok(record);
        }
        let clone_dir = settings
            .clone_path(&record.review)
            .filter(|p| p.is_dir())
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let context =
            ScriptContext::new(&record, verdict, seen_head, summary, clone_dir.as_deref());
        self.scripts.enqueue(runtime::Batch {
            review_id: record.id,
            context,
            clone_dir,
            planned,
        });
        Ok(record)
    }

    /// Register the `scripts:done` hook: called (on the scripts' thread)
    /// once the scripts of one submit have all ended.
    pub fn on_scripts_done(&self, callback: impl Fn(&ScriptsDone) + Send + Sync + 'static) {
        self.scripts.set_callback(Arc::new(callback));
    }

    /// Wait until no submit script runs or waits to (tests; `false` when
    /// `timeout` passed first).
    pub fn wait_scripts_idle(&self, timeout: Duration) -> bool {
        self.scripts.wait_idle(timeout)
    }
}
