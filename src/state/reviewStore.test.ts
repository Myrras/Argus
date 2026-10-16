import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, review } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import type { RefreshReport } from "../ipc/types";
import { subscribeReviewEvents, useReviewStore } from "./reviewStore";

const mockInvoke = vi.mocked(invoke);
const initial = useReviewStore.getState();

const a = review({ title: "A" });
const b = review({ title: "B" });
const c = review({ title: "C", status: "waitingOnAuthor" });

const report = (at: string): RefreshReport => ({
  changed: [a.id],
  errors: [
    {
      provider: "gitlab",
      review: null,
      message: "timed out",
      unavailable: false,
    },
  ],
  refreshedAt: at,
});

describe("reviewStore", () => {
  beforeEach(() => {
    useReviewStore.setState(
      { ...initial, reviews: [a, b, c], loaded: true },
      true,
    );
  });

  it("loads the list and the last refresh", async () => {
    const last = report("2026-10-16T10:00:00Z");
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_reviews" ? [a] : last),
    );
    useReviewStore.setState({ ...initial }, true);
    await useReviewStore.getState().load();
    expect(useReviewStore.getState()).toMatchObject({
      reviews: [a],
      loaded: true,
      lastRefresh: last,
    });
  });

  it("re-reads one review on review:changed and the list on reviews:refreshed", async () => {
    const updated = {
      ...a,
      title: "A, renamed",
      status: "needsReReview" as const,
    };
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "get_review" ? updated : [updated, b]),
    );
    const off = await subscribeReviewEvents();

    events.emit("review:changed", { reviewId: a.id });
    await vi.waitFor(() =>
      expect(useReviewStore.getState().reviews[0].title).toBe("A, renamed"),
    );
    expect(mockInvoke).toHaveBeenCalledWith("get_review", { reviewId: a.id });
    expect(useReviewStore.getState().reviews).toHaveLength(3);

    const r = report("2026-10-16T11:00:00Z");
    events.emit("reviews:refreshed", r);
    await vi.waitFor(() =>
      expect(useReviewStore.getState().reviews).toEqual([updated, b]),
    );
    expect(useReviewStore.getState().lastRefresh).toEqual(r);

    off();
    expect(events.count("review:changed")).toBe(0);
    expect(events.count("reviews:refreshed")).toBe(0);
  });

  it("refreshes, keeps the report and never treats provider errors as failures", async () => {
    const r = report("2026-10-16T12:00:00Z");
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "refresh_reviews" ? r : [a]),
    );
    await useReviewStore.getState().refresh();
    expect(mockInvoke).toHaveBeenCalledWith("refresh_reviews");
    expect(useReviewStore.getState()).toMatchObject({
      refreshing: false,
      lastRefresh: r,
      lastError: null,
      reviews: [a],
    });
  });

  it("marks a round reviewed, then re-reads the list", async () => {
    const marked = { ...a, status: "waitingOnAuthor" as const };
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "mark_reviewed" ? marked : [marked, b, c]),
    );
    await useReviewStore.getState().markReviewed(a.id);
    expect(mockInvoke).toHaveBeenCalledWith("mark_reviewed", {
      reviewId: a.id,
    });
    expect(useReviewStore.getState().reviews[0].status).toBe("waitingOnAuthor");
    expect(useReviewStore.getState().marking).toBeNull();
  });

  it("moves the cursor within the current tab and opens it", () => {
    const s = useReviewStore.getState();
    s.moveCursor(1);
    expect(useReviewStore.getState().cursorId).toBe(a.id);
    s.moveCursor(1);
    s.moveCursor(1); // clamped: c is in another tab
    expect(useReviewStore.getState().cursorId).toBe(b.id);
    s.openCursor();
    expect(useReviewStore.getState().selectedId).toBe(b.id);
    s.moveCursor(-1);
    expect(useReviewStore.getState().cursorId).toBe(a.id);
    s.setTab("waitingOnAuthor");
    s.moveCursor(1);
    expect(useReviewStore.getState().cursorId).toBe(c.id);
  });
});
