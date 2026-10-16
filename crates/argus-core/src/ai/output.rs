//! The agent's answer: find the one JSON object in it, validate it strictly
//! against the output contract, and drop what does not belong to the review.
//!
//! ```json
//! {"findings":[{"path":"","line":0,"side":"new|old","severity":"high|medium|low","title":"","body":""}],
//!  "checklist":[{"text":""}],
//!  "verdicts":[{"threadId":"","label":"addressed|partly|untouched","reason":""}]}
//! ```
//!
//! Validation is hand-written over [`serde_json::Value`] rather than a serde
//! derive so every error is Argus' own text (a serde message quotes the
//! model's output back) and names the exact field, which is what the repair
//! retry tells the agent.

use std::collections::HashSet;

use serde_json::{Map, Value};

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
    pub checklist: Vec<String>,
    pub verdicts: Vec<RawVerdict>,
}

/// What reaches the store: findings on files of the diff, verdicts on known
/// threads, and how many of each were dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RunOutput {
    pub findings: Vec<RawFinding>,
    pub checklist: Vec<String>,
    pub verdicts: Vec<(ThreadId, VerdictLabel, String)>,
    pub dropped_findings: u32,
    pub dropped_verdicts: u32,
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
        only_keys(
            o,
            &["path", "line", "side", "severity", "title", "body"],
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
        });
    }
    for (i, item) in array(top, "checklist")?.iter().enumerate() {
        let at = format!("checklist[{i}]");
        let o = object(item, &at)?;
        only_keys(o, &["text"], &at)?;
        out.checklist.push(non_empty(o, "text", &at)?.to_string());
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

fn only_keys(o: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
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
    for k in allowed {
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

/// Keep findings on `paths` (the files of the diff the agent was given) and
/// verdicts on `threads` (the threads it was asked about; one each, the
/// first wins), counting what was dropped.
pub(crate) fn filter(
    out: AgentOutput,
    paths: &HashSet<String>,
    threads: &HashSet<ThreadId>,
) -> RunOutput {
    let mut run = RunOutput {
        checklist: out.checklist,
        ..RunOutput::default()
    };
    for mut f in out.findings {
        match match_path(&f.path, paths) {
            Some(p) => {
                f.path = p.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let run = filter(out, &paths, &[known].into());
        assert_eq!(run.findings.len(), 3);
        assert!(run.findings.iter().all(|f| f.path == "src/a.rs"));
        assert_eq!(run.dropped_findings, 1);
        assert_eq!(run.verdicts.len(), 1);
        assert_eq!(run.verdicts[0].0, known);
        assert_eq!(run.dropped_verdicts, 3);
        assert_eq!(run.checklist, vec!["check"]);
    }
}
