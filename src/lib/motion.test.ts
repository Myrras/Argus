import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MOTION, usePresence } from "./motion";

describe("usePresence", () => {
  afterEach(() => vi.useRealTimers());

  it("keeps a closing element mounted for its exit animation", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ open }) => usePresence(open, MOTION.base),
      { initialProps: { open: true } },
    );
    expect(result.current).toEqual({ mounted: true, state: "open" });

    rerender({ open: false });
    expect(result.current).toEqual({ mounted: true, state: "closed" });

    act(() => vi.advanceTimersByTime(MOTION.base));
    expect(result.current.mounted).toBe(false);
  });
});
