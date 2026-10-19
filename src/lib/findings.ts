/**
 * Pure rules for the AI pass' output (`aiStore`, the AI island, the inline
 * finding blocks): ordering, rounds, the text a finding posts with, where a
 * finding sits in a file's diff, and the labels.
 */

import type {
  AiRunState,
  ChecklistItem,
  Finding,
  Severity,
  VerdictLabel,
} from "../ipc/types";
import type { DiffSide } from "./diffThreads";

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

/**
 * "2 skipped: already raised in the review" for the findings the latest
 * run left out as repeats of an existing thread or a posted finding; `null`
 * when it left out none.
 */
export function skippedDuplicatesText(run: AiRunState): string | null {
  const n = run.droppedDuplicates;
  return n > 0 ? `${n} skipped: already raised in the review` : null;
}

/** The review header chip's text for a run with `findings` findings shown. */
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
