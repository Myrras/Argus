/** Fictitious AI pass wire objects for tests. */

import type { AiRunState, ChecklistItem, Finding, Verdict } from "../ipc/types";

let seq = 0;
const uuid = (n: number) =>
  `00000000-0000-4000-a000-${String(n).padStart(12, "0")}`;

export function aiRun(overrides: Partial<AiRunState> = {}): AiRunState {
  return {
    reviewId: uuid(1),
    enabled: true,
    status: "done",
    queued: false,
    round: 1,
    adapter: "claude",
    error: null,
    startedAt: "2026-10-16T10:00:00Z",
    finishedAt: "2026-10-16T10:02:00Z",
    lastDoneRound: 1,
    droppedFindings: 0,
    droppedVerdicts: 0,
    droppedDuplicates: 0,
    readOnlyGuaranteed: true,
    ...overrides,
  };
}

export function finding(overrides: Partial<Finding> = {}): Finding {
  seq += 1;
  return {
    id: uuid(100 + seq),
    reviewId: uuid(1),
    round: 1,
    path: "src/cache.rs",
    line: 2,
    side: "new",
    severity: "medium",
    title: `Fixture finding ${seq}`,
    body: `Fixture body ${seq}.`,
    dismissed: false,
    postedAt: null,
    postedUrl: null,
    ...overrides,
  };
}

export function checklistItem(
  overrides: Partial<ChecklistItem> = {},
): ChecklistItem {
  seq += 1;
  return {
    id: uuid(500 + seq),
    reviewId: uuid(1),
    round: 1,
    text: `Check fixture thing ${seq}`,
    checked: false,
    ...overrides,
  };
}

export function verdict(overrides: Partial<Verdict> = {}): Verdict {
  const aiLabel =
    overrides.aiLabel === undefined ? "partly" : overrides.aiLabel;
  const userOverride = overrides.userOverride ?? null;
  return {
    threadId: uuid(900),
    reviewId: uuid(1),
    round: 2,
    aiLabel,
    aiReason: "The fixture guard was added, the test is missing.",
    userOverride,
    label: userOverride ?? aiLabel,
    ...overrides,
  };
}
