import { describe, expect, it } from "vitest";

import { file } from "../test/fixtures";
import {
  BLOCK_PX,
  estimateBodyHeight,
  FOLD_PX,
  heightKey,
  LINE_PX,
  measuredHeight,
  NOTE_PX,
  rememberHeight,
} from "./cardHeight";

const patch = [
  "@@ -1,4 +1,4 @@",
  " a",
  "-b",
  "-c",
  "+B",
  " d",
  "@@ -20,3 +20,4 @@",
  " x",
  "+y",
  " z",
].join("\n");

describe("estimateBodyHeight", () => {
  const f = file({ patch, additions: 2, deletions: 2 });

  it("counts the hunks' rows and a fold around each", () => {
    // Unified: 2 context + 2 removed + 1 added, then 2 context + 1 added.
    const unified = estimateBodyHeight(f, {
      split: false,
      threads: 0,
      large: false,
    });
    expect(unified).toBe(8 + 8 * LINE_PX + 3 * FOLD_PX);
    // Side by side a change takes its longer side.
    const split = estimateBodyHeight(f, {
      split: true,
      threads: 0,
      large: false,
    });
    expect(split).toBe(8 + 7 * LINE_PX + 3 * FOLD_PX);
  });

  it("adds its threads and keeps a large or binary file short", () => {
    const base = estimateBodyHeight(f, {
      split: false,
      threads: 0,
      large: false,
    });
    expect(
      estimateBodyHeight(f, { split: false, threads: 2, large: false }),
    ).toBe(base + 2 * BLOCK_PX);
    expect(
      estimateBodyHeight(f, { split: false, threads: 0, large: true }),
    ).toBe(NOTE_PX);
    expect(
      estimateBodyHeight(file({ binary: true, patch: null }), {
        split: false,
        threads: 0,
        large: false,
      }),
    ).toBe(NOTE_PX);
  });
});

describe("measured heights", () => {
  it("are kept per review, file, layout and whitespace mode", () => {
    const k = heightKey("r1", "src/a.rs", "unified", false);
    expect(measuredHeight(k)).toBeNull();
    rememberHeight(k, 640);
    expect(measuredHeight(k)).toBe(640);
    expect(measuredHeight(heightKey("r1", "src/a.rs", "split", false))).toBe(
      null,
    );
    expect(measuredHeight(heightKey("r1", "src/a.rs", "unified", true))).toBe(
      null,
    );
    // Nothing drawn: not a height.
    rememberHeight(k, 0);
    expect(measuredHeight(k)).toBe(640);
  });
});
