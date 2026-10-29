import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";
import { answerBatches } from "../test/batches";

import { registerDiffEditor } from "../lib/diffEditors";
import { CONVERSATION_CARD } from "../lib/focusModel";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { clearContentsCache, useCodeViewStore } from "./codeViewStore";
import { resetFindForTests, useFindStore } from "./findStore";
import { useFocusStore } from "./focusStore";
import { useHistoryStore } from "./historyStore";
import { useReviewStore } from "./reviewStore";
import { useThreadStore } from "./threadStore";

const mockInvoke = answerBatches(vi.mocked(invoke));
const r = review();
const a = file({ path: "src/a.rs" });
const b = file({ path: "src/b.rs" });
const bin = file({ path: "logo.png", binary: true, patch: null });
const big = file({ path: "gen/big.rs", additions: 3000, deletions: 0 });

/** File contents by sha + path; each call waits until `release`d. */
const contents: Record<string, string> = {};
let held: (() => void)[] = [];
let hold = false;

function release() {
  const all = held;
  held = [];
  for (const go of all) go();
}

beforeEach(() => {
  resetFindForTests();
  clearContentsCache();
  hold = false;
  held = [];
  const files = reviewFiles({ reviewId: r.id, files: [a, b, bin, big] });
  contents[`${files.baseSha}|src/a.rs`] = "keep\nold needle\nkeep";
  contents[`${files.headSha}|src/a.rs`] = "keep\nnew needle\nkeep";
  contents[`${files.baseSha}|src/b.rs`] = "";
  contents[`${files.headSha}|src/b.rs`] = "needle in b";
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd !== "file_contents") return Promise.reject(new Error(cmd));
    const { sha, path } = args as { sha: string; path: string };
    const text = contents[`${sha}|${path}`] ?? "";
    if (!hold) return Promise.resolve(text);
    return new Promise((resolve) => held.push(() => resolve(text)));
  });
  useReviewStore.setState({ reviews: [r], selectedId: r.id });
  useCodeViewStore.setState({
    byReview: {
      [r.id]: {
        since: false,
        files,
        loading: false,
        error: null,
        selected: null,
        order: [CONVERSATION_CARD, a.path, b.path, bin.path, big.path],
      },
    },
  });
  useThreadStore.setState({
    byReview: {
      [r.id]: {
        threads: [
          thread({
            reviewId: r.id,
            anchor: null,
            notes: [
              {
                id: "n1",
                author: "carol-fixture",
                body: "A needle in the conversation",
                createdAt: "2026-10-15T09:00:00Z",
                system: false,
              },
            ],
          }),
        ],
        loading: false,
        error: null,
      },
    },
  });
});

afterEach(() => {
  act(() => useFindStore.getState().close({ restoreFocus: false }));
});

/** The query's search ran to its end. */
async function settle() {
  await vi.waitFor(() => {
    const s = useFindStore.getState();
    expect(s.re).not.toBeNull();
    expect(s.searching).toBe(false);
  });
}

describe("findStore", () => {
  it("loads the files it has not seen, says how far it is, and lists what it skips", async () => {
    hold = true;
    act(() => {
      useFindStore.getState().openBar();
      useFindStore.getState().setQuery("needle");
    });
    // The conversation is searched at once; the code waits for its files.
    await vi.waitFor(() =>
      expect(useFindStore.getState().matches.map((m) => m.kind)).toEqual([
        "note",
      ]),
    );
    const s = useFindStore.getState();
    expect(s.searching).toBe(true);
    expect(s.progress).toEqual({ done: 0, total: 2 });
    expect(s.skipped.map((x) => [x.path, x.reason])).toEqual([
      ["logo.png", "binary file"],
      ["gen/big.rs", "large diff (3000 lines)"],
    ]);
    // Only the searchable files were asked for.
    const asked = mockInvoke.mock.calls
      .filter(([cmd]) => cmd === "file_contents")
      .map(([, args]) => (args as { path: string }).path);
    expect(new Set(asked)).toEqual(new Set(["src/a.rs", "src/b.rs"]));
    await act(async () => release());
    await settle();
    const done = useFindStore.getState();
    expect(done.progress).toEqual({ done: 2, total: 2 });
    expect(
      done.matches.map((m) =>
        m.kind === "code" ? `${m.card}:${m.side}:${m.line}` : m.kind,
      ),
    ).toEqual(["note", "src/a.rs:old:2", "src/a.rs:new:2", "src/b.rs:new:1"]);
    // Jumped to the first match from the top of the page.
    expect(done.current).toBe(0);
  });

  it("reports an invalid regex and searches nothing", async () => {
    act(() => {
      useFindStore.getState().openBar();
      useFindStore.getState().setOption("regex", true);
      useFindStore.getState().setQuery("(oops");
    });
    await vi.waitFor(() =>
      expect(useFindStore.getState().error).toMatch(/^Invalid regex/),
    );
    expect(useFindStore.getState().matches).toEqual([]);
  });

  it("goes to a match: focuses its item, opens its card, reveals its line; wraps", async () => {
    const revealLine = vi.fn(() => true);
    const off = registerDiffEditor(a.path, {
      reveal: () => false,
      revealLine,
    });
    act(() => {
      useFindStore.getState().openBar();
      useFindStore.getState().setQuery("needle");
    });
    await settle();
    expect(useFindStore.getState().current).toBe(0);

    act(() => void useFindStore.getState().next());
    expect(useFindStore.getState().current).toBe(1);
    expect(useFocusStore.getState().item).toEqual({
      kind: "line",
      path: a.path,
      side: "old",
      line: 2,
    });
    // Its card selected without the view's own scroll, and opened.
    const code = useCodeViewStore.getState().byReview[r.id];
    expect(code.selected).toBe(a.path);
    expect(code.quietly).toBe(a.path);
    expect(useFocusStore.getState().reveal?.card).toBe(a.path);
    await vi.waitFor(() => expect(revealLine).toHaveBeenCalledWith("old", 2));

    // Back to the conversation's note: its thread focused and opened.
    act(() => void useFindStore.getState().prev());
    const item = useFocusStore.getState().item;
    expect(item?.kind).toBe("thread");
    const t = useThreadStore.getState().byReview[r.id].threads[0];
    expect(useHistoryStore.getState().open[t.threadId]).toBe(true);

    // From the first, previous wraps to the last.
    const wraps = useFindStore.getState().wrapSeq;
    act(() => void useFindStore.getState().prev());
    expect(useFindStore.getState().current).toBe(3);
    expect(useFindStore.getState().wrapSeq).toBe(wraps + 1);
    off();
  });

  it("keeps the current match while the page is re-read", async () => {
    act(() => {
      useFindStore.getState().openBar();
      useFindStore.getState().setQuery("needle");
    });
    await settle();
    act(() => void useFindStore.getState().next());
    act(() => void useFindStore.getState().next());
    const before = useFindStore.getState().matches[2];
    // A new thread arrives at the top of the page.
    act(() => {
      const cur = useThreadStore.getState().byReview[r.id];
      useThreadStore.setState({
        byReview: {
          [r.id]: {
            ...cur,
            threads: [
              ...cur.threads,
              thread({
                reviewId: r.id,
                anchor: null,
                notes: [
                  {
                    id: "n2",
                    author: "dave-fixture",
                    body: "another needle",
                    createdAt: "2026-10-16T09:00:00Z",
                    system: false,
                  },
                ],
              }),
            ],
          },
        },
      });
    });
    await vi.waitFor(() =>
      expect(useFindStore.getState().matches).toHaveLength(5),
    );
    const s = useFindStore.getState();
    expect(s.matches[s.current]).toMatchObject({
      unit: before.unit,
      line: before.line,
      start: before.start,
    });
  });

  it("follows the scope menu", async () => {
    act(() => {
      useFindStore.getState().openBar();
      useFindStore.getState().setQuery("needle");
    });
    await settle();
    act(() => useFindStore.getState().setScope("comments"));
    await vi.waitFor(() =>
      expect(useFindStore.getState().matches.map((m) => m.kind)).toEqual([
        "note",
      ]),
    );
    act(() => useFindStore.getState().setScope("code"));
    await vi.waitFor(() =>
      expect(
        useFindStore.getState().matches.every((m) => m.kind === "code"),
      ).toBe(true),
    );
  });
});
