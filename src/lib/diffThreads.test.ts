import { Text } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { thread } from "../test/fixtures";
import {
  collapsedRanges,
  conversationThreads,
  fitPlacements,
  isConversationThread,
  mapOldLine,
  placeThreads,
  threadCounts,
  threadsOnFile,
} from "./diffThreads";

const anchor = thread().anchor!;

describe("conversationThreads", () => {
  const general = thread({ anchor: null, createdAt: "2026-10-10T00:00:00Z" });
  const mineGeneral = thread({
    anchor: null,
    mine: true,
    createdAt: "2026-10-11T00:00:00Z",
  });
  const pathless = thread({
    anchor: { ...anchor, pathNew: null, pathOld: null },
    createdAt: "2026-10-09T00:00:00Z",
  });
  const onCache = thread({ anchor: { ...anchor, pathNew: "src/cache.rs" } });
  const away = thread({
    anchor: { ...anchor, pathNew: "src/gone.rs", pathOld: "src/gone.rs" },
  });
  const all = [onCache, general, away, mineGeneral, pathless];
  const files = [{ path: "src/cache.rs", oldPath: null }];

  it("lists threads without a code reference, mine first", () => {
    expect(isConversationThread(general)).toBe(true);
    expect(isConversationThread(pathless)).toBe(true);
    expect(isConversationThread(onCache)).toBe(false);
    const got = conversationThreads(all, files, false);
    expect(got.conversation).toEqual([mineGeneral, pathless, general]);
    // Anchored to a file the list does not show: kept, not dropped.
    expect(got.elsewhere).toEqual([away]);
  });

  it("lists a thread the user replied in with the user's own", () => {
    // Carol's, older than the user's own and with a reply from the user.
    const replied = thread({
      anchor: null,
      author: "carol-fixture",
      mine: true,
      startedByMe: false,
      createdAt: "2026-10-08T00:00:00Z",
    });
    const got = conversationThreads([general, mineGeneral, replied], [], false);
    expect(got.conversation).toEqual([replied, mineGeneral, general]);
  });

  it("holds the anchored ones back while loading, shows them all on failure", () => {
    expect(conversationThreads(all, null, false).elsewhere).toEqual([]);
    expect(conversationThreads(all, null, true).elsewhere).toEqual([
      onCache,
      away,
    ]);
  });
});

describe("threadsOnFile", () => {
  it("matches either side of a rename and skips MR-level threads", () => {
    const onNew = thread({ anchor: { ...anchor, pathNew: "src/new.rs" } });
    const onOld = thread({
      anchor: { ...anchor, pathOld: "src/old.rs", pathNew: "src/old.rs" },
    });
    const elsewhere = thread({
      anchor: { ...anchor, pathNew: "x.rs", pathOld: "x.rs" },
    });
    const general = thread({ anchor: null });
    const got = threadsOnFile([onNew, onOld, elsewhere, general], {
      path: "src/new.rs",
      oldPath: "src/old.rs",
    });
    expect(got).toEqual([onNew, onOld]);
    expect(threadCounts([onNew, { ...onOld, mine: true }])).toEqual({
      total: 2,
      mine: 1,
    });
    // One the user replied in is highlighted as theirs too.
    expect(
      threadCounts([onNew, { ...onOld, mine: true, startedByMe: false }]),
    ).toEqual({ total: 2, mine: 1 });
  });
});

describe("placeThreads", () => {
  it("puts current threads at their line, outdated ones in the section", () => {
    const current = thread({
      createdAt: "2026-10-15T10:00:00Z",
      anchor: { ...anchor, lineNew: 31, currentLine: 33 },
    });
    const old = thread({
      createdAt: "2026-10-15T09:00:00Z",
      anchor: { ...anchor, side: "old", lineOld: 4, lineNew: null },
    });
    const outdated = thread({ outdated: true });
    const fileLevel = thread({
      anchor: { ...anchor, lineNew: null, lineOld: null },
    });
    const got = placeThreads([current, old, outdated, fileLevel], {
      since: false,
    });
    // GitHub's current line wins over the original one; oldest first.
    expect(got.inline).toEqual([
      { thread: old, side: "a", line: 4 },
      { thread: current, side: "b", line: 33 },
    ]);
    expect(got.outdated).toEqual([outdated]);
    expect(got.unplaced).toEqual([fileLevel]);

    // Since the last review the left side is another commit.
    const since = placeThreads([current, old], { since: true });
    expect(since.inline.map((p) => p.thread)).toEqual([current]);
    expect(since.unplaced).toEqual([old]);
  });

  it("keeps placements that fit their side", () => {
    const t = thread();
    const { fit, rest } = fitPlacements(
      [
        { thread: t, side: "b", line: 3 },
        { thread: t, side: "a", line: 1 },
      ],
      { a: 0, b: 3 },
    );
    expect(fit).toEqual([{ thread: t, side: "b", line: 3 }]);
    expect(rest).toEqual([t]);
  });
});

describe("mapOldLine", () => {
  // a: 1 keep / 2 gone / 3 keep / 4 keep;  b: 1 keep / 2 new / 3 new / 3 keep / 4 keep
  const a = Text.of(["keep1", "gone", "keep3", "keep4"]);
  const b = Text.of(["keep1", "new1", "new2", "keep3", "keep4"]);
  const chunk = {
    fromA: a.line(2).from,
    toA: a.line(3).from,
    fromB: b.line(2).from,
    toB: b.line(4).from,
  };

  it("maps unchanged lines across the change and removed ones to its end", () => {
    expect(mapOldLine([chunk], a, b, 1)).toBe(1);
    expect(mapOldLine([chunk], a, b, 2)).toBe(3);
    expect(mapOldLine([chunk], a, b, 3)).toBe(4);
    expect(mapOldLine([chunk], a, b, 4)).toBe(5);
  });
});

describe("collapsedRanges", () => {
  it("mirrors the merge view's collapsed stretches", () => {
    const doc = Text.of(Array.from({ length: 30 }, (_, i) => `l${i + 1}`));
    // One change on line 15.
    const c = {
      fromA: doc.line(15).from,
      toA: doc.line(16).from,
      fromB: doc.line(15).from,
      toB: doc.line(16).from,
    };
    const ranges = collapsedRanges([c], doc, "b", 3, 4);
    expect(
      ranges.map((r) => [doc.lineAt(r.from).number, doc.lineAt(r.to).number]),
    ).toEqual([
      [1, 11],
      [19, 30],
    ]);
  });
});
