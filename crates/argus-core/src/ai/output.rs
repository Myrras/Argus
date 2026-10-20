//! The agent's answer: find the one JSON object in it, validate it strictly
//! against the output contract, and drop what does not belong to the review.
//!
//! ```json
//! {"findings":[{"path":"","line":0,"side":"new|old","severity":"high|medium|low","title":"","body":"","relatedThreadIds":[]}],
//!  "checklist":[{"text":"","relatedThreadIds":[]}],
//!  "verdicts":[{"threadId":"","label":"addressed|partly|untouched","reason":""}]}
//! ```
//!
//! `relatedThreadIds` (the existing threads or posted findings a point
//! overlaps, see `super::dedupe`) is asked for on every finding and item
//! but taken as empty when left out: a missing list is no reason to fail a
//! run.
//!
//! Validation is hand-written over [`serde_json::Value`] rather than a serde
//! derive so every error is Argus' own text (a serde message quotes the
//! model's output back) and names the exact field, which is what the repair
//! retry tells the agent.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::dedupe::{RaisedRef, Repeat};
use super::model::{Severity, VerdictLabel};
use crate::ids::ThreadId;
use crate::provider::Side;

/// One finding as the agent wrote it (validated, not yet filtered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFinding {
    pub path: String,
    pub line: u32,
    pub side: Side,
    pub severity: Severity,
    pub title: String,
    pub body: String,
    /// The ids of the threads (or posted findings) the agent said this
    /// overlaps; after [`filter`], only ids the prompt listed, canonical.
    pub related: Vec<String>,
    /// Set by `super::dedupe::classify` when the review already makes
    /// this point.
    pub repeat: Option<Repeat>,
    /// Set by `super::dedupe::classify`: the points of [`Self::related`]
    /// as raised points (shown as "related to …" when not a repeat).
    pub related_to: Vec<RaisedRef>,
}

/// One "look here yourself" item as the agent wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawCheck {
    pub text: String,
    /// As [`RawFinding::related`].
    pub related: Vec<String>,
    /// As [`RawFinding::repeat`].
    pub repeat: Option<Repeat>,
}

/// Tests write items as plain text (nothing related) and compare by text.
#[cfg(test)]
impl From<&str> for RawCheck {
    fn from(text: &str) -> Self {
        RawCheck {
            text: text.into(),
            related: Vec::new(),
            repeat: None,
        }
    }
}

#[cfg(test)]
impl PartialEq<&str> for RawCheck {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawVerdict {
    /// As written: may not parse, or not be a thread of this review.
    pub thread_id: String,
    pub label: VerdictLabel,
    pub reason: String,
}

/// A schema-valid answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentOutput {
    pub findings: Vec<RawFinding>,
    pub checklist: Vec<RawCheck>,
    pub verdicts: Vec<RawVerdict>,
}

/// What reaches the store: findings on files of the diff, verdicts on known
/// threads, and how many of each were dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RunOutput {
    /// Repeats of the review included, marked ([`RawFinding::repeat`]).
    pub findings: Vec<RawFinding>,
    /// Repeats of the review included, marked ([`RawCheck::repeat`]).
    pub checklist: Vec<RawCheck>,
    pub verdicts: Vec<(ThreadId, VerdictLabel, String)>,
    pub dropped_findings: u32,
    pub dropped_verdicts: u32,
    /// Findings marked as repeats of the review (an existing thread or a
    /// posted finding; see `super::dedupe`): stored, listed apart.
    pub dropped_duplicates: u32,
    /// Checklist items marked as repeats of the review: stored, listed
    /// apart.
    pub skipped_checklist: u32,
    /// The diff the agent was given: its right side (where new-side lines
    /// are numbered) and left side (old-side lines; the watermark for a
    /// run over the commits since the last review).
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
}

/// Parse `text` into a valid answer, or say what is wrong with it.
pub(crate) fn parse(text: &str) -> Result<AgentOutput, String> {
    validate(&extract_json(text)?)
}

/// The JSON object in an answer: the whole text, else the content of a
/// ```` ```json ```` (or bare ```` ``` ````) fence, else the first `{` from
/// which a complete object parses (prose before or after is ignored).
pub(crate) fn extract_json(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }
    if let Some(inner) = fenced(trimmed) {
        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(inner.trim()) {
            return Ok(v);
        }
    }
    for (i, _) in trimmed.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&trimmed[i..]).into_iter::<Value>();
        if let Some(Ok(v @ Value::Object(_))) = stream.next() {
            return Ok(v);
        }
    }
    Err("the answer holds no JSON object".into())
}

/// The body of the first fenced code block, preferring a `json` one.
fn fenced(text: &str) -> Option<&str> {
    let start = text
        .find("```json")
        .map(|i| i + "```json".len())
        .or_else(|| {
            let i = text.find("```")?;
            // Skip an info string such as ```JSON or ```js.
            let rest = &text[i + 3..];
            Some(i + 3 + rest.find('\n').unwrap_or(0))
        })?;
    let body = &text[start..];
    let end = body.find("```")?;
    Some(&body[..end])
}

/// Check `v` against the contract: exactly the three keys, every item with
/// exactly its fields, the right types and enum values.
pub(crate) fn validate(v: &Value) -> Result<AgentOutput, String> {
    let top = v.as_object().ok_or("the answer must be one JSON object")?;
    only_keys(
        top,
        &["findings", "checklist", "verdicts"],
        "the top-level object",
    )?;
    let mut out = AgentOutput::default();
    for (i, item) in array(top, "findings")?.iter().enumerate() {
        let at = format!("findings[{i}]");
        let o = object(item, &at)?;
        only_keys_with(
            o,
            &["path", "line", "side", "severity", "title", "body"],
            &[RELATED],
            &at,
        )?;
        let line = o
            .get("line")
            .and_then(Value::as_u64)
            .filter(|l| *l >= 1)
            .and_then(|l| u32::try_from(l).ok())
            .ok_or_else(|| format!("{at}.line must be a line number of 1 or more"))?;
        let side = match string(o, "side", &at)? {
            "new" => Side::New,
            "old" => Side::Old,
            _ => return Err(format!("{at}.side must be \"new\" or \"old\"")),
        };
        let severity = Severity::parse(string(o, "severity", &at)?)
            .ok_or_else(|| format!("{at}.severity must be \"high\", \"medium\" or \"low\""))?;
        out.findings.push(RawFinding {
            path: non_empty(o, "path", &at)?.to_string(),
            line,
            side,
            severity,
            title: non_empty(o, "title", &at)?.to_string(),
            body: string(o, "body", &at)?.trim().to_string(),
            related: related(o, &at)?,
            repeat: None,
            related_to: Vec::new(),
        });
    }
    for (i, item) in array(top, "checklist")?.iter().enumerate() {
        let at = format!("checklist[{i}]");
        let o = object(item, &at)?;
        only_keys_with(o, &["text"], &[RELATED], &at)?;
        out.checklist.push(RawCheck {
            text: non_empty(o, "text", &at)?.to_string(),
            related: related(o, &at)?,
            repeat: None,
        });
    }
    for (i, item) in array(top, "verdicts")?.iter().enumerate() {
        let at = format!("verdicts[{i}]");
        let o = object(item, &at)?;
        only_keys(o, &["threadId", "label", "reason"], &at)?;
        let label = VerdictLabel::parse(string(o, "label", &at)?).ok_or_else(|| {
            format!("{at}.label must be \"addressed\", \"partly\" or \"untouched\"")
        })?;
        out.verdicts.push(RawVerdict {
            thread_id: non_empty(o, "threadId", &at)?.to_string(),
            label,
            reason: string(o, "reason", &at)?.trim().to_string(),
        });
    }
    Ok(out)
}

/// The key of the related-thread list on findings and checklist items.
const RELATED: &str = "relatedThreadIds";

/// `relatedThreadIds` of `o`: an array whose strings are kept (trimmed,
/// empty ones left out); absent is empty. Entries that are not strings are
/// dropped: the list is only a hint, so a stray number must not fail the
/// whole answer (unknown ids are dropped the same way by [`filter`]).
fn related(o: &Map<String, Value>, at: &str) -> Result<Vec<String>, String> {
    let Some(v) = o.get(RELATED) else {
        return Ok(Vec::new());
    };
    Ok(v.as_array()
        .ok_or_else(|| format!("{at}.{RELATED} must be an array of thread id strings"))?
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn only_keys(o: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    only_keys_with(o, allowed, &[], at)
}

/// Exactly the `required` keys, plus any of `optional`.
fn only_keys_with(
    o: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
    at: &str,
) -> Result<(), String> {
    let allowed: Vec<&str> = required.iter().chain(optional).copied().collect();
    if let Some(k) = o.keys().find(|k| !allowed.contains(&k.as_str())) {
        // The key is the model's text: name it only when it is short and
        // plain, so a junk key cannot smuggle prose into the error.
        let shown = if k.len() <= 32 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            format!("\"{k}\"")
        } else {
            "a key".to_string()
        };
        return Err(format!(
            "{at} has {shown} that is not in the contract (allowed: {})",
            allowed.join(", ")
        ));
    }
    for k in required {
        if !o.contains_key(*k) {
            return Err(format!("{at} is missing \"{k}\""));
        }
    }
    Ok(())
}

fn array<'a>(o: &'a Map<String, Value>, key: &str) -> Result<&'a Vec<Value>, String> {
    o.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("\"{key}\" must be an array"))
}

fn object<'a>(v: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("{at} must be an object"))
}

fn string<'a>(o: &'a Map<String, Value>, key: &str, at: &str) -> Result<&'a str, String> {
    o.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{at}.{key} must be a string"))
}

fn non_empty<'a>(o: &'a Map<String, Value>, key: &str, at: &str) -> Result<&'a str, String> {
    let s = string(o, key, at)?.trim();
    if s.is_empty() {
        return Err(format!("{at}.{key} must not be empty"));
    }
    Ok(s)
}

/// A path as the agent may spell it (`./src/a.rs`, `/src/a.rs`, `b/src/a.rs`)
/// matched against the diff's own paths.
fn match_path<'a>(path: &str, allowed: &'a HashSet<String>) -> Option<&'a String> {
    let p = path.trim().trim_start_matches("./").trim_start_matches('/');
    allowed.get(p).or_else(|| {
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .and_then(|q| allowed.get(q))
    })
}

/// The ids of `ids` that are in `known` (the threads and posted findings
/// the prompt listed), canonical (lower-case UUID), each once. An unknown
/// id is dropped from the list, never the point that named it.
fn known_ids(ids: Vec<String>, known: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if let Ok(u) = uuid::Uuid::parse_str(id.trim()) {
            let id = u.to_string();
            if known.contains(&id) && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// Keep findings on `paths` (the files of the diff the agent was given) and
/// verdicts on `threads` (the threads it was asked about; one each, the
/// first wins), counting what was dropped; keep only `related` ids (those
/// the prompt listed) in the related lists.
pub(crate) fn filter(
    out: AgentOutput,
    paths: &HashSet<String>,
    threads: &HashSet<ThreadId>,
    related: &HashSet<String>,
) -> RunOutput {
    let mut run = RunOutput {
        checklist: out
            .checklist
            .into_iter()
            .map(|c| RawCheck {
                related: known_ids(c.related, related),
                ..c
            })
            .collect(),
        ..RunOutput::default()
    };
    for mut f in out.findings {
        match match_path(&f.path, paths) {
            Some(p) => {
                f.path = p.clone();
                f.related = known_ids(std::mem::take(&mut f.related), related);
                run.findings.push(f);
            }
            None => run.dropped_findings += 1,
        }
    }
    let mut seen = HashSet::new();
    for v in out.verdicts {
        match v.thread_id.trim().parse::<ThreadId>() {
            Ok(id) if threads.contains(&id) && seen.insert(id) => {
                run.verdicts.push((id, v.label, v.reason))
            }
            _ => run.dropped_verdicts += 1,
        }
    }
    run
}

/// A token with a fixed prefix: what may follow it, at least how much, and
/// whether it is distinctive enough to match inside a word.
struct TokenShape {
    prefix: &'static str,
    tail: fn(char) -> bool,
    min_tail: usize,
    /// Matched even right after a token character (`%3Aghp_…`, `x_glpat-…`).
    /// Shapes with a short or common prefix need a boundary before them:
    /// a `%XX` escape or a non-token character.
    anywhere: bool,
    /// The tail must hold a digit (random keys do; words do not).
    digit: bool,
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// GitLab tokens may carry a `.`-separated routing part (`glpat-…​.01.…`).
fn gitlab_char(c: char) -> bool {
    token_char(c) || c == '.'
}

const fn shape(prefix: &'static str, tail: fn(char) -> bool, min_tail: usize) -> TokenShape {
    TokenShape {
        prefix,
        tail,
        min_tail,
        anywhere: true,
        digit: false,
    }
}

const fn bounded(prefix: &'static str, tail: fn(char) -> bool, min_tail: usize) -> TokenShape {
    TokenShape {
        prefix,
        tail,
        min_tail,
        anywhere: false,
        digit: true,
    }
}

/// Token shapes: GitHub (fine-grained, classic, OAuth, user, server,
/// refresh), GitLab (personal, deploy, runner, pipeline trigger, CI job,
/// OAuth app secret, feed), npm, Anthropic, OpenAI-style `sk-`, Slack.
/// Longer prefixes first (`sk-ant-` before `sk-`).
const TOKEN_SHAPES: &[TokenShape] = &[
    shape("github_pat_", token_char, 22),
    shape("ghp_", alnum, 30),
    shape("gho_", alnum, 30),
    shape("ghu_", alnum, 30),
    shape("ghs_", alnum, 30),
    shape("ghr_", alnum, 30),
    shape("glpat-", gitlab_char, 16),
    shape("gldt-", gitlab_char, 16),
    shape("glrt-", gitlab_char, 16),
    shape("glptt-", gitlab_char, 16),
    shape("glcbt-", gitlab_char, 16),
    shape("gloas-", gitlab_char, 16),
    shape("glft-", gitlab_char, 16),
    shape("npm_", alnum, 36),
    shape("sk-ant-", token_char, 20),
    bounded("sk-", token_char, 20),
    bounded("xoxb-", token_char, 10),
    bounded("xoxp-", token_char, 10),
    bounded("xoxa-", token_char, 10),
    bounded("xoxe-", token_char, 10),
    bounded("xapp-", token_char, 10),
];
/// The shortest known secret that is replaced wherever it occurs.
const MIN_SECRET: usize = 8;
const REDACTED: &str = "[redacted]";

/// Whether a token may start after `before` (the text so far): at its
/// start, after a non-token character, or right after a `%XX` escape.
fn at_boundary(before: &str) -> bool {
    let Some(last) = before.chars().last() else {
        return true;
    };
    if !token_char(last) {
        return true;
    }
    let b = before.as_bytes();
    b.len() >= 3
        && b[b.len() - 3] == b'%'
        && b[b.len() - 2].is_ascii_hexdigit()
        && b[b.len() - 1].is_ascii_hexdigit()
}

/// Length of the token at the start of `rest`, if one starts there.
fn token_at(rest: &str, boundary: bool) -> Option<usize> {
    for s in TOKEN_SHAPES {
        if !(boundary || s.anywhere) {
            continue;
        }
        let Some(tail) = rest.strip_prefix(s.prefix) else {
            continue;
        };
        let n = tail.chars().take_while(|c| (s.tail)(*c)).count();
        let tail = &tail[..n];
        if n >= s.min_tail && (!s.digit || tail.chars().any(|c| c.is_ascii_digit())) {
            return Some(s.prefix.len() + n);
        }
    }
    if boundary {
        // AWS access key id: `AKIA` + 16 upper-case letters or digits.
        if let Some(tail) = rest.strip_prefix("AKIA") {
            let n = tail
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                .count();
            if n == 16 {
                return Some(4 + n);
            }
        }
    }
    None
}

/// Length of a URL's credentials at the start of `rest` (just after
/// `://`): `user:password@` or a long bare user (a token as the user
/// name, `https://<token>@host`), up to the `@`. `git@` and the like stay.
fn userinfo_at(rest: &str) -> Option<usize> {
    let end =
        rest.find(|c: char| c == '@' || c == '/' || c == '?' || c == '#' || c.is_whitespace())?;
    if !rest[end..].starts_with('@') || end == 0 {
        return None;
    }
    let info = &rest[..end];
    (info.contains(':') || info.len() >= 20).then_some(end)
}

/// Length of HTTP Basic credentials at the start of `rest` (just after
/// `Basic `): base64 that decodes to printable `user:password`, so prose
/// like "Basic authentication" stays.
fn basic_auth_at(rest: &str) -> Option<usize> {
    use base64::Engine as _;
    let n = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
        .count();
    if n < 8 || n % 4 != 0 {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&rest[..n])
        .ok()?;
    let printable = decoded.iter().all(|b| (0x20..0x7f).contains(b));
    (printable && decoded.contains(&b':')).then_some(n)
}

/// `text` with every `secrets` occurrence and every credential-shaped
/// string replaced: the [`TOKEN_SHAPES`], AWS access key ids, URL
/// credentials, HTTP Basic credentials, and PEM private key blocks.
pub(crate) fn redact_text(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets.iter().filter(|s| s.len() >= MIN_SECRET) {
        out = out.replace(s.as_str(), REDACTED);
    }
    out = redact_pem(&out);
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while !rest.is_empty() {
        if let Some(n) = token_at(rest, at_boundary(&result)) {
            result.push_str(REDACTED);
            rest = &rest[n..];
            continue;
        }
        let lead = if let Some(after) = rest.strip_prefix("://") {
            userinfo_at(after).map(|n| (3, n))
        } else if rest
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("basic "))
            && at_boundary(&result)
        {
            basic_auth_at(&rest[6..]).map(|n| (6, n))
        } else {
            None
        };
        if let Some((keep, n)) = lead {
            result.push_str(&rest[..keep]);
            result.push_str(REDACTED);
            rest = &rest[keep + n..];
            continue;
        }
        let c = rest.chars().next().expect("not empty");
        result.push(c);
        rest = &rest[c.len_utf8()..];
    }
    result
}

/// PEM private key blocks (`-----BEGIN … PRIVATE KEY-----` to its `END`
/// line, or to the end of the text when the block is cut off).
fn redact_pem(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(BEGIN) {
        let after = &rest[i + BEGIN.len()..];
        let Some(label_end) = after.find("-----") else {
            break;
        };
        let label = &after[..label_end];
        if !label.ends_with("PRIVATE KEY") || label.contains('\n') {
            out.push_str(&rest[..i + BEGIN.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..i]);
        out.push_str(REDACTED);
        let end_marker = format!("-----END {label}-----");
        rest = match after.find(&end_marker) {
            Some(j) => &after[j + end_marker.len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Redact every text of a run's output that reaches the store (and from
/// there, one click away, the platform): finding titles and bodies,
/// checklist items, verdict reasons. The agent can read files beside the
/// diff; whatever secret it quoted stays out.
pub(crate) fn redact(out: &mut RunOutput, secrets: &[String]) {
    for f in &mut out.findings {
        f.title = redact_text(&f.title, secrets);
        f.body = redact_text(&f.body, secrets);
    }
    for item in &mut out.checklist {
        item.text = redact_text(&item.text, secrets);
    }
    for (_, _, reason) in &mut out.verdicts {
        *reason = redact_text(reason, secrets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shapes_and_known_secrets_are_redacted() {
        // Fictitious tokens of the right shapes.
        let gh = format!("ghp_{}", "A1b2C3d4E5".repeat(4));
        let pat = format!("github_pat_{}", "x9".repeat(20));
        let glpat = "glpat-FixtureFixture1234";
        let aws = "AKIAFIXTUREFIXTURE12";
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk=\n-----END OPENSSH PRIVATE KEY-----";
        let text = format!(
            "Found {gh} in ~/.config/gh/hosts.yml, ({pat}), token={glpat}; aws {aws}.\n{pem}\nand fixture-secret-value here"
        );
        let out = redact_text(&text, &["fixture-secret-value".into()]);
        for leaked in [
            gh.as_str(),
            pat.as_str(),
            glpat,
            aws,
            "b3BlbnNzaC1rZXk",
            "fixture-secret-value",
        ] {
            assert!(!out.contains(leaked), "{leaked} in {out}");
        }
        assert_eq!(out.matches(REDACTED).count(), 6, "{out}");
        assert!(out.starts_with(
            "Found [redacted] in ~/.config/gh/hosts.yml, ([redacted]), token=[redacted];"
        ));
        // Prose about the formats, and lookalikes, stay.
        for kept in [
            "Tokens start with ghp_ or glpat-.",
            "nightly_ghp_build",
            "AKIA is the prefix",
            "-----BEGIN PUBLIC KEY-----\nabc\n-----END PUBLIC KEY-----",
        ] {
            assert_eq!(redact_text(kept, &[]), kept);
        }
        // A cut-off key block is redacted to the end.
        assert_eq!(
            redact_text("x -----BEGIN RSA PRIVATE KEY-----\nMIIE", &[]),
            "x [redacted]"
        );
    }

    #[test]
    fn every_credential_shape_is_redacted() {
        use base64::Engine as _;
        // Fictitious values of each shape, each with the text around it
        // that must stay.
        let basic = base64::engine::general_purpose::STANDARD.encode("fixture-user:fixture-pass");
        let cases = [
            (
                "https://oauth2:FixtureSecret1@gitlab.example.test/acme/widgets.git".to_string(),
                "https://[redacted]@gitlab.example.test/acme/widgets.git",
            ),
            (
                format!("remote https://{}@github.example.test/x", "Fx9".repeat(10)),
                "remote https://[redacted]@github.example.test/x",
            ),
            (
                format!("key sk-ant-api03-{}", "Fx9_".repeat(10)),
                "key [redacted]",
            ),
            (
                format!("OPENAI=sk-proj-{}", "Fx9".repeat(10)),
                "OPENAI=[redacted]",
            ),
            (
                format!("npm {}", format_args!("npm_{}", "Fx9".repeat(12))),
                "npm [redacted]",
            ),
            (format!("glrt-{}", "Fx9".repeat(7)), "[redacted]"),
            (format!("glptt-{}", "Fx9".repeat(7)), "[redacted]"),
            (format!("glcbt-64_{}", "Fx9".repeat(7)), "[redacted]"),
            (format!("gloas-{}", "Fx9".repeat(7)), "[redacted]"),
            (format!("glft-{}", "Fx9".repeat(7)), "[redacted]"),
            ("slack xoxa-2-1234567890-fixture".into(), "slack [redacted]"),
            ("xoxe-1-fixture12345".into(), "[redacted]"),
            ("app xapp-1-A0FIXTURE-123".into(), "app [redacted]"),
            (
                format!("Authorization: Basic {basic}\n"),
                "Authorization: Basic [redacted]\n",
            ),
            // After a token character: a URL-encoded `:` or a joined name.
            (
                format!("u=x%3Aghp_{}&y", "Fx9".repeat(12)),
                "u=x%3A[redacted]&y",
            ),
            (
                format!("TOKEN_glpat-{}", "Fx9".repeat(7)),
                "TOKEN_[redacted]",
            ),
            (format!("a%40sk-{}", "Fx9".repeat(8)), "a%40[redacted]"),
        ];
        for (text, want) in &cases {
            assert_eq!(redact_text(text, &[]), *want, "{text}");
        }
    }

    #[test]
    fn ordinary_code_and_prose_are_not_redacted() {
        for kept in [
            "let task-sk-1 = 2;",
            "the sk-learn-compatible-estimator-wrapper type",
            "task_sk-12345678901234567890 is an id",
            "Basic authentication is off; Basic auth too.",
            "Basic dGVzdA== decodes to no pair",
            "ssh://git@github.example.test/acme/widgets.git",
            "https://gitlab.example.test/acme/widgets/-/merge_requests/1",
            "email someone@example.test",
            "run_npm_install_script_for_the_workspace",
            "xapp-service-config-loader",
            "myxoxb-handler",
            "a ghp_ token, a glrt- token, an npm_ token",
            "ghp_short1",
        ] {
            assert_eq!(redact_text(kept, &[]), kept);
        }
    }

    #[test]
    fn a_run_output_is_redacted_everywhere_it_has_text() {
        let tok = format!("gho_{}", "Z".repeat(36));
        let mut out = RunOutput {
            findings: vec![RawFinding {
                path: "src/a.rs".into(),
                line: 1,
                side: Side::New,
                severity: Severity::High,
                title: format!("Leaks {tok}"),
                body: format!("The token {tok} is printed."),
                related: Vec::new(),
                repeat: None,
                related_to: Vec::new(),
            }],
            checklist: vec![format!("Rotate {tok}").as_str().into()],
            verdicts: vec![(
                ThreadId::from_key(crate::ids::ReviewId::from_key("r"), "1"),
                VerdictLabel::Partly,
                format!("still uses {tok}"),
            )],
            ..RunOutput::default()
        };
        redact(&mut out, &[]);
        let all = format!("{out:?}");
        assert!(!all.contains(&tok), "{all}");
        assert_eq!(out.findings[0].title, "Leaks [redacted]");
        assert_eq!(out.checklist[0], "Rotate [redacted]");
    }

    const VALID: &str = r#"{"findings":[{"path":"src/a.rs","line":3,"side":"new","severity":"high","title":"Off by one","body":"The loop skips the last item."}],"checklist":[{"text":"Is the cache meant to be per user?"}],"verdicts":[]}"#;

    #[test]
    fn parses_plain_fenced_and_prose_wrapped_answers() {
        let plain = parse(VALID).unwrap();
        assert_eq!(plain.findings.len(), 1);
        assert_eq!(plain.findings[0].side, Side::New);
        assert_eq!(plain.findings[0].severity, Severity::High);
        assert_eq!(plain.checklist, vec!["Is the cache meant to be per user?"]);

        let fenced = format!("Here is my review:\n\n```json\n{VALID}\n```\nThanks!");
        assert_eq!(parse(&fenced).unwrap(), plain);
        let bare_fence = format!("```\n{VALID}\n```");
        assert_eq!(parse(&bare_fence).unwrap(), plain);
        let prose = format!("I looked at the {{diff}} carefully. {VALID} That is all.");
        assert_eq!(parse(&prose).unwrap(), plain);
    }

    #[test]
    fn malformed_answers_are_rejected() {
        assert!(extract_json("no json here").is_err());
        assert!(extract_json("{\"findings\": [").is_err());
        assert!(extract_json("[1, 2]").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn schema_violations_name_the_field() {
        let cases = [
            (r#"{"findings":[],"checklist":[]}"#, "missing \"verdicts\""),
            (
                r#"{"findings":[],"checklist":[],"verdicts":[],"summary":"x"}"#,
                "\"summary\" that is not in the contract",
            ),
            (
                r#"{"findings":{},"checklist":[],"verdicts":[]}"#,
                "\"findings\" must be an array",
            ),
            (
                r#"{"findings":[{"path":"a","line":0,"side":"new","severity":"low","title":"t","body":""}],"checklist":[],"verdicts":[]}"#,
                "findings[0].line",
            ),
            (
                r#"{"findings":[{"path":"a","line":"3","side":"new","severity":"low","title":"t","body":""}],"checklist":[],"verdicts":[]}"#,
                "findings[0].line",
            ),
            (
                r#"{"findings":[{"path":"a","line":1,"side":"right","severity":"low","title":"t","body":""}],"checklist":[],"verdicts":[]}"#,
                "findings[0].side",
            ),
            (
                r#"{"findings":[{"path":"a","line":1,"side":"new","severity":"critical","title":"t","body":""}],"checklist":[],"verdicts":[]}"#,
                "findings[0].severity",
            ),
            (
                r#"{"findings":[{"path":"","line":1,"side":"new","severity":"low","title":"t","body":""}],"checklist":[],"verdicts":[]}"#,
                "findings[0].path must not be empty",
            ),
            (
                r#"{"findings":[{"path":"a","line":1,"side":"new","severity":"low","title":"t"}],"checklist":[],"verdicts":[]}"#,
                "findings[0] is missing \"body\"",
            ),
            (
                r#"{"findings":[],"checklist":["plain string"],"verdicts":[]}"#,
                "checklist[0] must be an object",
            ),
            (
                r#"{"findings":[],"checklist":[{"text":"  "}],"verdicts":[]}"#,
                "checklist[0].text must not be empty",
            ),
            (
                r#"{"findings":[],"checklist":[],"verdicts":[{"threadId":"x","label":"done","reason":""}]}"#,
                "verdicts[0].label",
            ),
            (
                r#"{"findings":[],"checklist":[],"verdicts":[{"threadId":"x","label":"partly"}]}"#,
                "verdicts[0] is missing \"reason\"",
            ),
        ];
        for (input, expect) in cases {
            let err = parse(input).unwrap_err();
            assert!(err.contains(expect), "{input}: got {err:?}");
        }
    }

    #[test]
    fn related_thread_ids_are_read_checked_and_optional() {
        let known = "6f1c2a8e-0000-4000-8000-000000000001";
        let answer = format!(
            r#"{{"findings":[
              {{"path":"src/a.rs","line":3,"side":"new","severity":"high","title":"t","body":"b","relatedThreadIds":["{}"," ", "not-a-uuid","6f1c2a8e-0000-4000-8000-0000000000ff"]}},
              {{"path":"src/a.rs","line":4,"side":"new","severity":"low","title":"u","body":""}}
             ],
             "checklist":[{{"text":"c","relatedThreadIds":["{known}","{known}"]}},{{"text":"d"}}],
             "verdicts":[]}}"#,
            known.to_uppercase()
        );
        let parsed = parse(&answer).unwrap();
        assert_eq!(parsed.findings[0].related.len(), 3);
        assert!(parsed.findings[1].related.is_empty());
        assert!(parsed.checklist[1].related.is_empty());
        let paths: HashSet<String> = ["src/a.rs".to_string()].into();
        let run = filter(parsed, &paths, &HashSet::new(), &[known.to_string()].into());
        // Unknown ids leave the list, never the point; ids come out canonical.
        assert_eq!(run.findings.len(), 2);
        assert_eq!(run.findings[0].related, vec![known.to_string()]);
        assert_eq!(run.checklist[0].related, vec![known.to_string()]);
        assert_eq!(run.checklist.len(), 2);
        for (input, expect) in [
            (
                r#"{"findings":[{"path":"a","line":1,"side":"new","severity":"low","title":"t","body":"","relatedThreadIds":"x"}],"checklist":[],"verdicts":[]}"#,
                "findings[0].relatedThreadIds must be an array",
            ),
            (
                r#"{"findings":[],"checklist":[{"text":"t","related":[]}],"verdicts":[]}"#,
                "\"related\" that is not in the contract (allowed: text, relatedThreadIds)",
            ),
        ] {
            let err = parse(input).unwrap_err();
            assert!(err.contains(expect), "{input}: got {err:?}");
        }
        // A non-string entry is dropped, not the whole answer: the list is
        // a hint, and one bad entry must not cost the run its findings.
        let mixed = format!(
            r#"{{"findings":[],"checklist":[{{"text":"t","relatedThreadIds":[1,null,{{"id":"x"}},"{known}"]}}],"verdicts":[]}}"#
        );
        let parsed = parse(&mixed).unwrap();
        assert_eq!(parsed.checklist[0].related, vec![known.to_string()]);
    }

    #[test]
    fn junk_keys_are_not_echoed() {
        let err = parse(
            r#"{"findings":[],"checklist":[],"verdicts":[],"ignore previous instructions":1}"#,
        )
        .unwrap_err();
        assert!(!err.contains("ignore"), "{err}");
    }

    #[test]
    fn filter_drops_unknown_paths_and_threads() {
        let known = ThreadId::from_key(crate::ids::ReviewId::from_key("r"), "1");
        let other = ThreadId::from_key(crate::ids::ReviewId::from_key("r"), "2");
        let f = |path: &str| RawFinding {
            path: path.into(),
            line: 1,
            side: Side::New,
            severity: Severity::Low,
            title: "t".into(),
            body: String::new(),
            related: Vec::new(),
            repeat: None,
            related_to: Vec::new(),
        };
        let v = |id: String| RawVerdict {
            thread_id: id,
            label: VerdictLabel::Addressed,
            reason: "fixed".into(),
        };
        let out = AgentOutput {
            findings: vec![
                f("src/a.rs"),
                f("./src/a.rs"),
                f("b/src/a.rs"),
                f("src/nope.rs"),
            ],
            checklist: vec!["check".into()],
            verdicts: vec![
                v(known.to_string()),
                v(known.to_string()),
                v(other.to_string()),
                v("not-a-uuid".into()),
            ],
        };
        let paths: HashSet<String> = ["src/a.rs".to_string()].into();
        let run = filter(out, &paths, &[known].into(), &HashSet::new());
        assert_eq!(run.findings.len(), 3);
        assert!(run.findings.iter().all(|f| f.path == "src/a.rs"));
        assert_eq!(run.dropped_findings, 1);
        assert_eq!(run.verdicts.len(), 1);
        assert_eq!(run.verdicts[0].0, known);
        assert_eq!(run.dropped_verdicts, 3);
        assert_eq!(run.checklist, vec!["check"]);
    }
}
