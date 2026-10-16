import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import { reviewHistories, threadHistory } from "./commands";

const mockInvoke = vi.mocked(invoke);
const REVIEW = "00000000-0000-4000-8000-000000000001";
const THREAD = "00000000-0000-4000-8000-000000000002";

beforeEach(() => {
  vi.clearAllMocks();
  mockInvoke.mockResolvedValue(undefined);
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
