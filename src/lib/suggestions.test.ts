import { describe, expect, it } from "vitest";

import { thread } from "../test/fixtures";
import {
  buildSuggestion,
  linesFromContents,
  linesFromHunk,
  originalSource,
  parseSuggestionInfo,
  suggestedLines,
  suggestionRange,
} from "./suggestions";

const anchor = {
  ...thread().anchor!,
  pathNew: "src/cache.rs",
  pathOld: "src/cache.rs",
  lineNew: 4,
  startLine: null,
  headSha: "c".repeat(40),
};

const FILE =
  "fn a() {}\nlet one = 1;\nlet two = 2;\nlet three = 3;\nfn b() {}\n";

describe("parseSuggestionInfo", () => {
  it("reads GitHub's bare fence and GitLab's -N+M range", () => {
    expect(parseSuggestionInfo("suggestion")).toEqual({ kind: "default" });
    expect(parseSuggestionInfo("  suggestion  ")).toEqual({ kind: "default" });
    expect(parseSuggestionInfo("suggestion:-2+1")).toEqual({
      kind: "relative",
      above: 2,
      below: 1,
    });
    expect(parseSuggestionInfo("suggestion:-0+0")).toEqual({
      kind: "relative",
      above: 0,
      below: 0,
    });
  });

  it("ignores other fences", () => {
    expect(parseSuggestionInfo("ts")).toBeNull();
    expect(parseSuggestionInfo("")).toBeNull();
    expect(parseSuggestionInfo("suggestions")).toBeNull();
    expect(parseSuggestionInfo("suggestion:+1")).toBeNull();
  });
});

describe("suggestionRange", () => {
  it("is the commented line on GitHub, or start_line..line", () => {
    expect(suggestionRange({ kind: "default" }, anchor, "github")).toEqual({
      start: 4,
      end: 4,
    });
    expect(
      suggestionRange(
        { kind: "default" },
        { ...anchor, startLine: 2 },
        "github",
      ),
    ).toEqual({ start: 2, end: 4 });
  });

  it("is relative to the anchored line on GitLab", () => {
    expect(
      suggestionRange(
        { kind: "relative", above: 2, below: 1 },
        anchor,
        "gitlab",
      ),
    ).toEqual({ start: 2, end: 5 });
    // A bare fence on GitLab is -0+0, even on a multi-line comment.
    expect(
      suggestionRange(
        { kind: "default" },
        { ...anchor, startLine: 2 },
        "gitlab",
      ),
    ).toEqual({ start: 4, end: 4 });
  });

  it("is unknown without a line, or above line 1", () => {
    expect(suggestionRange({ kind: "default" }, null, "github")).toBeNull();
    expect(
      suggestionRange(
        { kind: "default" },
        { ...anchor, lineNew: null },
        "github",
      ),
    ).toBeNull();
    expect(
      suggestionRange(
        { kind: "relative", above: 9, below: 0 },
        anchor,
        "gitlab",
      ),
    ).toBeNull();
  });
});

describe("suggestedLines", () => {
  it("splits the body; an empty one deletes", () => {
    expect(suggestedLines("let x = 1;\nlet y = 2;\n")).toEqual([
      "let x = 1;",
      "let y = 2;",
    ]);
    expect(suggestedLines("")).toEqual([]);
  });
});

describe("linesFromContents / linesFromHunk", () => {
  it("takes the range from the file", () => {
    expect(linesFromContents(FILE, { start: 2, end: 3 })).toEqual([
      "let one = 1;",
      "let two = 2;",
    ]);
    expect(linesFromContents(FILE, { start: 5, end: 6 })).toBeNull();
  });

  it("takes the last lines of the hunk's side, checked against its header", () => {
    const hunk =
      "@@ -1,3 +1,4 @@\n fn a() {}\n-let one = 0;\n+let one = 1;\n+let two = 2;\n let three = 3;";
    expect(linesFromHunk(hunk, { start: 3, end: 4 }, 4, "new")).toEqual([
      "let two = 2;",
      "let three = 3;",
    ]);
    // The header says the hunk ends at line 4, not 5.
    expect(linesFromHunk(hunk, { start: 5, end: 5 }, 5, "new")).toBeNull();
    // Lines below the anchor are not in the hunk.
    expect(linesFromHunk(hunk, { start: 4, end: 5 }, 4, "new")).toBeNull();
    // The old side: no added lines.
    expect(linesFromHunk(hunk, { start: 3, end: 3 }, 3, "old")).toEqual([
      "let three = 3;",
    ]);
  });
});

describe("originalSource", () => {
  it("reads the head side at the comment's commit, the base side at GitLab's base", () => {
    expect(originalSource(anchor)).toEqual({
      sha: "c".repeat(40),
      path: "src/cache.rs",
    });
    expect(
      originalSource({ ...anchor, side: "old", baseSha: null }),
    ).toBeNull();
    expect(
      originalSource({ ...anchor, side: "old", baseSha: "d".repeat(40) }),
    ).toEqual({ sha: "d".repeat(40), path: "src/cache.rs" });
  });
});

describe("buildSuggestion", () => {
  const spec = parseSuggestionInfo("suggestion")!;

  it("GitHub single line: the commented line out, the body in", () => {
    expect(
      buildSuggestion(spec, "let three = 30;", anchor, "github", FILE),
    ).toEqual({
      range: { start: 4, end: 4 },
      removed: ["let three = 3;"],
      added: ["let three = 30;"],
    });
  });

  it("GitHub multi-line range", () => {
    expect(
      buildSuggestion(
        spec,
        "let sum = 5;",
        { ...anchor, startLine: 2 },
        "github",
        FILE,
      ),
    ).toEqual({
      range: { start: 2, end: 4 },
      removed: ["let one = 1;", "let two = 2;", "let three = 3;"],
      added: ["let sum = 5;"],
    });
  });

  it("an empty suggestion deletes the lines", () => {
    const got = buildSuggestion(spec, "", anchor, "github", FILE);
    expect(got.removed).toEqual(["let three = 3;"]);
    expect(got.added).toEqual([]);
  });

  it("GitLab -N+M around the anchored line", () => {
    expect(
      buildSuggestion(
        parseSuggestionInfo("suggestion:-1+1")!,
        "let middle = 2;",
        { ...anchor, lineNew: 3 },
        "gitlab",
        FILE,
      ),
    ).toEqual({
      range: { start: 2, end: 4 },
      removed: ["let one = 1;", "let two = 2;", "let three = 3;"],
      added: ["let middle = 2;"],
    });
  });

  it("falls back to GitHub's diff hunk without the file", () => {
    const withHunk = {
      ...anchor,
      diffHunk:
        "@@ -2,2 +2,3 @@\n let one = 1;\n+let two = 2;\n let three = 3;",
    };
    expect(
      buildSuggestion(spec, "x", withHunk, "github", null).removed,
    ).toEqual(["let three = 3;"]);
  });

  it("says when the original lines are unavailable", () => {
    // No file, no hunk.
    expect(buildSuggestion(spec, "x", anchor, "gitlab", null)).toEqual({
      range: { start: 4, end: 4 },
      removed: null,
      added: ["x"],
    });
    // A file shorter than the range.
    expect(
      buildSuggestion(spec, "x", { ...anchor, lineNew: 40 }, "github", FILE)
        .removed,
    ).toBeNull();
    // No code reference at all.
    expect(buildSuggestion(spec, "x", null, "github")).toEqual({
      range: null,
      removed: null,
      added: ["x"],
    });
  });
});
