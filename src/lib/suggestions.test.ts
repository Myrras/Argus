import { describe, expect, it } from "vitest";

import { thread } from "../test/fixtures";
import {
  buildSuggestion,
  commentSuggestionPlan,
  enterInSuggestion,
  findFences,
  indentUnit,
  insertSuggestion,
  replySuggestionPlan,
  retargetSuggestions,
  suggestionAt,
  suggestionFence,
  suggestionMismatch,
  tabInSuggestion,
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
    expect(parseSuggestionInfo("suggestion:x")).toBeNull();
    expect(parseSuggestionInfo("suggestion:+1-2")).toBeNull();
  });

  it("takes GitLab's one-sided offsets, as its SuggestionsParser does", () => {
    expect(parseSuggestionInfo("suggestion:+1")).toEqual({
      kind: "relative",
      above: 0,
      below: 1,
    });
    expect(parseSuggestionInfo("suggestion:-3")).toEqual({
      kind: "relative",
      above: 3,
      below: 0,
    });
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

/** `edit` applied to `body`. */
function apply(
  body: string,
  e: { start: number; end: number; text: string },
): string {
  return body.slice(0, e.start) + e.text + body.slice(e.end);
}

describe("insertSuggestion", () => {
  it("GitHub: a bare fence with the lines, content selected", () => {
    const e = insertSuggestion("", 0, "github", { start: 4, end: 4 }, [
      "let three = 3;",
    ]);
    const out = apply("", e);
    expect(out).toBe("```suggestion\nlet three = 3;\n```");
    expect(out.slice(e.select[0], e.select[1])).toBe("let three = 3;");
  });

  it("GitLab: -N+0 with N = range length - 1 (its own button's rule)", () => {
    const one = insertSuggestion("", 0, "gitlab", { start: 4, end: 4 }, ["x"]);
    expect(one.text).toBe("```suggestion:-0+0\nx\n```");
    const three = insertSuggestion("", 0, "gitlab", { start: 2, end: 4 }, [
      "a",
      "b",
      "c",
    ]);
    expect(three.text).toBe("```suggestion:-2+0\na\nb\nc\n```");
    // GitHub's fence names no range: the comment's start_line..line is it.
    expect(
      insertSuggestion("", 0, "github", { start: 2, end: 4 }, ["a", "b", "c"])
        .text,
    ).toBe("```suggestion\na\nb\nc\n```");
  });

  it("goes on lines of its own, mid-text", () => {
    const body = "Before this after";
    const e = insertSuggestion(body, 6, "github", { start: 1, end: 1 }, ["x"]);
    expect(apply(body, e)).toBe("Before\n```suggestion\nx\n```\n this after");
    const atLineEnd = "Line one\n";
    expect(
      apply(
        atLineEnd,
        insertSuggestion(atLineEnd, 9, "github", { start: 1, end: 1 }, ["x"]),
      ),
    ).toBe("Line one\n```suggestion\nx\n```");
  });

  it("takes a longer fence when the lines hold one", () => {
    const lines = ["/// ```rust", "/// let x = 1;", "/// ````"];
    expect(suggestionFence(lines)).toBe("`````");
    expect(suggestionFence(["no `ticks` here", "``two``"])).toBe("```");
    const e = insertSuggestion("", 0, "gitlab", { start: 1, end: 3 }, lines);
    expect(e.text.startsWith("`````suggestion:-2+0\n")).toBe(true);
    expect(e.text.endsWith("\n`````")).toBe(true);
    // Read back as one block, the inner fences its content.
    const fences = findFences(e.text);
    expect(fences).toHaveLength(1);
    expect(e.text.slice(fences[0]!.contentStart, fences[0]!.contentEnd)).toBe(
      lines.join("\n") + "\n",
    );
  });

  it("keeps tabs; CRLF and the file's last newline are gone (lines from linesFromContents)", () => {
    const file = "fn a() {\r\n\tlet x = 1;\r\n}\r\n";
    const lines = linesFromContents(file, { start: 1, end: 3 })!;
    expect(lines).toEqual(["fn a() {", "\tlet x = 1;", "}"]);
    const e = insertSuggestion("", 0, "github", { start: 1, end: 3 }, lines);
    expect(e.text).toBe("```suggestion\nfn a() {\n\tlet x = 1;\n}\n```");
    expect(e.text).not.toContain("\r");
  });
});

describe("findFences / suggestionAt", () => {
  const body = "Hi\n```suggestion\n  a\n  b\n```\nAfter\n```ts\nc\n```";

  it("finds suggestion and other fences, unclosed ones to the end", () => {
    const f = findFences(body);
    expect(f.map((x) => x.info)).toEqual(["suggestion", "ts"]);
    expect(f[0]!.spec).toEqual({ kind: "default" });
    expect(f[1]!.spec).toBeNull();
    const open = findFences("```suggestion:-1+0\nx\ny");
    expect(open).toHaveLength(1);
    expect(open[0]!.contentEnd).toBe("```suggestion:-1+0\nx\ny".length);
    // A ~~~ block hides the ``` inside it.
    expect(
      findFences("~~~\n```suggestion\nx\n```\n~~~").map((x) => x.info),
    ).toEqual([""]);
  });

  it("is inside on the content lines only", () => {
    const start = body.indexOf("  a");
    expect(suggestionAt(body, start)).not.toBeNull();
    expect(suggestionAt(body, body.indexOf("  b") + 3)).not.toBeNull();
    // The opening and closing fence lines, and other code, are outside.
    expect(suggestionAt(body, 5)).toBeNull();
    expect(suggestionAt(body, body.indexOf("```\nAfter"))).toBeNull();
    expect(suggestionAt(body, body.indexOf("c\n"))).toBeNull();
  });
});

describe("editing inside a suggestion", () => {
  const body = "```suggestion\n    if x {\n```";
  const end = body.indexOf("{") + 1;

  it("Enter keeps the line's indentation", () => {
    const e = enterInSuggestion(body, end, end)!;
    expect(apply(body, e)).toBe("```suggestion\n    if x {\n    \n```");
    expect(e.select).toEqual([end + 5, end + 5]);
    // Outside a suggestion Enter is the field's own.
    expect(enterInSuggestion("plain text", 5, 5)).toBeNull();
    expect(enterInSuggestion(body, 3, 3)).toBeNull();
  });

  it("Tab inserts the file's step; Shift+Tab takes one off", () => {
    const e = tabInSuggestion(body, end, end, "  ")!;
    expect(e.text).toBe("  ");
    const line = body.indexOf("    if");
    const out = tabInSuggestion(body, line + 2, line + 2, "    ", true)!;
    expect(apply(body, out)).toBe("```suggestion\nif x {\n```");
    expect(tabInSuggestion("no fence", 2, 2, "  ")).toBeNull();
  });

  it("Tab on a selection over lines indents each", () => {
    const b = "```suggestion\na\n\tb\nc\n```";
    const from = b.indexOf("a");
    const to = b.indexOf("c") + 1;
    expect(apply(b, tabInSuggestion(b, from, to, "\t")!)).toBe(
      "```suggestion\n\ta\n\t\tb\n\tc\n```",
    );
    expect(apply(b, tabInSuggestion(b, from, to, "\t", true)!)).toBe(
      "```suggestion\na\nb\nc\n```",
    );
  });

  it("reads the file's indentation", () => {
    expect(indentUnit("fn a() {\n\tlet x;\n\tif y {\n\t\tz;\n\t}\n}\n")).toBe(
      "\t",
    );
    expect(indentUnit("a:\n  b:\n    c: 1\n  d: 2\n")).toBe("  ");
    expect(indentUnit("class A {\n    f() {\n        g();\n    }\n}\n")).toBe(
      "    ",
    );
    expect(indentUnit("no indentation\n")).toBe("    ");
  });
});

describe("suggestionMismatch / retargetSuggestions", () => {
  const five = { start: 6, end: 10 };

  it("GitLab: offsets that do not cover the comment's lines", () => {
    const body = "```suggestion:-2+0\na\nb\nc\n```";
    expect(
      suggestionMismatch(body, "gitlab", { start: 8, end: 10 }),
    ).toBeNull();
    const m = suggestionMismatch(body, "gitlab", five)!;
    expect(m.message).toBe(
      "The suggestion covers 3 lines but the comment is on 5",
    );
    expect(m.covered).toEqual({ start: 8, end: 10 });
    // A bare fence on a multi-line GitLab comment is its last line only.
    expect(
      suggestionMismatch("```suggestion\nx\n```", "gitlab", five)?.message,
    ).toBe("The suggestion covers 1 line but the comment is on 5");
    // Same size, other lines.
    expect(
      suggestionMismatch("```suggestion:-1+1\nx\n```", "gitlab", {
        start: 9,
        end: 11,
      })?.message,
    ).toBe(
      "The suggestion covers lines 10–12 but the comment is on lines 9–11",
    );
  });

  it("GitLab: Update suggestion range rewrites every fence's offsets", () => {
    const body =
      "One:\n```suggestion:-2+0 extra\na\n```\nTwo:\n````suggestion\nb\n````";
    const fixed = retargetSuggestions(body, five);
    expect(fixed).toBe(
      "One:\n```suggestion:-4+0 extra\na\n```\nTwo:\n````suggestion:-4+0\nb\n````",
    );
    expect(suggestionMismatch(fixed, "gitlab", five)).toBeNull();
  });

  it("GitHub: only a range changed since inserting", () => {
    const body = "```suggestion\na\n```";
    expect(suggestionMismatch(body, "github", five)).toBeNull();
    expect(suggestionMismatch(body, "github", five, five)).toBeNull();
    expect(
      suggestionMismatch(body, "github", five, { start: 8, end: 10 })?.message,
    ).toBe("The suggestion covers 3 lines but the comment is on 5");
    // Offsets mean nothing to GitHub: no suggestion of its.
    expect(
      suggestionMismatch("```suggestion:-1+0\na\n```", "github", five),
    ).toBeNull();
    // No block, no warning.
    expect(
      suggestionMismatch("plain", "github", five, { start: 1, end: 1 }),
    ).toBeNull();
  });
});

describe("where a suggestion can be written", () => {
  const file = { binary: false, tooLarge: false, patch: "@@ -1 +1 @@" };
  const comment = {
    provider: "github" as const,
    path: "src/cache.rs",
    side: "new" as const,
    line: 4,
    startLine: 2,
    headSha: "h".repeat(40),
    outside: null,
    file,
  };

  it("a new comment on new-side lines, at the head", () => {
    expect(commentSuggestionPlan(comment)).toEqual({
      ok: true,
      path: "src/cache.rs",
      sha: "h".repeat(40),
      range: { start: 2, end: 4 },
    });
    // GitLab outside the diff takes a line comment: fine.
    expect(
      commentSuggestionPlan({
        ...comment,
        provider: "gitlab",
        outside: "native",
      }).ok,
    ).toBe(true);
  });

  it("not on the old side, GitHub outside the diff, or a file without a diff", () => {
    const reason = (p: ReturnType<typeof commentSuggestionPlan>) =>
      p.ok ? null : p.reason;
    expect(reason(commentSuggestionPlan({ ...comment, side: "old" }))).toMatch(
      /apply to the new code/,
    );
    expect(
      reason(commentSuggestionPlan({ ...comment, outside: "fallback" })),
    ).toMatch(/GitHub takes no suggestion outside the diff/);
    expect(
      reason(
        commentSuggestionPlan({ ...comment, file: { ...file, binary: true } }),
      ),
    ).toMatch(/binary/);
    expect(
      reason(
        commentSuggestionPlan({
          ...comment,
          file: { ...file, tooLarge: true },
        }),
      ),
    ).toMatch(/no diff/);
    expect(
      reason(
        commentSuggestionPlan({ ...comment, file: { ...file, patch: null } }),
      ),
    ).toMatch(/no diff/);
    expect(
      reason(
        commentSuggestionPlan({
          ...comment,
          provider: "gitlab",
          startLine: 1,
          line: 102,
        }),
      ),
    ).toMatch(/at most 101 lines/);
  });

  const t = thread();
  const head = "e".repeat(40);

  it("a GitHub reply: the thread's current lines at the head", () => {
    const th = {
      outdated: false,
      anchor: { ...t.anchor!, lineNew: 7, startLine: 5, currentLine: 12 },
    };
    expect(
      replySuggestionPlan({
        provider: "github",
        thread: th,
        headSha: head,
        file,
      }),
    ).toEqual({
      ok: true,
      path: "src/cache.rs",
      sha: head,
      range: { start: 10, end: 12 },
    });
  });

  it("a GitLab reply: the discussion's line_range at its (current) head", () => {
    const th = {
      outdated: false,
      anchor: { ...t.anchor!, lineNew: 7, startLine: 5, headSha: head },
    };
    expect(
      replySuggestionPlan({
        provider: "gitlab",
        thread: th,
        headSha: head,
        file,
      }),
    ).toEqual({
      ok: true,
      path: "src/cache.rs",
      sha: head,
      range: { start: 5, end: 7 },
    });
  });

  it("no reply suggestion on review-level, outdated, old-side or file threads", () => {
    const reason = (p: ReturnType<typeof replySuggestionPlan>) =>
      p.ok ? null : p.reason;
    const base = { provider: "github" as const, headSha: head, file };
    expect(
      reason(
        replySuggestionPlan({
          ...base,
          thread: { outdated: false, anchor: null },
        }),
      ),
    ).toMatch(/whole review/);
    expect(
      reason(
        replySuggestionPlan({
          ...base,
          thread: { outdated: true, anchor: t.anchor },
        }),
      ),
    ).toMatch(/outdated/);
    expect(
      reason(
        replySuggestionPlan({
          ...base,
          thread: { outdated: false, anchor: { ...t.anchor!, side: "old" } },
        }),
      ),
    ).toMatch(/new code/);
    expect(
      reason(
        replySuggestionPlan({
          ...base,
          thread: {
            outdated: false,
            anchor: { ...t.anchor!, outsideDiff: true },
          },
        }),
      ),
    ).toMatch(/outside the diff/);
    expect(
      reason(
        replySuggestionPlan({
          ...base,
          thread: { outdated: false, anchor: { ...t.anchor!, lineNew: null } },
        }),
      ),
    ).toMatch(/whole file/);
    // GitHub without a current line: outdated.
    expect(
      reason(
        replySuggestionPlan({
          ...base,
          thread: {
            outdated: false,
            anchor: { ...t.anchor!, currentLine: null },
          },
        }),
      ),
    ).toMatch(/outdated/);
  });
});
