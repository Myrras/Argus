import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { StoredThread, ThreadHistory } from "../ipc/types";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import { clearHistoryReads, useHistoryStore } from "../state/historyStore";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { review, thread } from "../test/fixtures";
import {
  FIX_SHA,
  HEAD_SHA,
  historyFor,
  THEN_SHA,
} from "../test/historyFixtures";
import { ListedThread, ThreadBlock } from "./ThreadBlock";

// jsdom has no layout: CodeMirror asks ranges for their rectangles.
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () =>
    ({
      length: 0,
      item: () => null,
      [Symbol.iterator]: [][Symbol.iterator],
    }) as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

const mockInvoke = vi.mocked(invoke);
const initialHistory = useHistoryStore.getState();
const initialThreads = useThreadStore.getState();
const initialReviews = useReviewStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");
const r = review();

function mine(overrides: Partial<StoredThread> = {}) {
  return thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    ...overrides,
  });
}

function theirs(overrides: Partial<StoredThread> = {}) {
  return thread({ reviewId: r.id, author: "bob-fixture", ...overrides });
}

/** Seed the store as if `review_histories` returned these. */
function seed(...histories: ThreadHistory[]) {
  useHistoryStore.setState({
    byReview: {
      [r.id]: { headSha: HEAD_SHA, loading: false, loaded: true, error: null },
    },
    byThread: Object.fromEntries(
      histories.map((h) => [
        h.threadId,
        { history: h, loading: false, error: null },
      ]),
    ),
  });
}

function show(t: StoredThread) {
  return render(<ThreadBlock thread={t} provider="github" now={NOW} />);
}

beforeEach(() => {
  mockInvoke.mockReset();
  clearHistoryReads();
  useHistoryStore.setState(
    {
      ...initialHistory,
      byReview: {},
      byThread: {},
      expanded: {},
      open: {},
      focused: null,
    },
    true,
  );
  useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
  useReviewStore.setState(
    { ...initialReviews, reviews: [r], selectedId: r.id },
    true,
  );
});

describe("then/now", () => {
  it("shows the user's own thread's code then and now without a click", () => {
    const t = mine();
    seed(historyFor(t));
    show(t);
    const block = screen.getByRole("region", { name: "Code then and now" });
    expect(
      within(block).getByText("Untouched since your comment"),
    ).toBeInTheDocument();
    const then = within(block).getByLabelText("Code then");
    expect(within(then).getByText(THEN_SHA.slice(0, 8))).toBeInTheDocument();
    expect(within(then).getByText("src/cache.rs:41")).toBeInTheDocument();
    const now = within(block).getByLabelText("Code now");
    expect(within(now).getByText("src/cache.rs:41")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Then/now history" }),
    ).toHaveAttribute("aria-expanded", "true");
  });

  it("opens a colleague's thread the user replied in like their own", () => {
    const t = theirs({
      mine: true,
      startedByMe: false,
      notes: [
        {
          id: "n-bob",
          author: "bob-fixture",
          body: "Is this cached twice?",
          createdAt: "2026-10-15T09:00:00Z",
          system: false,
        },
        {
          id: "n-me",
          author: "rev-fixture",
          body: "Looks like it.",
          createdAt: "2026-10-15T10:00:00Z",
          system: false,
        },
      ],
    });
    seed(historyFor(t));
    show(t);
    const article = screen.getByRole("article", {
      name: "Thread by bob-fixture",
    });
    expect(article).toHaveAttribute("data-mine", "true");
    expect(article).toHaveAttribute("data-open", "true");
    expect(within(article).getByText("Looks like it.")).toBeInTheDocument();
    const block = screen.getByRole("region", { name: "Code then and now" });
    // The code is tracked from bob's comment, not the user's.
    expect(
      within(block).getByText("Untouched since the comment"),
    ).toBeInTheDocument();
  });

  it("keeps someone else's thread collapsed until the toggle", () => {
    const t = theirs();
    seed(historyFor(t));
    show(t);
    expect(
      screen.queryByRole("region", { name: "Code then and now" }),
    ).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Then/now history" }));
    expect(
      screen.getByRole("region", { name: "Code then and now" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Untouched since the comment")).toBeInTheDocument();
  });

  it("marks the changed and the commented lines in the code", async () => {
    const t = mine();
    seed(
      historyFor(t, {
        diffLines: [
          {
            kind: "changed",
            then: { line: 41, text: "let key = raw;", anchored: true },
            now: { line: 43, text: "let key = hash(raw);", anchored: true },
          },
          {
            kind: "added",
            then: null,
            now: { line: 44, text: "log(key);", anchored: false },
          },
        ],
      }),
    );
    const { container } = show(t);
    await waitFor(() =>
      expect(container.querySelectorAll(".cm-line")).toHaveLength(4),
    );
    const then = screen.getByLabelText("Code then");
    const now = screen.getByLabelText("Code now");
    const kinds = (el: HTMLElement) =>
      [...el.querySelectorAll(".cm-line")].map(
        (l) =>
          `${l.getAttribute("data-kind")}${
            l.getAttribute("data-anchored") === "true" ? "*" : ""
          }`,
      );
    expect(kinds(then)).toEqual(["del*", "pad"]);
    expect(kinds(now)).toEqual(["add*", "add"]);
    expect(now.textContent).toContain("let key = hash(raw);");
  });

  it.each<[string, ThreadHistory["status"], string]>([
    [
      "moved",
      {
        kind: "located",
        path: "src/cache.rs",
        startLine: 47,
        line: 47,
        moved: true,
        renamed: false,
        touched: true,
      },
      "Moved from L41 to L47 · touched by 1 commit",
    ],
    [
      "deleted",
      { kind: "deleted", path: "src/cache.rs", removedBy: FIX_SHA },
      "The commented lines were removed in abcdef01 Drop the raw key",
    ],
    [
      "fileGone",
      { kind: "fileGone", path: "src/cache.rs", removedBy: FIX_SHA },
      "The file was deleted in abcdef01 Drop the raw key",
    ],
    [
      "unlocated",
      { kind: "unlocated", reason: "the branch was rebased" },
      "Could not locate the lines now: the branch was rebased",
    ],
  ])("says what happened when %s", (kind, status, message) => {
    const t = mine();
    const gone = kind !== "moved";
    seed(
      historyFor(t, {
        status,
        touchedBy: [FIX_SHA],
        nowSnapshot: gone ? null : historyFor(t).nowSnapshot,
        diffLines: gone ? [] : historyFor(t).diffLines,
        timeline: [
          ...historyFor(t).timeline,
          {
            at: "2026-10-15T12:00:00Z",
            kind: "commit",
            sha: FIX_SHA,
            title: "Drop the raw key",
            author: "alice-fixture",
            change: { kind: kind === "fileGone" ? "fileGone" : "modified" },
          },
        ],
      }),
    );
    show(t);
    expect(screen.getByRole("status")).toHaveTextContent(message);
  });

  it("never shows a line for an unlocated thread", () => {
    const t = mine();
    seed(
      historyFor(t, {
        status: { kind: "unlocated", reason: "the branch was rebased" },
      }),
    );
    show(t);
    const now = screen.getByLabelText("Code now");
    expect(within(now).getByText("Not located.")).toBeInTheDocument();
    expect(now.textContent).not.toMatch(/\d/);
    expect(now.textContent).not.toContain("let key");
  });

  it("keeps an outdated thread fully readable", () => {
    const t = theirs({
      outdated: true,
      anchor: { ...thread().anchor!, diffHunk: "@@ -1 +1 @@\n+old line" },
    });
    seed(historyFor(t, { outdated: true }));
    render(<ListedThread thread={t} provider="github" now={NOW} />);
    expect(screen.getByText("Outdated")).toBeInTheDocument();
    // Folded, the platform's excerpt shows; the then/now replaces it.
    expect(screen.getByText("+old line")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Then/now history" }));
    expect(screen.queryByText("+old line")).toBeNull();
    expect(screen.getByLabelText("Code then")).toBeInTheDocument();
    expect(screen.getByLabelText("Timeline")).toHaveTextContent(
      t.notes[0].body,
    );
  });
});

describe("timeline", () => {
  it("interleaves replies and commits by time", () => {
    const t = mine();
    seed(
      historyFor(t, {
        timeline: [
          {
            at: "2026-10-14T09:00:00Z",
            kind: "note",
            noteId: "n1",
            author: "rev-fixture",
            body: "Hash the key.",
            system: false,
            root: true,
          },
          {
            at: "2026-10-14T10:00:00Z",
            kind: "note",
            noteId: "n2",
            author: "alice-fixture",
            body: "Fixed.",
            system: false,
            root: false,
          },
          {
            at: "2026-10-15T09:00:00Z",
            kind: "commit",
            sha: FIX_SHA,
            title: "Hash the cache key",
            author: "alice-fixture",
            change: { kind: "modified" },
          },
          {
            at: "2026-10-15T10:00:00Z",
            kind: "resolved",
            by: "alice-fixture",
          },
        ],
      }),
    );
    show(t);
    const items = within(screen.getByLabelText("Timeline")).getAllByRole(
      "listitem",
    );
    // The reply came before the commit that touched the lines.
    expect(items.map((li) => li.getAttribute("data-kind"))).toEqual([
      "note",
      "note",
      "commit",
      "resolved",
    ]);
    expect(items[1]).toHaveTextContent("Fixed.");
    expect(items[2]).toHaveTextContent(
      "abcdef01Hash the cache keyalice-fixture changed the lines",
    );
    expect(items[3]).toHaveTextContent("Resolved by alice-fixture");
  });

  it("folds a long note behind Show more", () => {
    const t = mine();
    seed(
      historyFor(t, {
        timeline: [
          {
            at: "2026-10-14T09:00:00Z",
            kind: "note",
            noteId: "n1",
            author: "rev-fixture",
            body: "A paragraph.\n\n".repeat(10),
            system: false,
            root: true,
          },
        ],
      }),
    );
    show(t);
    const more = screen.getByRole("button", { name: "Show more" });
    fireEvent.click(more);
    expect(more).toHaveTextContent("Show less");
  });

  it("shows a thread without code as its timeline only", () => {
    const t = mine({ anchor: null });
    seed(historyFor(t, { status: { kind: "notCode" }, thenSnapshot: null }));
    show(t);
    expect(
      screen.queryByRole("button", { name: "Then/now history" }),
    ).toBeNull();
    expect(
      screen.queryByRole("region", { name: "Code then and now" }),
    ).toBeNull();
    expect(screen.getByLabelText("Timeline")).toHaveTextContent(
      t.notes[0].body,
    );
  });
});

describe("loading and errors", () => {
  it("shows the plain notes while the history loads", () => {
    const t = mine();
    useHistoryStore.setState({
      byReview: {
        [r.id]: { headSha: null, loading: true, loaded: false, error: null },
      },
    });
    show(t);
    expect(
      screen.getByText("Loading the code then and now…"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Timeline")).toBeNull();
    expect(screen.getByText(t.notes[0].body)).toBeInTheDocument();
  });

  it("falls back to the plain notes on an error, with a retry", async () => {
    const t = mine();
    const err = { kind: "provider", message: "GitHub is unreachable" };
    useHistoryStore.setState({
      byReview: {
        [r.id]: { headSha: null, loading: false, loaded: false, error: err },
      },
    });
    mockInvoke.mockResolvedValue(historyFor(t));
    show(t);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not load the history: GitHub is unreachable",
    );
    expect(screen.getByText(t.notes[0].body)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mockInvoke).toHaveBeenCalledWith("thread_history", {
      threadId: t.threadId,
    });
    expect(await screen.findByLabelText("Timeline")).toBeInTheDocument();
  });
});

describe("the then/now key", () => {
  function Keys() {
    useAppShortcuts();
    return null;
  }

  it("toggles the thread last clicked", () => {
    const t = mine();
    seed(historyFor(t));
    render(
      <>
        <Keys />
        <ThreadBlock thread={t} provider="github" now={NOW} />
      </>,
    );
    const region = () =>
      screen.queryByRole("region", { name: "Code then and now" });
    // Nothing clicked yet: the key does nothing.
    fireEvent.keyDown(window, { key: "h", code: "KeyH" });
    expect(region()).not.toBeNull();
    fireEvent.pointerDown(screen.getByRole("article"));
    fireEvent.keyDown(window, { key: "h", code: "KeyH" });
    expect(region()).toBeNull();
    fireEvent.keyDown(window, { key: "h", code: "KeyH" });
    expect(region()).not.toBeNull();
  });
});
