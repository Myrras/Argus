import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, thread } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import { subscribeThreadEvents, useThreadStore } from "./threadStore";

const mockInvoke = vi.mocked(invoke);
const initial = useThreadStore.getState();
const OPEN = "00000000-0000-4000-8000-0000000000aa";
const OTHER = "00000000-0000-4000-8000-0000000000bb";

describe("threadStore", () => {
  beforeEach(() => {
    useThreadStore.setState({ ...initial, byReview: {} }, true);
  });

  it("loads a review's threads", async () => {
    const t = thread({ reviewId: OPEN });
    mockInvoke.mockResolvedValue([t]);
    await useThreadStore.getState().load(OPEN);
    expect(mockInvoke).toHaveBeenCalledWith("list_threads", { reviewId: OPEN });
    expect(useThreadStore.getState().byReview[OPEN]).toEqual({
      threads: [t],
      loading: false,
      error: null,
    });
  });

  it("reloads on review:changed only for reviews it holds", async () => {
    const before = thread({ reviewId: OPEN });
    const after = thread({ reviewId: OPEN, resolved: true });
    useThreadStore.setState(
      {
        ...initial,
        byReview: {
          [OPEN]: { threads: [before], loading: false, error: null },
        },
      },
      true,
    );
    mockInvoke.mockResolvedValue([after]);
    const off = await subscribeThreadEvents();

    events.emit("review:changed", { reviewId: OTHER });
    expect(mockInvoke).not.toHaveBeenCalled();

    events.emit("review:changed", { reviewId: OPEN });
    await vi.waitFor(() =>
      expect(useThreadStore.getState().byReview[OPEN].threads).toEqual([after]),
    );
    off();
  });

  it("keeps the threads and records the error when a load fails", async () => {
    const t = thread({ reviewId: OPEN });
    useThreadStore.setState(
      {
        ...initial,
        byReview: { [OPEN]: { threads: [t], loading: false, error: null } },
      },
      true,
    );
    mockInvoke.mockRejectedValue({ kind: "notFound", message: "not found" });
    await useThreadStore.getState().load(OPEN);
    expect(useThreadStore.getState().byReview[OPEN]).toEqual({
      threads: [t],
      loading: false,
      error: { kind: "notFound", message: "not found" },
    });
  });
});
