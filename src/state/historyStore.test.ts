import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, thread } from "../test/fixtures";
import { HEAD_SHA, historyFor } from "../test/historyFixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import type { StoredThread } from "../ipc/types";
import {
  clearHistoryReads,
  subscribeHistoryEvents,
  useHistoryStore,
} from "./historyStore";
import { useThreadStore } from "./threadStore";

const mockInvoke = vi.mocked(invoke);
const initial = useHistoryStore.getState();
const initialThreads = useThreadStore.getState();
const OPEN = "00000000-0000-4000-8000-0000000000aa";
const OTHER = "00000000-0000-4000-8000-0000000000bb";

const a = thread({ reviewId: OPEN, mine: true });
const b = thread({ reviewId: OPEN });

function seedThreads(threads: StoredThread[]) {
  useThreadStore.setState(
    {
      ...initialThreads,
      byReview: { [OPEN]: { threads, loading: false, error: null } },
    },
    true,
  );
}

function calls(cmd: string) {
  return mockInvoke.mock.calls.filter(([c]) => c === cmd).length;
}

describe("historyStore", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    clearHistoryReads();
    useHistoryStore.setState(
      {
        ...initial,
        byReview: {},
        byThread: {},
        expanded: {},
        open: {},
        focused: null,
      },
      true,
    );
    seedThreads([a, b]);
  });

  it("loads a review's histories in one call, once per head", async () => {
    mockInvoke.mockResolvedValue([historyFor(a), historyFor(b)]);
    await useHistoryStore.getState().loadReview(OPEN, HEAD_SHA);
    expect(mockInvoke).toHaveBeenCalledWith("review_histories", {
      reviewId: OPEN,
    });
    const s = useHistoryStore.getState();
    expect(s.byThread[a.threadId]).toEqual({
      history: historyFor(a),
      loading: false,
      error: null,
    });
    expect(s.byReview[OPEN]).toMatchObject({
      headSha: HEAD_SHA,
      loaded: true,
      error: null,
    });

    await useHistoryStore.getState().loadReview(OPEN, HEAD_SHA);
    expect(calls("review_histories")).toBe(1);
    await useHistoryStore.getState().loadReview(OPEN, "f".repeat(40));
    expect(calls("review_histories")).toBe(2);
  });

  it("re-reads on review:changed only for reviews it holds", async () => {
    mockInvoke.mockResolvedValue([historyFor(a)]);
    await useHistoryStore.getState().loadReview(OPEN, HEAD_SHA);
    const off = await subscribeHistoryEvents();

    events.emit("review:changed", { reviewId: OTHER });
    expect(calls("review_histories")).toBe(1);

    const touched = historyFor(a, { touchedBy: ["c1"] });
    mockInvoke.mockResolvedValue([touched]);
    events.emit("review:changed", { reviewId: OPEN });
    await vi.waitFor(() =>
      expect(useHistoryStore.getState().byThread[a.threadId].history).toEqual(
        touched,
      ),
    );
    expect(calls("review_histories")).toBe(2);
    off();
  });

  it("re-reads when the threads are re-read with new content only", async () => {
    mockInvoke.mockResolvedValue([historyFor(a)]);
    await useHistoryStore.getState().loadReview(OPEN, HEAD_SHA);

    // The same threads again (a review reopened): nothing to do.
    seedThreads([{ ...a }, { ...b }]);
    expect(calls("review_histories")).toBe(1);

    const replied: StoredThread = {
      ...a,
      notes: [
        ...a.notes,
        {
          id: "reply",
          author: "bob-fixture",
          body: "Done.",
          createdAt: "2026-10-16T09:00:00Z",
          system: false,
        },
      ],
    };
    seedThreads([replied, b]);
    await vi.waitFor(() => expect(calls("review_histories")).toBe(2));
  });

  it("coalesces overlapping re-reads into one more call", async () => {
    let release: (v: unknown) => void = () => undefined;
    mockInvoke.mockImplementationOnce(
      () => new Promise((r) => (release = r)) as never,
    );
    mockInvoke.mockResolvedValue([historyFor(a)]);
    const first = useHistoryStore.getState().loadReview(OPEN, HEAD_SHA);
    void useHistoryStore.getState().reloadReview(OPEN);
    void useHistoryStore.getState().reloadReview(OPEN);
    release([historyFor(a)]);
    await first;
    expect(calls("review_histories")).toBe(2);
  });

  it("keeps earlier histories and records the error per thread", async () => {
    mockInvoke.mockResolvedValueOnce([historyFor(a)]);
    await useHistoryStore.getState().loadReview(OPEN, HEAD_SHA);
    const err = { kind: "provider", message: "GitHub is unreachable" };
    mockInvoke.mockRejectedValueOnce(err);
    await useHistoryStore.getState().reloadReview(OPEN);
    const s = useHistoryStore.getState();
    expect(s.byThread[a.threadId]).toEqual({
      history: historyFor(a),
      loading: false,
      error: err,
    });
    expect(s.byThread[b.threadId]).toEqual({
      history: null,
      loading: false,
      error: err,
    });
    expect(s.byReview[OPEN].error).toEqual(err);
  });

  it("loads one thread on demand", async () => {
    mockInvoke.mockResolvedValue(historyFor(b));
    await useHistoryStore.getState().loadThread(b.threadId);
    expect(mockInvoke).toHaveBeenCalledWith("thread_history", {
      threadId: b.threadId,
    });
    expect(useHistoryStore.getState().byThread[b.threadId].history).toEqual(
      historyFor(b),
    );
  });

  it("toggles the then/now of the thread last clicked in the open review", () => {
    const s = useHistoryStore.getState();
    expect(s.toggleFocused(OPEN)).toBe(false);
    s.focus({ threadId: a.threadId, reviewId: OPEN, mine: true });
    expect(useHistoryStore.getState().toggleFocused(OTHER)).toBe(false);
    expect(useHistoryStore.getState().toggleFocused(OPEN)).toBe(true);
    // Mine starts open, so the first toggle closes it.
    expect(useHistoryStore.getState().expanded[a.threadId]).toBe(false);
    useHistoryStore.getState().toggleFocused(OPEN);
    expect(useHistoryStore.getState().expanded[a.threadId]).toBe(true);
  });
});
