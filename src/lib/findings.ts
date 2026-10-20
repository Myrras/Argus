/**
 * Pure rules for the AI pass' output (`aiStore`, the AI island, the inline
 * finding blocks): ordering, rounds, the text a finding posts with, where a
 * finding sits in a file's diff, and the labels.
 */

import type {
  AiRunState,
  ChecklistItem,
  Finding,
  RelatedRef,
  RepeatReason,
  RunContext,
  Severity,
  StoredThread,
  VerdictLabel,
} from "../ipc/types";
import type { DiffSide } from "./diffThreads";
import { SOURCE_LABEL } from "./reviewSkills";

export const SEVERITIES: Severity[] = ["high", "medium", "low"];

export const SEVERITY_LABEL: Record<Severity, string> = {
  high: "High",
  medium: "Medium",
  low: "Low",
};

export const VERDICT_LABEL: Record<VerdictLabel, string> = {
  addressed: "Addressed",
  partly: "Partly addressed",
  untouched: "Untouched",
};

/** Lowercase, for "you: addressed · AI: partly". */
export const VERDICT_SHORT: Record<VerdictLabel, string> = {
  addressed: "addressed",
  partly: "partly",
  untouched: "untouched",
};

/** The `Badge` tone per severity. */
export const SEVERITY_TONE: Record<Severity, "danger" | "warning" | "neutral"> =
  {
    high: "danger",
    medium: "warning",
    low: "neutral",
  };

const RANK: Record<Severity, number> = { high: 0, medium: 1, low: 2 };

/** High → medium → low, then path, then line (a new array). */
export function sortFindings(findings: readonly Finding[]): Finding[] {
  return [...findings].sort(
    (a, b) =>
      RANK[a.severity] - RANK[b.severity] ||
      a.path.localeCompare(b.path) ||
      a.line - b.line,
  );
}

/** The findings per severity, each sorted by path and line; empty groups left out. */
export function groupBySeverity(
  findings: readonly Finding[],
): { severity: Severity; findings: Finding[] }[] {
  const sorted = sortFindings(findings);
  return SEVERITIES.map((severity) => ({
    severity,
    findings: sorted.filter((f) => f.severity === severity),
  })).filter((g) => g.findings.length > 0);
}

/**
 * The round the AI's output is current for: the last successful run's, else
 * the newest round any finding or item has. `null` before any output.
 */
export function currentRound(
  state: AiRunState | null | undefined,
  findings: readonly Finding[],
  checklist: readonly ChecklistItem[] = [],
): number | null {
  if (state?.lastDoneRound != null) return state.lastDoneRound;
  let max: number | null = null;
  for (const x of [...findings, ...checklist]) {
    if (max === null || x.round > max) max = x.round;
  }
  return max;
}

/** An unposted finding of an earlier round than `current`. */
export function fromEarlierRound(f: Finding, current: number | null): boolean {
  return current !== null && f.round < current && !f.postedAt;
}

/** What a finding posts as without an edit (the backend's own default). */
export function defaultBody(f: Finding): string {
  return `**${f.title}**\n\n${f.body}`;
}

/** Can still be selected and posted. */
export function postable(f: Finding): boolean {
  return !f.postedAt && !f.dismissed;
}

/** `src/cache.rs:42` (`old` side marked). */
export function findingLocation(f: Finding): string {
  return `${f.path}:${f.side === "old" ? "old " : ""}${f.line}`;
}

/**
 * Where a finding goes in its file's diff: the editor side and line, or
 * `null` when its line is not in the diff shown (it is then listed at the
 * top of the card). In "since my last review" mode the left side is the
 * last-reviewed commit, which old-side lines do not refer to.
 */
export function findingPlacement(
  f: Finding,
  docLines: Record<DiffSide, number>,
  since: boolean,
): { side: DiffSide; line: number } | null {
  const side: DiffSide = f.side === "old" ? "a" : "b";
  if (side === "a" && since) return null;
  if (f.line < 1 || f.line > docLines[side]) return null;
  return { side, line: f.line };
}

/** The checklist's newest round, and the earlier rounds (newest first). */
export function splitChecklist(items: readonly ChecklistItem[]): {
  round: number | null;
  current: ChecklistItem[];
  earlier: ChecklistItem[];
} {
  const round = items.reduce<number | null>(
    (m, i) => (m === null || i.round > m ? i.round : m),
    null,
  );
  return {
    round,
    current: items.filter((i) => i.round === round),
    earlier: items
      .filter((i) => i.round !== round)
      .sort((a, b) => b.round - a.round),
  };
}

/** "6 findings", "1 finding". */
export function findingCount(n: number): string {
  return `${n} ${n === 1 ? "finding" : "findings"}`;
}

/** What is set aside as already raised: how many, and how many are high. */
export interface SkippedSummary {
  total: number;
  high: number;
}

/**
 * The set-aside findings and checklist items counted, from the lists the
 * panel shows (never the run's own counts, which a rerun or "Show anyway"
 * leaves behind).
 */
export function skippedSummary(
  findings: readonly Finding[],
  items: readonly ChecklistItem[],
): SkippedSummary {
  return {
    total: findings.length + items.length,
    high: findings.filter((f) => f.severity === "high").length,
  };
}

/** "Skipped as already raised (3)", or "(3, 1 high)" with high ones. */
export function skippedHeader(s: SkippedSummary): string {
  const high = s.high > 0 ? `, ${s.high} high` : "";
  return `Skipped as already raised (${s.total}${high})`;
}

/** "2 skipped: already raised in the review"; `null` for none. */
export function skippedText(s: SkippedSummary): string | null {
  return s.total > 0
    ? `${s.total} skipped: already raised in the review`
    : null;
}

/** Why a finding was set aside, for its tooltip. */
export const REPEAT_REASON_HINT: Record<RepeatReason, string> = {
  agentMarked:
    "The AI named this thread as covering the point, and the text says the same",
  similar: "Argus found the same point in this thread, near the same lines",
};

/** Tooltip of a "related to …" tag. */
export const RELATED_HINT =
  "Close to that comment (the AI's mark or the same words), but kept as a point of its own";

/** Tooltip of a "repeats …" tag on a finding or item shown anyway. */
export const SHOWN_ANYWAY_HINT =
  "Shown anyway: the review already makes this point";

/**
 * A point of the review by who made it: "bob's comment", "your comment" (a
 * thread the user started), "your posted finding", or "an existing comment"
 * when the thread is not loaded.
 */
export function pointLabel(
  of: RelatedRef | null,
  threads: readonly StoredThread[],
): string {
  if (!of) return "an existing comment";
  if (of.threadId) {
    const t = threads.find((x) => x.threadId === of.threadId);
    if (!t) return "an existing comment";
    return t.startedByMe ? "your comment" : `${t.author}'s comment`;
  }
  return of.findingId ? "your posted finding" : "an existing comment";
}

/** What a set-aside finding or checklist item repeats, for "repeats …". */
export function repeatTarget(
  f: Pick<Finding, "repeatOf">,
  threads: readonly StoredThread[],
): string {
  return pointLabel(f.repeatOf, threads);
}

/**
 * "related to bob's comment" ("… and 1 more") for a finding or checklist
 * item tied to points of the review it does not repeat (the agent's mark,
 * or close in words); `null` for none, or for one marked as a repeat (it
 * says "repeats …" instead).
 */
export function relatedLabel(
  f: Pick<Finding, "relatedTo" | "repeatOf">,
  threads: readonly StoredThread[],
): string | null {
  const [first, ...rest] = f.relatedTo;
  if (!first || f.repeatOf) return null;
  const more = rest.length > 0 ? ` and ${rest.length} more` : "";
  return `related to ${pointLabel(first, threads)}${more}`;
}

/**
 * The header chip's visible words: short, since its sparkle already says
 * "AI" and the tooltip and the AI panel carry the rest (a failure's reason).
 */
export function aiChipText(run: AiRunState, findings: number): string {
  if (run.status === "running") return "Running…";
  if (run.queued) return "Queued";
  switch (run.status) {
    case "done":
      return findingCount(findings);
    case "failed":
      return run.error === "cancelled" ? "Cancelled" : "Failed";
    default:
      return "Not run";
  }
}

/**
 * What the last successful run was given besides the diff, one line each,
 * for the AI chip's tooltip: the rules files, the review skill, the
 * worktree, and what did not go as set up. Empty without a context.
 */
export function aiContextLines(context: RunContext | null): string[] {
  if (!context) return [];
  const skill = context.skill
    ? `${context.skill.name} (${SOURCE_LABEL[context.skill.source]})`
    : "none";
  return [
    `Rules: ${context.rulesFiles.length > 0 ? context.rulesFiles.join(", ") : "none found"}`,
    `Review skill: ${skill}`,
    ...(context.worktree ? ["Read a worktree at the review's head"] : []),
    ...context.notes,
  ];
}

/** The review header chip's full text for a run with `findings` findings shown. */
export function aiStatusText(run: AiRunState, findings: number): string {
  if (run.status === "running") return "AI: running…";
  if (run.queued) return "AI: queued";
  switch (run.status) {
    case "done":
      return `AI: done · ${findingCount(findings)}`;
    case "failed":
      return run.error === "cancelled"
        ? "AI: cancelled"
        : `AI: failed — ${run.error ?? "unknown error"}`;
    default:
      return "AI: not started";
  }
}
