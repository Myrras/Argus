import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, review } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { RefreshReport } from "../ipc/types";
import { reviewsInTab } from "../lib/reviews";
import {
  APPROVED_LINGER_MS,
  subscribeReviewEvents,
  useReviewStore,
} from "./reviewStore";
import { useToastStore } from "./toastStore";

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
      rateLimitedUntil: null,
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
    useToastStore.setState({ toasts: [] });
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
      reviews: [a],
    });
    // Provider errors stay in the report (the Providers section), no toast.
    expect(useToastStore.getState().toasts).toEqual([]);
  });

  it("keeps the newest report by time, not by spelling", async () => {
    // The core leaves out a zero fraction: as strings, "…01Z" sorts after
    // "…01.200Z", which is later.
    mockInvoke.mockImplementation(() => Promise.resolve([a]));
    useReviewStore.setState({ lastRefresh: report("2026-10-16T10:00:01Z") });
    const later = report("2026-10-16T10:00:01.200Z");
    await useReviewStore.getState().applyRefreshed(later);
    expect(useReviewStore.getState().lastRefresh).toEqual(later);
    await useReviewStore
      .getState()
      .applyRefreshed(report("2026-10-16T10:00:01Z"));
    expect(useReviewStore.getState().lastRefresh).toEqual(later);
  });

  it("toasts a failed refresh and keeps the list", async () => {
    mockInvoke.mockRejectedValue({ kind: "io", message: "disk full" });
    await useReviewStore.getState().refresh();
    expect(useReviewStore.getState()).toMatchObject({
      refreshing: false,
      reviews: [a, b, c],
    });
    expect(useToastStore.getState().toasts).toMatchObject([
      { kind: "error", message: "Could not refresh", detail: "disk full" },
    ]);
  });

  it("submits a review on the platform, then re-reads the list", async () => {
    const submitted = {
      ...a,
      status: "waitingOnAuthor" as const,
      myReviewState: "requestedChanges" as const,
    };
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "submit_review" ? submitted : [submitted, b, c]),
    );
    const err = await useReviewStore
      .getState()
      .submit(a.id, a.headSha, "requestChanges", "Please add a test.");
    expect(err).toBeNull();
    expect(mockInvoke).toHaveBeenCalledWith("submit_review", {
      reviewId: a.id,
      headSha: a.headSha,
      verdict: "requestChanges",
      summary: "Please add a test.",
    });
    expect(useReviewStore.getState().reviews[0].status).toBe("waitingOnAuthor");
    expect(useReviewStore.getState().approving).toBeNull();
    // Not an approval: nothing lingers.
    expect(useReviewStore.getState().justApproved).toEqual({});
    expect(useToastStore.getState().toasts).toMatchObject([
      {
        kind: "success",
        message: `Requested changes on acme-fixture/widgets#${a.number} at ${a.headSha.slice(0, 7)}`,
      },
    ]);
    // A blank summary goes as none.
    await useReviewStore.getState().submit(a.id, a.headSha, "approve", "  ");
    expect(mockInvoke).toHaveBeenLastCalledWith("list_reviews");
    expect(mockInvoke).toHaveBeenCalledWith("submit_review", {
      reviewId: a.id,
      headSha: a.headSha,
      verdict: "approve",
      summary: null,
    });
  });

  it("keeps an approved review in its tab for a moment, then lets it go", async () => {
    vi.useFakeTimers();
    try {
      const approved = {
        ...a,
        status: "approved" as const,
        approvedAt: "2026-10-16T12:00:00Z",
      };
      // The list leaves approved reviews out, as the backend's does.
      mockInvoke.mockImplementation((cmd: string) =>
        Promise.resolve(cmd === "submit_review" ? approved : [b, c]),
      );
      await useReviewStore.getState().approve(a.id, a.headSha);
      const inTab = () => {
        const s = useReviewStore.getState();
        return reviewsInTab(s.reviews, "needsReview", s.justApproved).map(
          (r) => r.title,
        );
      };
      expect(inTab()).toEqual(["A", "B"]);
      vi.advanceTimersByTime(APPROVED_LINGER_MS - 1);
      expect(inTab()).toEqual(["A", "B"]);
      vi.advanceTimersByTime(1);
      expect(inTab()).toEqual(["B"]);

      // Later the platform drops the approval: back in a tab as usual.
      useReviewStore.setState({
        reviews: [{ ...a, status: "needsReReview", approvedAt: null }, b, c],
      });
      const s = useReviewStore.getState();
      expect(
        reviewsInTab(s.reviews, "needsReReview", s.justApproved).map(
          (r) => r.title,
        ),
      ).toEqual(["A"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a failed submit keeps the review where it is", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "submit_review"
        ? Promise.reject({
            kind: "unsupported",
            message:
              "Your GitLab doesn't support requesting changes via the API",
          })
        : Promise.resolve([a, b, c]),
    );
    const err = await useReviewStore
      .getState()
      .submit(a.id, a.headSha, "requestChanges");
    expect(err).toMatchObject({ kind: "unsupported" });
    expect(useReviewStore.getState().justApproved).toEqual({});
    expect(useToastStore.getState().toasts).toMatchObject([
      { kind: "error", message: "Could not submit the review" },
    ]);
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
  it("keeps the newest list when an older read answers last", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(() => new Promise((r) => answers.push(r)));
    const older = useReviewStore.getState().reloadList();
    const newer = useReviewStore.getState().reloadList();
    answers[1]([b]);
    await newer;
    answers[0]([a]);
    await older;
    expect(useReviewStore.getState().reviews).toEqual([b]);
  });

  it("keeps the newest record when an older read of it answers last", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(() => new Promise((r) => answers.push(r)));
    const older = useReviewStore.getState().applyChanged(a.id);
    const newer = useReviewStore.getState().applyChanged(a.id);
    answers[1]({ ...a, title: "A, renamed" });
    await newer;
    answers[0](a);
    await older;
    expect(useReviewStore.getState().reviews[0].title).toBe("A, renamed");
  });
  it("a list read that started first never overwrites a newer record", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(() => new Promise((r) => answers.push(r)));
    const list = useReviewStore.getState().reloadList();
    const one = useReviewStore.getState().applyChanged(a.id);
    answers[1]({ ...a, title: "A, renamed" });
    await one;
    // The list was read before that record: its copy of A is older.
    answers[0]([a, { ...b, title: "B, renamed" }]);
    await list;
    expect(useReviewStore.getState().reviews.map((r) => r.title)).toEqual([
      "A, renamed",
      "B, renamed",
    ]);
  });

  it("a record read that started before a list read never overwrites the newer list", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(() => new Promise((r) => answers.push(r)));
    const one = useReviewStore.getState().applyChanged(a.id);
    const list = useReviewStore.getState().reloadList();
    answers[1]([{ ...a, title: "A, newest" }, b]);
    await list;
    // The record was read before the list: its answer is older.
    answers[0]({ ...a, title: "A, older" });
    await one;
    expect(useReviewStore.getState().reviews.map((r) => r.title)).toEqual([
      "A, newest",
      "B",
    ]);
  });

  it("a list read still drops a review that left the list", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(() => new Promise((r) => answers.push(r)));
    const list = useReviewStore.getState().reloadList();
    const one = useReviewStore.getState().applyChanged(a.id);
    answers[1]({ ...a, title: "A, renamed" });
    await one;
    answers[0]([b]);
    await list;
    expect(useReviewStore.getState().reviews.map((r) => r.title)).toEqual([
      "B",
    ]);
  });

  it("a second approval while one runs is refused, not reported done", async () => {
    let finish: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "submit_review"
        ? new Promise((r) => (finish = r))
        : Promise.resolve([a, b, c]),
    );
    const first = useReviewStore.getState().approve(a.id, a.headSha);
    const second = await useReviewStore.getState().approve(b.id, b.headSha);
    expect(second).toMatchObject({ kind: "busy" });
    finish({ ...a, approvedAt: "2026-10-16T12:00:00Z" });
    expect(await first).toBeNull();
  });
});
