import { describe, expect, it } from "vitest";

import type { CommentableLines } from "../ipc/types";
import {
  clampRange,
  commentableSet,
  diffSideOf,
  hunkLines,
  rangeLabel,
  sideOf,
} from "./commentRanges";

const lines: CommentableLines = {
  reviewId: "00000000-0000-4000-8000-000000000001",
  path: "src/cache.rs",
  baseSha: "b".repeat(40),
  headSha: "a".repeat(40),
  hunks: [
    { old: [1, 2, 3], new: [1, 2, 3, 4] },
    { old: [20, 21], new: [21, 22, 23] },
  ],
};

describe("commentRanges", () => {
  it("collects a side's commentable lines", () => {
    expect([...commentableSet(lines, "new")]).toEqual([1, 2, 3, 4, 21, 22, 23]);
    expect([...commentableSet(lines, "old")]).toEqual([1, 2, 3, 20, 21]);
    expect(commentableSet(null, "new").size).toBe(0);
    expect(hunkLines(lines, "new", 22)).toEqual([21, 22, 23]);
    expect(hunkLines(lines, "new", 10)).toBeNull();
  });

  it("keeps a range inside the hunk it started in", () => {
    expect(clampRange(lines, "new", 2, 2)).toEqual({
      line: 2,
      startLine: null,
    });
    expect(clampRange(lines, "new", 2, 4)).toEqual({ line: 4, startLine: 2 });
    // Upwards works too.
    expect(clampRange(lines, "new", 4, 1)).toEqual({ line: 4, startLine: 1 });
    // Dragged into the next hunk: clamped to the first hunk's end.
    expect(clampRange(lines, "new", 2, 22)).toEqual({ line: 4, startLine: 2 });
    // Not commentable where it started: nothing.
    expect(clampRange(lines, "new", 10, 12)).toBeNull();
  });

  it("names sides and ranges", () => {
    expect(sideOf("a")).toBe("old");
    expect(diffSideOf("new")).toBe("b");
    expect(rangeLabel("new", { line: 4, startLine: 2 })).toBe("lines 2–4");
    expect(rangeLabel("old", { line: 3, startLine: null })).toBe(
      "line 3 (old)",
    );
  });
});
