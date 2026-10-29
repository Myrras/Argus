/**
 * Submit scripts, the pure rules: which scripts the submit dialog offers
 * (mirroring the core's `SubmitScript::matches_verdict` / `matches_repo`),
 * what a new one looks like, how a run's results read as toasts and in the
 * Scripts tab, and how "Write with AI" drafts read.
 */

import type {
  DraftStage,
  GeneratedScript,
  ReviewRecord,
  ReviewVerdict,
  ScriptDraft,
  ScriptDraftProgress,
  ScriptDraftProgressEvent,
  ScriptResult,
  ScriptStatus,
  ScriptsDone,
  SubmitScript,
} from "../ipc/types";

/** The bounds of a script's timeout (the core clamps to them too). */
export const TIMEOUT_MIN = 5;
export const TIMEOUT_MAX = 600;
export const TIMEOUT_DEFAULT = 60;

/** Every verdict, in the order the Scripts tab lists them. */
export const ALL_VERDICTS: readonly ReviewVerdict[] = [
  "approve",
  "requestChanges",
  "comment",
];

/** A verdict as a script's checkbox names it. */
export const VERDICT_CHOICE: Record<ReviewVerdict, string> = {
  approve: "Approve",
  requestChanges: "Request changes",
  comment: "Comment",
};

/** The `ARGUS_*` variables, for the Scripts tab's table. */
export const SCRIPT_ENV: readonly [string, string][] = [
  ["ARGUS_PROVIDER", "gitlab or github"],
  ["ARGUS_HOST", "the platform's host, e.g. gitlab.example.com"],
  ["ARGUS_REPO", "group/project (GitLab) or owner/repo (GitHub)"],
  ["ARGUS_NUMBER", "the MR iid or PR number"],
  ["ARGUS_URL", "the merge or pull request's page"],
  ["ARGUS_TITLE", "its title"],
  ["ARGUS_VERDICT", "approve, requestChanges or comment"],
  ["ARGUS_HEAD_SHA", "the commit you submitted the review for"],
  ["ARGUS_CLONE", "your local clone's folder (empty without one)"],
];

/** The example in the Scripts tab: ticks the MR description's task list. */
export const EXAMPLE_SCRIPT = `#!/bin/sh
# Ticks every task list item in the MR description.
p="$(printf %s "$ARGUS_REPO" | sed 's#/#%2F#g')"
desc="$(glab api --hostname "$ARGUS_HOST" "projects/$p/merge_requests/$ARGUS_NUMBER" | jq -r .description)"
new="$(printf %s "$desc" | sed 's/^\\([[:space:]]*[-*] \\)\\[ \\]/\\1[x]/')"
glab api --hostname "$ARGUS_HOST" -X PUT "projects/$p/merge_requests/$ARGUS_NUMBER" -f description="$new" >/dev/null
`;

/** A repository key as the core compares it: case and a trailing slash do not count. */
function normRepo(repo: string): string {
  return repo.trim().replace(/\/+$/, "").toLowerCase();
}

/** `host/repo` of a review: the key `clonePaths` and a script's repos use. */
export function repoKey(r: Pick<ReviewRecord, "host" | "repo">): string {
  return `${r.host}/${r.repo}`;
}

/** Whether `script` is offered when submitting `verdict` on `repo` (`host/repo`). */
export function scriptMatches(
  script: SubmitScript,
  verdict: ReviewVerdict,
  repo: string,
): boolean {
  if (!script.verdicts.includes(verdict)) return false;
  if (script.repos.length === 0) return true;
  const want = normRepo(repo);
  return script.repos.some((r) => normRepo(r) === want);
}

/** The scripts the submit dialog offers, in their configured (run) order. */
export function scriptsFor(
  scripts: readonly SubmitScript[],
  verdict: ReviewVerdict | null,
  repo: string,
): SubmitScript[] {
  if (!verdict) return [];
  return scripts.filter((s) => scriptMatches(s, verdict, repo));
}

/**
 * Whether the dialog ticks `script`: the user's own tick when they set one,
 * else its "Ticked by default".
 */
export function isTicked(
  script: SubmitScript,
  ticks: Readonly<Record<string, boolean>>,
): boolean {
  return ticks[script.id] ?? script.defaultOn;
}

/** A fresh script for "Add script". */
export function newScript(
  id: string = crypto.randomUUID(),
  overrides: Partial<SubmitScript> = {},
): SubmitScript {
  return {
    id,
    name: "New script",
    command: "",
    args: [],
    verdicts: ["approve"],
    repos: [],
    defaultOn: false,
    timeoutSecs: TIMEOUT_DEFAULT,
    ...overrides,
  };
}

/** The display name: the user's, else the command's file name. */
export function scriptName(s: Pick<SubmitScript, "name" | "command">): string {
  const name = s.name.trim();
  if (name) return name;
  const file = s.command.trim().split(/[\\/]/).pop() ?? "";
  return file || "Untitled script";
}

/** Arguments as the form shows them: one per line. */
export function argsText(args: readonly string[]): string {
  return args.join("\n");
}

/** Arguments from the form: one per line, blank lines left out, kept as typed. */
export function parseArgs(text: string): string[] {
  return text.split("\n").filter((line) => line.trim() !== "");
}

/** Repositories from the form: one per line or comma separated. */
export function parseRepos(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split(/[\n,]/)) {
    const r = raw.trim().replace(/\/+$/, "");
    if (r && !out.some((k) => normRepo(k) === normRepo(r))) out.push(r);
  }
  return out;
}

/** `scripts` with the one of `id` replaced by `patch` over it. */
export function withScript(
  scripts: readonly SubmitScript[],
  id: string,
  patch: Partial<SubmitScript>,
): SubmitScript[] {
  return scripts.map((s) => (s.id === id ? { ...s, ...patch } : s));
}

/** `scripts` with the one of `id` moved by `delta` places (clamped). */
export function moveScript(
  scripts: readonly SubmitScript[],
  id: string,
  delta: number,
): SubmitScript[] {
  const from = scripts.findIndex((s) => s.id === id);
  if (from < 0) return [...scripts];
  const to = Math.min(scripts.length - 1, Math.max(0, from + delta));
  const out = [...scripts];
  const [item] = out.splice(from, 1);
  out.splice(to, 0, item!);
  return out;
}

/** Whether `command` lies inside `dir` (a script Argus saved). */
export function insideDir(command: string, dir: string | null): boolean {
  if (!dir) return false;
  const base = dir.replace(/[\\/]+$/, "");
  const c = command.trim();
  return (
    c.length > base.length + 1 &&
    c.startsWith(base) &&
    (c[base.length] === "/" || c[base.length] === "\\")
  );
}

/** The last `n` non-empty lines of `text`. */
export function tailLines(text: string, n = 3): string {
  return text
    .split("\n")
    .map((l) => l.trimEnd())
    .filter((l) => l.trim() !== "")
    .slice(-n)
    .join("\n");
}

/** A status as the Scripts tab names it. */
export const STATUS_LABEL: Record<ScriptStatus, string> = {
  ok: "Ran",
  failed: "Failed",
  timedOut: "Timed out",
  skipped: "Skipped",
  notFound: "Not found",
  notExecutable: "Not executable",
};

/** Whether a status is a failure worth an error toast. */
export function isFailure(status: ScriptStatus): boolean {
  return status !== "ok" && status !== "skipped";
}

/** What a failed result says: the tail of stderr, else Argus' message. */
export function failureDetail(r: ScriptResult): string {
  const tail = tailLines(r.stderr);
  const message = r.message?.trim() ?? "";
  if (tail && message) return `${message}\n${tail}`;
  return tail || message || STATUS_LABEL[r.status];
}

/** One toast to show for a submit's scripts. */
export interface ScriptToast {
  kind: "error" | "success" | "info";
  message: string;
  detail?: string;
}

/**
 * The toasts for one `scripts:done`: an error per failed script (its name,
 * the last lines of stderr or Argus' message), one quiet note for the
 * skipped ones, and a short success line for the ones that ran fine.
 */
export function doneToasts(done: ScriptsDone): ScriptToast[] {
  const out: ScriptToast[] = [];
  const ok = done.results.filter((r) => r.status === "ok");
  const skipped = done.results.filter((r) => r.status === "skipped");
  for (const r of done.results.filter((r) => isFailure(r.status))) {
    out.push({
      kind: "error",
      message: `Script "${r.name}": ${STATUS_LABEL[r.status].toLowerCase()}`,
      detail: failureDetail(r),
    });
  }
  if (ok.length === 1) {
    out.push({ kind: "success", message: `${ok[0]!.name} ran` });
  } else if (ok.length > 1) {
    out.push({ kind: "success", message: `Ran ${ok.length} scripts` });
  }
  if (skipped.length > 0) {
    out.push({
      kind: "info",
      message:
        skipped.length === 1
          ? `Skipped ${skipped[0]!.name}`
          : `Skipped ${skipped.length} scripts`,
      detail: skipped.map((r) => r.message ?? "").find((m) => m) ?? undefined,
    });
  }
  return out;
}

/**
 * The file name Argus saves a script under, as the core makes it
 * (`scripts::save::file_name`): lower-case letters and digits, a `-` for
 * each run of anything else, at most 64 characters, then `.sh`. "" when no
 * letter or digit is left.
 */
export function slugFileName(name: string): string {
  const trimmed = name.trim();
  const stem = /\.sh$/i.test(trimmed) ? trimmed.slice(0, -3) : trimmed;
  let slug = "";
  for (const c of stem) {
    if (/^[A-Za-z0-9]$/.test(c)) slug += c.toLowerCase();
    else if (!slug.endsWith("-")) slug += "-";
  }
  slug = slug
    .replace(/^-+|-+$/g, "")
    .slice(0, 64)
    .replace(/-+$/, "");
  return slug ? `${slug}.sh` : "";
}

// --- Write with AI drafts ---

/** The draft's newest script: the last turn's that has one, else its base. */
export function latestScript(d: ScriptDraft): GeneratedScript | null {
  for (let i = d.turns.length - 1; i >= 0; i--) {
    const r = d.turns[i]!.result;
    if (r) return r;
  }
  return d.base;
}

/** `drafts` with `d` in place (or added): the higher `revision` wins. */
export function upsertDraft(
  drafts: readonly ScriptDraft[],
  d: ScriptDraft,
): ScriptDraft[] {
  const at = drafts.findIndex((x) => x.id === d.id);
  if (at < 0) return [...drafts, d];
  if (drafts[at]!.revision > d.revision) return [...drafts];
  return drafts.map((x, i) => (i === at ? d : x));
}

/**
 * `drafts` with a `script-draft:progress` applied: only to a draft that is
 * running on that turn (a late report after the end is dropped).
 */
export function withProgress(
  drafts: readonly ScriptDraft[],
  e: ScriptDraftProgressEvent,
): ScriptDraft[] {
  return drafts.map((d) =>
    d.id === e.draftId &&
    d.status === "running" &&
    !d.waiting &&
    e.progress.turn === d.turns.length - 1
      ? { ...d, progress: e.progress }
      : d,
  );
}

/** A running turn's stage in words. */
export function stageLabel(stage: DraftStage, agent = "the agent"): string {
  switch (stage) {
    case "reading":
      return "Reading what you linked…";
    case "starting":
      return `Starting ${agent}…`;
    case "thinking":
      return "Thinking…";
    case "writing":
      return "Writing the script…";
    case "validating":
      return "Checking the answer…";
    case "repairing":
      return "The answer was not usable: asking again…";
  }
}

/** "42s", "3m 05s", "1h 02m". */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

/** One line for the Drafts section: the notice, else the thinking's last line. */
export function progressSnippet(
  p: ScriptDraftProgress | null,
  max = 90,
): string | null {
  if (!p) return null;
  if (p.notice) return p.notice;
  const lines = p.thinking
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const last = lines[lines.length - 1];
  if (!last) return null;
  return last.length > max ? `…${last.slice(last.length - max + 1)}` : last;
}

/** "Attached: MR !4242 · Code_review.md", or null without any. */
export function attachedLabel(attachments: readonly string[]): string | null {
  return attachments.length ? `Attached: ${attachments.join(" · ")}` : null;
}

/** A draft's status in words, for the Drafts section and the writer. */
export function draftStatusLabel(d: ScriptDraft): string {
  switch (d.status) {
    case "running":
      if (d.waiting) return "Waiting for another draft…";
      return d.progress ? stageLabel(d.progress.stage) : "Writing…";
    case "idle":
      return latestScript(d) ? "Ready" : "Not started";
    case "failed":
      return `Failed: ${d.error ?? "unknown error"}`;
    case "cancelled":
      return "Stopped";
  }
}

/** "1 request" / "3 requests". */
export function requestCount(d: ScriptDraft): string {
  const n = d.turns.length;
  return `${n} request${n === 1 ? "" : "s"}`;
}
