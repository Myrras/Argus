import { describe, expect, it } from "vitest";

import type { CommentableLines } from "../ipc/types";
import {
  clampRange,
  commentableSet,
  diffSideOf,
  hunkLines,
  outsideNote,
  rangeLabel,
  regionOf,
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
  outside: "none",
};

/** The same diff on a platform that takes lines outside the hunks. */
const open: CommentableLines = { ...lines, outside: "native" };

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

  it("takes every line when the platform takes lines outside the diff", () => {
    // Not without a line count, and never for `none`.
    expect(commentableSet(open, "new").size).toBe(7);
    expect(commentableSet(lines, "new", 30).size).toBe(7);
    const all = commentableSet(open, "new", 30);
    expect(all.size).toBe(30);
    expect(all.has(10)).toBe(true);
  });

  it("splits a side into hunks and the stretches between them", () => {
    expect(regionOf(open, "new", 2, 30)).toEqual({
      key: "h0",
      outside: false,
      from: 1,
      to: 4,
    });
    expect(regionOf(open, "new", 10, 30)).toEqual({
      key: "g1",
      outside: true,
      from: 5,
      to: 20,
    });
    expect(regionOf(open, "new", 25, 30)).toEqual({
      key: "g2",
      outside: true,
      from: 24,
      to: 30,
    });
    // The old side counts its own lines.
    expect(regionOf(open, "old", 19, 25)).toMatchObject({
      key: "g1",
      from: 4,
      to: 19,
    });
    // Past the end, or a platform without outside lines.
    expect(regionOf(open, "new", 31, 30)).toBeNull();
    expect(regionOf(lines, "new", 10, 30)).toBeNull();
  });

  it("keeps a range inside its stretch outside the diff", () => {
    expect(clampRange(open, "new", 8, 12, 30)).toEqual({
      line: 12,
      startLine: 8,
    });
    // Into the next hunk: stops at its edge.
    expect(clampRange(open, "new", 8, 22, 30)).toEqual({
      line: 20,
      startLine: 8,
    });
    // From a hunk outward: stops at the hunk's end.
    expect(clampRange(open, "new", 3, 8, 30)).toEqual({
      line: 4,
      startLine: 3,
    });
    // After the last hunk, up to the end of the file.
    expect(clampRange(open, "new", 26, 99, 30)).toEqual({
      line: 30,
      startLine: 26,
    });
  });

  it("says what a comment outside the diff becomes", () => {
    expect(outsideNote("fallback")).toContain("file comment");
    expect(outsideNote("native")).toContain("unchanged lines");
    expect(outsideNote("none")).toBeNull();
    expect(outsideNote(null)).toBeNull();
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
