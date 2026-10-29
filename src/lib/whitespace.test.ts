import { describe, expect, it } from "vitest";

import { file } from "../test/fixtures";
import {
  fileWhitespaceStats,
  onlyWhitespaceDiffers,
  patchWhitespaceStats,
  shownLineStats,
  unmatchedLines,
  whitespaceKey,
} from "./whitespace";

describe("whitespace", () => {
  it("keys a line without any whitespace", () => {
    expect(whitespaceKey("\t if (a  ==b) {\r")).toBe("if(a==b){");
  });

  it("tells whitespace-only differences apart", () => {
    expect(onlyWhitespaceDiffers("a\n  b\n", "a\n\tb  \n")).toBe(true);
    expect(onlyWhitespaceDiffers("a\r\nb", "a\nb")).toBe(true);
    expect(onlyWhitespaceDiffers("a\nb", "a\nb")).toBe(false);
    expect(onlyWhitespaceDiffers("a\nb", "a\nc")).toBe(false);
    // A blank line added is a change (git diff -w shows it).
    expect(onlyWhitespaceDiffers("a\nb", "a\n\nb")).toBe(false);
  });

  it("counts the lines a longest common subsequence leaves", () => {
    expect(unmatchedLines(["a", "b", "c"], ["a", "x", "c", "d"])).toEqual({
      removed: 1,
      added: 2,
    });
    expect(unmatchedLines([], ["a"])).toEqual({ removed: 0, added: 1 });
  });

  it("counts a patch's changes with whitespace hidden", () => {
    const patch = [
      "@@ -1,4 +1,5 @@",
      " fn main() {",
      "-  run();",
      "-  stop(1);",
      "+    run();",
      "+    stop(2);",
      "+    log();",
      " }",
      "@@ -10,2 +11,2 @@",
      "-x = 1;  ",
      "+x = 1;",
      "\\ No newline at end of file",
    ].join("\n");
    // run() is reindented only; stop changed; log() is new; x only trailing.
    expect(patchWhitespaceStats(patch)).toEqual({ additions: 2, deletions: 1 });
  });

  it("marks a file whose patch is whitespace only", () => {
    const f = file({
      additions: 2,
      deletions: 2,
      patch: "@@ -1,2 +1,2 @@\n-\tfoo();\n-bar(); \n+    foo();\n+bar();",
    });
    expect(fileWhitespaceStats(f)).toEqual({
      additions: 0,
      deletions: 0,
      whitespaceOnly: true,
    });
    expect(shownLineStats(f, true)).toEqual({ additions: 0, deletions: 0 });
    expect(shownLineStats(f, false)).toEqual({ additions: 2, deletions: 2 });
  });

  it("leaves files without a patch, added files and pure renames alone", () => {
    expect(fileWhitespaceStats(file({ patch: null }))).toBeNull();
    expect(
      fileWhitespaceStats(file({ status: "added", patch: "@@ -0,0 +1 @@\n+ " }))
        ?.whitespaceOnly,
    ).toBe(false);
    expect(
      fileWhitespaceStats(
        file({ status: "renamed", additions: 0, deletions: 0, patch: "" }),
      ),
    ).toBeNull();
    const big = file({ patch: null, additions: 9, deletions: 3 });
    expect(shownLineStats(big, true)).toEqual({ additions: 9, deletions: 3 });
  });
});
