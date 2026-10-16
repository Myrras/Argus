import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
// CodeMirror needs a real layout engine; the wrapper is smoke-tested on its
// own. The stub shows what it was given.
vi.mock("./DiffEditor", () => ({
  DiffEditor: (p: {
    path: string;
    layout: string;
    original: string;
    modified: string;
    placements: { line: number; side: string }[];
  }) => (
    <div
      data-testid={`editor-${p.path}`}
      data-layout={p.layout}
      data-placements={p.placements.map((x) => `${x.side}${x.line}`).join(",")}
    >
      {p.original}|{p.modified}
    </div>
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import type { FileEntry, ReviewFiles } from "../ipc/types";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import {
  clearContentsCache,
  CONVERSATION,
  useCodeViewStore,
} from "../state/codeViewStore";
import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { ReviewView } from "./ReviewView";

const mockInvoke = vi.mocked(invoke);
const initialCode = useCodeViewStore.getState();
const initialThreads = useThreadStore.getState();
const initialReviews = useReviewStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");

const r = review({ headSha: "a".repeat(40) });
const anchor = thread().anchor!;

const files: FileEntry[] = [
  file({ path: "src/cache.rs", additions: 3, deletions: 1 }),
  file({ path: "src/widget.rs", status: "added", additions: 10, deletions: 0 }),
  file({
    path: "assets/logo.png",
    binary: true,
    additions: 0,
    deletions: 0,
    patch: null,
  }),
  file({
    path: "data/big.json",
    tooLarge: true,
    additions: 4000,
    deletions: 0,
    patch: null,
  }),
];

const threads = [
  thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    anchor: {
      ...anchor,
      pathNew: "src/cache.rs",
      pathOld: "src/cache.rs",
      lineNew: 2,
    },
  }),
  thread({
    reviewId: r.id,
    author: "bob-fixture",
    anchor: {
      ...anchor,
      pathNew: "src/cache.rs",
      pathOld: "src/cache.rs",
      lineNew: 1,
    },
  }),
  thread({
    reviewId: r.id,
    author: "bob-fixture",
    outdated: true,
    anchor: {
      ...anchor,
      pathNew: "src/widget.rs",
      pathOld: "src/widget.rs",
      lineNew: 9,
      diffHunk: "@@ -0,0 +1,9 @@\n+fn gone() {}",
    },
    notes: [
      {
        id: "n-out",
        author: "bob-fixture",
        body: "This was about the old version.",
        createdAt: "2026-10-14T09:00:00Z",
        system: false,
      },
    ],
  }),
  // On the review as a whole: someone else's first, then the user's.
  thread({
    reviewId: r.id,
    author: "carol-fixture",
    createdAt: "2026-10-12T09:00:00Z",
    anchor: null,
    notes: [
      {
        id: "n-carol",
        author: "carol-fixture",
        body: "Overall this looks fine.",
        createdAt: "2026-10-12T09:00:00Z",
        system: false,
      },
    ],
  }),
  thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    createdAt: "2026-10-13T09:00:00Z",
    anchor: null,
    notes: [
      {
        id: "n-mine-pr",
        author: "rev-fixture",
        body: "Please add a changelog entry.",
        createdAt: "2026-10-13T09:00:00Z",
        system: false,
      },
    ],
  }),
  // On a file outside this diff.
  thread({
    reviewId: r.id,
    author: "bob-fixture",
    anchor: {
      ...anchor,
      pathNew: "src/elsewhere.rs",
      pathOld: "src/elsewhere.rs",
      lineNew: 3,
    },
  }),
];

let listed: ReviewFiles;

function backend(cmd: string, args?: unknown) {
  const a = (args ?? {}) as Record<string, unknown>;
  switch (cmd) {
    case "list_threads":
      return Promise.resolve(threads);
    case "list_files":
      return Promise.resolve(
        a.sinceLastReview
          ? {
              ...listed,
              sinceLastReview: true,
              files: listed.files.slice(0, 1),
            }
          : listed,
      );
    case "file_contents":
      // Three lines, so the threads on lines 1–2 fit.
      return Promise.resolve(
        `${String(a.path)}@${String(a.sha).slice(0, 1)}\n2\n3`,
      );
    case "set_file_viewed":
      return Promise.resolve(a.viewed ? [a.path] : []);
    default:
      return Promise.resolve(null);
  }
}

beforeEach(() => {
  localStorage.clear();
  clearContentsCache();
  listed = reviewFiles({ reviewId: r.id, files });
  mockInvoke.mockImplementation(backend as typeof invoke);
  useCodeViewStore.setState({ ...initialCode, byReview: {} }, true);
  useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
  useReviewStore.setState(
    { ...initialReviews, reviews: [r], selectedId: r.id },
    true,
  );
  useLayoutStore.setState({
    diffLayout: "unified",
    fileListMode: "tree",
    fileTreeOpen: true,
  });
});

const tree = () => screen.getByRole("tree", { name: "Changed files" });
const conversation = () => screen.getByRole("region", { name: "Conversation" });
const card = (path: string) =>
  screen.getByRole("region", { name: `Diff of ${path}` });

describe("ReviewView", () => {
  it("lists the files with their stats and thread counts", async () => {
    render(<ReviewView review={r} now={NOW} />);
    expect(mockInvoke).toHaveBeenCalledWith("list_files", {
      reviewId: r.id,
      sinceLastReview: false,
    });
    expect(await screen.findByText("0 of 4 files viewed")).toBeInTheDocument();
    const rows = within(tree()).getAllByRole("treeitem");
    // Folders first, compacted, then the files under them.
    expect(
      rows.map((row) => row.textContent?.replace(/\s+/g, " ").trim()),
    ).toEqual([
      // Conversation first: two review-level threads and one elsewhere.
      "Conversation3",
      "assets",
      "Mlogo.png",
      "data",
      "Mbig.json+4000",
      "src",
      "Mcache.rs2+3−1",
      "Awidget.rs1+10",
    ]);
    const cache = within(tree()).getByLabelText("2 threads, 1 yours");
    expect(cache).toHaveAttribute("data-mine", "true");
    expect(within(tree()).getByLabelText("1 thread")).toHaveAttribute(
      "data-mine",
      "false",
    );
    expect(
      within(card("src/cache.rs")).getByLabelText("3 added, 1 removed"),
    ).toBeInTheDocument();

    // The filter narrows the tree and the diff column alike.
    fireEvent.change(screen.getByLabelText("Filter files"), {
      target: { value: "widget" },
    });
    // Conversation stays; one file is left.
    expect(within(tree()).getAllByRole("treeitem")).toHaveLength(3);
    expect(screen.getAllByRole("region")).toHaveLength(2);
  });

  it("shows the conversation and the files in one page, without tabs", async () => {
    render(<ReviewView review={r} now={NOW} />);
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("tab")).toBeNull();
    await screen.findByText("0 of 4 files viewed");
    const regions = screen.getAllByRole("region");
    // The Conversation section first, then the files in tree order.
    expect(regions.map((x) => x.getAttribute("aria-label"))).toEqual([
      "Conversation",
      "Diff of assets/logo.png",
      "Diff of data/big.json",
      "Diff of src/cache.rs",
      "Diff of src/widget.rs",
    ]);
    // Review-level threads, mine first and open, others folded.
    const talk = await within(conversation()).findAllByRole("article");
    expect(talk.map((t) => t.getAttribute("aria-label"))).toEqual([
      "Thread by rev-fixture",
      "Thread by carol-fixture",
      "Thread by bob-fixture",
    ]);
    expect(
      within(talk[0]).getByText("Please add a changelog entry."),
    ).toBeInTheDocument();
    expect(talk[1]).toHaveAttribute("data-open", "false");
    // No code reference: no then/now history.
    expect(
      within(talk[0]).queryByRole("button", { name: "Then/now history" }),
    ).toBeNull();
    // A thread on a file this diff does not show is listed, not lost.
    expect(within(conversation()).getByText("On other files")).toBeVisible();
    expect(
      within(conversation()).getByText("src/elsewhere.rs:3"),
    ).toBeInTheDocument();
    // The files still carry their own threads inline.
    expect(await screen.findByTestId("editor-src/cache.rs")).toHaveAttribute(
      "data-placements",
      "b2,b1",
    );
  });

  it("scrolls to the conversation from its tree entry", async () => {
    // jsdom has no scrollIntoView; record what would scroll.
    const scrolled: string[] = [];
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this.getAttribute("aria-label") ?? "");
    };
    render(<ReviewView review={r} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    const entry = within(tree()).getByRole("button", { name: /Conversation/ });
    fireEvent.click(entry);
    expect(useCodeViewStore.getState().byReview[r.id].selected).toBe(
      CONVERSATION,
    );
    expect(scrolled[scrolled.length - 1]).toBe("Conversation");
    expect(conversation()).toHaveAttribute("data-selected", "true");
    fireEvent.click(within(tree()).getByRole("button", { name: /cache\.rs/ }));
    expect(scrolled[scrolled.length - 1]).toBe("Diff of src/cache.rs");
    Element.prototype.scrollIntoView = original;
  });

  it("renders each diff from both sides with the threads at their lines", async () => {
    render(<ReviewView review={r} now={NOW} />);
    const editor = await screen.findByTestId("editor-src/cache.rs");
    // Base at the left sha, head at the right one.
    expect(editor).toHaveTextContent("src/cache.rs@b 2 3|src/cache.rs@a 2 3");
    expect(editor).toHaveAttribute("data-placements", "b2,b1");
    // An added file has no base to fetch.
    expect(await screen.findByTestId("editor-src/widget.rs")).toHaveTextContent(
      /^\|src\/widget\.rs@a/,
    );
    expect(mockInvoke).not.toHaveBeenCalledWith("file_contents", {
      reviewId: r.id,
      sha: "b".repeat(40),
      path: "src/widget.rs",
    });
  });

  it("toggles viewed, folds the file and counts it; a reset unchecks it", async () => {
    render(<ReviewView review={r} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    fireEvent.click(within(card("src/cache.rs")).getByRole("checkbox"));
    expect(mockInvoke).toHaveBeenCalledWith("set_file_viewed", {
      reviewId: r.id,
      path: "src/cache.rs",
      viewed: true,
    });
    expect(await screen.findByText("1 of 4 files viewed")).toBeInTheDocument();
    expect(within(tree()).getByLabelText("Viewed src/cache.rs")).toBeChecked();
    // Viewed files fold away.
    expect(screen.queryByTestId("editor-src/cache.rs")).toBeNull();

    // The file changed at head: the backend no longer counts the mark.
    listed = reviewFiles({ reviewId: r.id, files });
    await useCodeViewStore.getState().applyChanged(r.id);
    expect(await screen.findByText("0 of 4 files viewed")).toBeInTheDocument();
    expect(
      within(tree()).getByLabelText("Viewed src/cache.rs"),
    ).not.toBeChecked();
  });

  it("switches unified / side by side and remembers it", async () => {
    render(<ReviewView review={r} now={NOW} />);
    const editor = await screen.findByTestId("editor-src/cache.rs");
    expect(editor).toHaveAttribute("data-layout", "unified");
    fireEvent.click(screen.getByRole("button", { name: "Side by side" }));
    expect(screen.getByTestId("editor-src/cache.rs")).toHaveAttribute(
      "data-layout",
      "split",
    );
    expect(
      screen.getByRole("button", { name: "Side by side" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      JSON.parse(localStorage.getItem("argus.layout") ?? "{}").diffLayout,
    ).toBe("split");
  });

  it("shows outdated threads at the top of their file, with the hunk and an in-place history", async () => {
    render(<ReviewView review={r} now={NOW} />);
    const widget = await screen.findByRole("region", {
      name: "Diff of src/widget.rs",
    });
    expect(
      await within(widget).findByText("Outdated threads"),
    ).toBeInTheDocument();
    expect(within(widget).getByText("Outdated")).toBeInTheDocument();
    expect(within(widget).getByText("+fn gone() {}")).toBeInTheDocument();
    // Someone else's thread is folded to a preview; it opens on click.
    const t = within(widget).getByRole("article", {
      name: "Thread by bob-fixture",
    });
    expect(
      within(t).getByText("This was about the old version."),
    ).toBeInTheDocument();
    expect(
      within(t).getByRole("button", { name: /bob-fixture/, expanded: false }),
    ).toBeInTheDocument();
    // Not placed in the diff.
    expect(screen.getByTestId("editor-src/widget.rs")).toHaveAttribute(
      "data-placements",
      "",
    );

    // The then/now history expands in the card itself.
    expect(t.querySelector('[data-slot="then-now"]')).toBeNull();
    const history = within(t).getByRole("button", {
      name: "Then/now history",
    });
    expect(history).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(history);
    expect(history).toHaveAttribute("aria-expanded", "true");
    expect(t.querySelector('[data-slot="then-now"]')).not.toBeNull();
    // Opening the history opens the thread too.
    expect(t).toHaveAttribute("data-open", "true");
    fireEvent.click(history);
    expect(t.querySelector('[data-slot="then-now"]')).toBeNull();
  });

  it("offers 'since my last review' only in later rounds", async () => {
    const { unmount } = render(<ReviewView review={r} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    expect(
      screen.queryByRole("button", { name: "Since my last review" }),
    ).toBeNull();
    unmount();

    // Marked reviewed at the current head: nothing new yet.
    const caughtUp = { ...r, lastReviewedSha: r.headSha };
    const second = render(<ReviewView review={caughtUp} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    expect(
      screen.queryByRole("button", { name: "Since my last review" }),
    ).toBeNull();
    second.unmount();

    const later = { ...r, lastReviewedSha: "c".repeat(40), round: 2 };
    render(<ReviewView review={later} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    const toggle = screen.getByRole("button", { name: "Since my last review" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle);
    expect(mockInvoke).toHaveBeenCalledWith("list_files", {
      reviewId: r.id,
      sinceLastReview: true,
    });
    expect(await screen.findByText("0 of 1 files viewed")).toBeInTheDocument();
    expect(toggle).toHaveAttribute("aria-pressed", "true");
  });

  it("keeps binary and large files as cards; a large one loads on request", async () => {
    render(<ReviewView review={r} now={NOW} />);
    await screen.findByTestId("editor-src/cache.rs");
    expect(
      within(card("assets/logo.png")).getByText("Binary file"),
    ).toBeInTheDocument();
    const big = card("data/big.json");
    expect(within(big).getByText("Large diff")).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "file_contents",
      expect.objectContaining({ path: "data/big.json" }),
    );
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "file_contents",
      expect.objectContaining({ path: "assets/logo.png" }),
    );
    fireEvent.click(within(big).getByRole("button", { name: "Load anyway" }));
    expect(
      await screen.findByTestId("editor-data/big.json"),
    ).toBeInTheDocument();
  });

  it("walks the conversation and the files with j/k and toggles viewed with v", async () => {
    function Shell() {
      useAppShortcuts();
      return <ReviewView review={r} now={NOW} />;
    }
    render(<Shell />);
    await screen.findByText("0 of 4 files viewed");
    const selected = () =>
      within(tree())
        .getAllByRole("treeitem")
        .find((row) => row.getAttribute("aria-selected") === "true")
        ?.textContent?.replace(/\s+/g, " ");
    // The Conversation is the first stop; v does nothing there.
    fireEvent.keyDown(window, { key: "j", code: "KeyJ" });
    expect(selected()).toContain("Conversation");
    fireEvent.keyDown(window, { key: "v", code: "KeyV" });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_file_viewed",
      expect.anything(),
    );
    fireEvent.keyDown(window, { key: "j", code: "KeyJ" });
    expect(selected()).toContain("logo.png");
    fireEvent.keyDown(window, { key: "j", code: "KeyJ" });
    fireEvent.keyDown(window, { key: "j", code: "KeyJ" });
    expect(selected()).toContain("cache.rs");
    fireEvent.keyDown(window, { key: "k", code: "KeyK" });
    expect(selected()).toContain("big.json");
    fireEvent.keyDown(window, { key: "v", code: "KeyV" });
    expect(mockInvoke).toHaveBeenCalledWith("set_file_viewed", {
      reviewId: r.id,
      path: "data/big.json",
      viewed: true,
    });
    expect(await screen.findByText("1 of 4 files viewed")).toBeInTheDocument();
    // k walks back up to the Conversation and stops there.
    for (let i = 0; i < 4; i++) {
      fireEvent.keyDown(window, { key: "k", code: "KeyK" });
    }
    expect(selected()).toContain("Conversation");

    // Not without an open review.
    useReviewStore.setState({ selectedId: null });
    fireEvent.keyDown(window, { key: "j", code: "KeyJ" });
    expect(selected()).toContain("Conversation");
  });
});
