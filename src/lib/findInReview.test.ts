import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { finding } from "../test/aiFixtures";
import { file, thread } from "../test/fixtures";
import {
  codeUnit,
  findStatus,
  firstFrom,
  noteUnit,
  prepareCode,
  searchReview,
  skipReason,
  threadUnit,
  type FindMatch,
  type PreparedCode,
  type ReviewPage,
} from "./findInReview";
import { compileFind, lineMatches, textMatches } from "./findQuery";
import { CONVERSATION_CARD } from "./focusModel";
import { markdownText } from "./markdownText";
import { editorDiffConfig } from "./whitespaceDiff";

const plain = { caseSensitive: false, wholeWord: false, regex: false };

function re(text: string, opts: Partial<typeof plain> = {}): RegExp {
  const c = compileFind(text, { ...plain, ...opts });
  if (!c.re) throw new Error(c.error ?? "empty");
  return c.re;
}

/** Prepared as the diff editor diffs it (`Chunk.build`). */
function code(
  original: string,
  modified: string,
  hideWhitespace = false,
): PreparedCode {
  const chunks = Chunk.build(
    Text.of(original.split("\n")),
    Text.of(modified.split("\n")),
    editorDiffConfig(hideWhitespace),
  );
  return prepareCode(original, modified, chunks);
}

function page(over: Partial<ReviewPage> = {}): ReviewPage {
  return {
    threads: [],
    files: [],
    failed: false,
    order: [CONVERSATION_CARD],
    since: false,
    findings: [],
    ...over,
  };
}

const brief = (m: FindMatch) =>
  m.kind === "code"
    ? `${m.card}:${m.side}:${m.line}:${m.start}`
    : `${m.kind}:${m.card === CONVERSATION_CARD ? "conv" : m.card}:${m.unit}:${m.start}`;

describe("compileFind", () => {
  it("ignores case unless asked", () => {
    expect(lineMatches("Cache cache CACHE", re("cache"))).toEqual([
      [0, 5],
      [6, 11],
      [12, 17],
    ]);
    expect(
      lineMatches("Cache cache CACHE", re("cache", { caseSensitive: true })),
    ).toEqual([[6, 11]]);
  });

  it("matches whole words only when asked, also around punctuation", () => {
    const line = "cache caches cache_key (cache) .cache";
    expect(lineMatches(line, re("cache", { wholeWord: true }))).toEqual([
      [0, 5],
      [24, 29],
      [32, 37],
    ]);
    // A query that ends in punctuation still works as a whole word.
    expect(lineMatches("a.b a.bc", re("a.b", { wholeWord: true }))).toEqual([
      [0, 3],
    ]);
  });

  it("takes the text literally unless Regex is on", () => {
    expect(lineMatches("a.b axb", re("a.b"))).toEqual([[0, 3]]);
    expect(lineMatches("a.b axb", re("a.b", { regex: true }))).toEqual([
      [0, 3],
      [4, 7],
    ]);
    expect(
      lineMatches("fn foo() fn bar()", re("fn \\w+", { regex: true })),
    ).toEqual([
      [0, 6],
      [9, 15],
    ]);
  });

  it("says why a regex is invalid, and finds nothing for an empty query", () => {
    const bad = compileFind("(unclosed", { ...plain, regex: true });
    expect(bad.re).toBeNull();
    expect(bad.error).toMatch(/^Invalid regex/);
    // The same text without Regex is fine.
    expect(compileFind("(unclosed", plain).error).toBeNull();
    expect(compileFind("", plain)).toEqual({ re: null, error: null });
  });

  it("skips empty matches and never crosses a line", () => {
    expect(lineMatches("abc", re("x*", { regex: true }))).toEqual([]);
    expect(lineMatches("aXa", re("a?", { regex: true }))).toEqual([
      [0, 1],
      [2, 3],
    ]);
    expect(textMatches("one\ntwo one", re("one"))).toEqual([
      [0, 3],
      [8, 11],
    ]);
    expect(textMatches("one\ntwo", re("one\\ntwo", { regex: true }))).toEqual(
      [],
    );
  });
});

describe("markdownText", () => {
  it("is the text a comment shows: no markers, HTML or hidden markers", () => {
    const body = [
      "**Bold** and [a link](https://example.test/x) with `code`.",
      "",
      "<!-- argus:reply-to 3f2c -->",
    ].join("\n");
    const text = markdownText(body);
    expect(text).toContain("Bold and a link with code.");
    expect(text).not.toContain("argus");
    expect(text).not.toContain("**");
    expect(text).not.toContain("example.test");
    // Raw HTML tags are markup, not text; the text between them counts.
    expect(markdownText("<b>bold</b> text")).toBe("bold text");
  });
});

describe("prepareCode", () => {
  it("knows the removed lines and where their block sits on the head", () => {
    const c = code("a\nold1\nold2\nb\nc", "a\nnew\nb\nc");
    expect(c.removed).toEqual([{ from: 2, to: 3, before: 2 }]);
    expect(c.newLines).toEqual(["a", "new", "b", "c"]);
    // Removed at the end: the last head line (no newline after it) is
    // part of the chunk, as the editor shows it.
    expect(code("a\nb\ngone", "a\nb").removed).toEqual([
      { from: 2, to: 3, before: 2 },
    ]);
  });
});

describe("prepareCode with whitespace hidden", () => {
  it("counts no removed lines for a whitespace-only change", () => {
    const before = "a\n  b\nc";
    const after = "a\n\tb \nc";
    expect(code(before, after).removed).toEqual([
      { from: 2, to: 2, before: 2 },
    ]);
    expect(code(before, after, true).removed).toEqual([]);
  });

  it("keeps the removed lines of a real change", () => {
    expect(code("a\n  b\nc", "a\nc", true).removed).toEqual([
      { from: 2, to: 2, before: 2 },
    ]);
  });
});

describe("searchReview", () => {
  const f1 = file({ path: "src/a.rs" });
  const f2 = file({ path: "src/b.rs" });
  const orderAll = [CONVERSATION_CARD, f1.path, f2.path];

  it("searches both sides: every head line and the removed base lines", () => {
    // `keep` is unchanged: on the head only, once.
    const c = code("keep\nfoo old\nkeep", "keep\nfoo new\nkeep");
    const got = searchReview(
      page({ files: [f1], order: [CONVERSATION_CARD, f1.path] }),
      new Map([[f1.path, c]]),
      re("foo|keep", { regex: true }),
      "everything",
    ).map(brief);
    expect(got).toEqual([
      "src/a.rs:new:1:0",
      // The removed line's block sits above its chunk's head line.
      "src/a.rs:old:2:0",
      "src/a.rs:new:2:0",
      "src/a.rs:new:3:0",
    ]);
  });

  it("finds lines in collapsed unchanged stretches (it reads the contents)", () => {
    const base = Array.from({ length: 200 }, (_, i) => `line ${i + 1}`);
    const head = [...base];
    head[0] = "changed first";
    base[149] = "needle far below the hunk";
    head[149] = "needle far below the hunk";
    const got = searchReview(
      page({ files: [f1], order: [CONVERSATION_CARD, f1.path] }),
      new Map([[f1.path, code(base.join("\n"), head.join("\n"))]]),
      re("needle"),
      "everything",
    );
    expect(got.map(brief)).toEqual(["src/a.rs:new:150:0"]);
  });

  it("walks the page in order: Conversation, then each file's listed items, code and threads by line", () => {
    const general = thread({
      anchor: null,
      notes: [
        {
          id: "g1",
          author: "carol-fixture",
          body: "needle in the conversation",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
      ],
    });
    const outdated = thread({
      outdated: true,
      anchor: { ...thread().anchor!, pathNew: f1.path, pathOld: f1.path },
      notes: [
        {
          id: "o1",
          author: "bob-fixture",
          body: "outdated needle",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
      ],
    });
    const inline = thread({
      anchor: {
        ...thread().anchor!,
        pathNew: f1.path,
        pathOld: f1.path,
        lineNew: 1,
      },
      notes: [
        {
          id: "i1",
          author: "bob-fixture",
          body: "inline needle",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
      ],
    });
    const listedFinding = finding({
      path: f2.path,
      line: 0,
      title: "needle finding",
      body: "body",
    });
    const got = searchReview(
      page({
        threads: [inline, outdated, general],
        files: [f1, f2],
        order: orderAll,
        findings: [
          { finding: listedFinding, title: listedFinding.title, body: "body" },
        ],
      }),
      new Map([
        [f1.path, code("x", "needle 1\nneedle 2")],
        [f2.path, code("", "needle")],
      ]),
      re("needle"),
      "everything",
    ).map(brief);
    expect(got).toEqual([
      `note:conv:${noteUnit(general.threadId, "g1")}:0`,
      `note:src/a.rs:${noteUnit(outdated.threadId, "o1")}:9`,
      "src/a.rs:new:1:0",
      // Under line 1.
      `note:src/a.rs:${noteUnit(inline.threadId, "i1")}:7`,
      "src/a.rs:new:2:0",
      // Listed at the top of its card: a finding without a line.
      `finding:src/b.rs:finding:title:${listedFinding.id}:0`,
      "src/b.rs:new:1:0",
    ]);
  });

  it("leaves out the code or the comments by scope, and searches authors", () => {
    const t = thread({
      author: "needle-fixture",
      anchor: null,
      notes: [
        {
          id: "n1",
          author: "needle-fixture",
          body: "a needle",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
      ],
    });
    const p = page({
      threads: [t],
      files: [f1],
      order: [CONVERSATION_CARD, f1.path],
    });
    const codes = new Map([[f1.path, code("", "needle")]]);
    const kinds = (scope: "everything" | "code" | "comments") =>
      searchReview(p, codes, re("needle"), scope).map((m) => m.kind);
    expect(kinds("everything")).toEqual(["thread", "note", "code"]);
    expect(kinds("code")).toEqual(["code"]);
    expect(kinds("comments")).toEqual(["thread", "note"]);
    expect(searchReview(p, codes, re("needle"), "comments")[0].unit).toBe(
      threadUnit(t.threadId),
    );
  });

  it("never finds Argus' hidden markers or raw HTML in a comment", () => {
    const t = thread({
      anchor: null,
      notes: [
        {
          id: "m1",
          author: "bob-fixture",
          body: "Thanks!\n\n<!-- argus:reply-to 1234 -->",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
      ],
    });
    const found = searchReview(
      page({ threads: [t] }),
      new Map(),
      re("argus"),
      "everything",
    );
    expect(found).toEqual([]);
  });

  it("numbers the matches of a unit for the highlighting", () => {
    const c = code("", "aa aa");
    const got = searchReview(
      page({ files: [f1], order: [CONVERSATION_CARD, f1.path] }),
      new Map([[f1.path, c]]),
      re("aa"),
      "everything",
    );
    expect(got.map((m) => [m.unit, m.ordinal, m.start])).toEqual([
      [codeUnit(f1.path, "new"), 0, 0],
      [codeUnit(f1.path, "new"), 1, 3],
    ]);
  });
});

describe("skipReason", () => {
  it("skips what the card shows no diff for", () => {
    expect(skipReason(file({ binary: true }), false)).toBe("binary file");
    expect(skipReason(file({ tooLarge: true }), true)).toMatch(/too large/);
    expect(skipReason(file({ additions: 1600, deletions: 0 }), true)).toBe(
      "large diff (1600 lines)",
    );
    expect(skipReason(file({ patch: null }), false)).toMatch(/no patch/);
    expect(skipReason(file(), false)).toBeNull();
  });
});

describe("firstFrom", () => {
  const order = [CONVERSATION_CARD, "a", "b"];
  const m = (card: string, row: number) =>
    ({ card, row }) as unknown as FindMatch;
  const list = [m(CONVERSATION_CARD, 0), m("a", 3), m("a", 9), m("b", 1)];

  it("picks the first match at or after the reading place, wrapping", () => {
    expect(firstFrom(list, order, null)).toBe(0);
    expect(firstFrom(list, order, { card: 1, row: 4 })).toBe(2);
    expect(firstFrom(list, order, { card: 2, row: -Infinity })).toBe(3);
    expect(firstFrom(list, order, { card: 2, row: 5 })).toBe(0);
    expect(firstFrom([], order, null)).toBe(-1);
  });
});

describe("findStatus", () => {
  const base = {
    re: /x/g,
    error: null,
    matches: [1, 2, 3],
    current: 0,
    capped: false,
    searching: false,
    progress: { done: 4, total: 4 },
  };
  it("says where you are, and what the search is doing", () => {
    expect(findStatus(base).text).toBe("1 of 3");
    expect(findStatus({ ...base, current: -1 }).text).toBe("3 found");
    expect(findStatus({ ...base, matches: [] }).text).toBe("No results");
    expect(
      findStatus({
        ...base,
        searching: true,
        progress: { done: 12, total: 40 },
      }).searching,
    ).toBe("Searching 12 of 40 files…");
    expect(findStatus({ ...base, error: "Invalid regex: x" }).text).toBe(
      "Invalid regex: x",
    );
    expect(findStatus({ ...base, capped: true }).text).toBe("1 of 3+");
  });
});
