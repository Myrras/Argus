import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import {
  aiState,
  cancelAi,
  dismissFinding,
  listChecklist,
  listFindings,
  listVerdicts,
  postFindings,
  reviewHistories,
  runAi,
  setChecklistChecked,
  setVerdictOverride,
  threadHistory,
  toIpcError,
} from "./commands";
import { onAiState } from "./events";
import type { AiRunState } from "./types";

const mockInvoke = vi.mocked(invoke);
const REVIEW = "00000000-0000-4000-8000-000000000001";
const THREAD = "00000000-0000-4000-8000-000000000002";
const FINDING = "00000000-0000-4000-8000-000000000003";
const ITEM = "00000000-0000-4000-8000-000000000004";

beforeEach(() => {
  vi.clearAllMocks();
  mockInvoke.mockResolvedValue(undefined);
});

describe("toIpcError", () => {
  it("shows an internal failure as a generic error", () => {
    expect(
      toIpcError({ kind: "internal", message: "Something went wrong inside" }),
    ).toEqual({
      kind: "internal",
      message: "Something went wrong in Argus; see the log",
    });
    // Other kinds keep their own text.
    expect(
      toIpcError({ kind: "notFound", message: "No such review." }),
    ).toEqual({ kind: "notFound", message: "No such review." });
  });
});

describe("thread history wrappers", () => {
  it.each([
    [
      "thread_history",
      () => threadHistory(THREAD),
      { threadId: THREAD },
    ] as const,
    [
      "review_histories",
      () => reviewHistories(REVIEW),
      { reviewId: REVIEW },
    ] as const,
  ])("calls %s with camelCase args", async (cmd, call, args) => {
    await call();
    expect(mockInvoke).toHaveBeenCalledWith(cmd, args);
  });
});

describe("AI pass wrappers", () => {
  it.each([
    ["ai_state", () => aiState(REVIEW), { reviewId: REVIEW }],
    ["list_findings", () => listFindings(REVIEW), { reviewId: REVIEW }],
    ["list_checklist", () => listChecklist(REVIEW), { reviewId: REVIEW }],
    ["list_verdicts", () => listVerdicts(REVIEW), { reviewId: REVIEW }],
    ["run_ai", () => runAi(REVIEW), { reviewId: REVIEW }],
    ["cancel_ai", () => cancelAi(REVIEW), { reviewId: REVIEW }],
    [
      "dismiss_finding",
      () => dismissFinding(FINDING, true),
      { findingId: FINDING, dismissed: true },
    ],
    [
      "set_checklist_checked",
      () => setChecklistChecked(ITEM, false),
      { checklistId: ITEM, checked: false },
    ],
    [
      "set_verdict_override",
      () => setVerdictOverride(THREAD, "partly"),
      { threadId: THREAD, label: "partly" },
    ],
    [
      "set_verdict_override",
      () => setVerdictOverride(THREAD, null),
      { threadId: THREAD, label: null },
    ],
    [
      "post_findings",
      () =>
        postFindings(REVIEW, [
          { findingId: FINDING },
          { findingId: ITEM, body: "Edited text" },
        ]),
      {
        reviewId: REVIEW,
        posts: [
          { findingId: FINDING },
          { findingId: ITEM, body: "Edited text" },
        ],
      },
    ],
  ] as const)("calls %s with camelCase args", async (cmd, call, args) => {
    await call();
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith(cmd, args);
  });

  it("forwards ai:state payloads to onAiState", async () => {
    const seen: AiRunState[] = [];
    const unlisten = await onAiState((s) => seen.push(s));
    const state: AiRunState = {
      reviewId: REVIEW,
      enabled: true,
      status: "running",
      queued: false,
      round: 1,
      adapter: "claude",
      error: null,
      startedAt: "2026-01-02T03:04:05Z",
      finishedAt: null,
      lastDoneRound: null,
      droppedFindings: 0,
      droppedVerdicts: 0,
      droppedDuplicates: 0,
      readOnlyGuaranteed: true,
    };
    events.emit("ai:state", state);
    expect(seen).toEqual([state]);
    unlisten();
    expect(events.count("ai:state")).toBe(0);
  });
});
