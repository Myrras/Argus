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
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { FilesTab } from "./FilesTab";

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
  useCodeViewStore.setState(
    { ...initialCode, byReview: {}, focusThread: null },
    true,
  );
  useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
  useReviewStore.setState(
    { ...initialReviews, reviews: [r], selectedId: r.id },
    true,
  );
  useLayoutStore.setState({
    detailTab: "files",
    diffLayout: "unified",
    fileListMode: "tree",
    fileTreeOpen: true,
  });
});

const tree = () => screen.getByRole("tree", { name: "Changed files" });
const card = (path: string) =>
  screen.getByRole("region", { name: `Diff of ${path}` });

describe("FilesTab", () => {
  it("lists the files with their stats and thread counts", async () => {
    render(<FilesTab review={r} now={NOW} />);
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
    expect(within(tree()).getAllByRole("treeitem")).toHaveLength(2);
    expect(screen.getAllByRole("region")).toHaveLength(1);
  });

  it("renders each diff from both sides with the threads at their lines", async () => {
    render(<FilesTab review={r} now={NOW} />);
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
    render(<FilesTab review={r} now={NOW} />);
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
    render(<FilesTab review={r} now={NOW} />);
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

  it("shows outdated threads at the top of their file, with the hunk and history link", async () => {
    render(<FilesTab review={r} now={NOW} />);
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
      within(t).getByRole("button", { expanded: false }),
    ).toBeInTheDocument();
    // Not placed in the diff.
    expect(screen.getByTestId("editor-src/widget.rs")).toHaveAttribute(
      "data-placements",
      "",
    );

    fireEvent.click(
      within(widget).getByRole("button", { name: "Then/now history" }),
    );
    expect(useLayoutStore.getState().detailTab).toBe("threads");
    expect(useCodeViewStore.getState().focusThread).toBe(threads[2].threadId);
  });

  it("offers 'since my last review' only in later rounds", async () => {
    const { unmount } = render(<FilesTab review={r} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    expect(
      screen.queryByRole("button", { name: "Since my last review" }),
    ).toBeNull();
    unmount();

    // Marked reviewed at the current head: nothing new yet.
    const caughtUp = { ...r, lastReviewedSha: r.headSha };
    const second = render(<FilesTab review={caughtUp} now={NOW} />);
    await screen.findByText("0 of 4 files viewed");
    expect(
      screen.queryByRole("button", { name: "Since my last review" }),
    ).toBeNull();
    second.unmount();

    const later = { ...r, lastReviewedSha: "c".repeat(40), round: 2 };
    render(<FilesTab review={later} now={NOW} />);
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
    render(<FilesTab review={r} now={NOW} />);
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

  it("walks the files with j/k and toggles viewed with v", async () => {
    function Shell() {
      useAppShortcuts();
      return <FilesTab review={r} now={NOW} />;
    }
    render(<Shell />);
    await screen.findByText("0 of 4 files viewed");
    const selected = () =>
      within(tree())
        .getAllByRole("treeitem")
        .find((row) => row.getAttribute("aria-selected") === "true")
        ?.textContent?.replace(/\s+/g, " ");
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

    // Not on the Threads tab.
    useLayoutStore.setState({ detailTab: "threads" });
    fireEvent.keyDown(window, { key: "j", code: "KeyJ" });
    expect(selected()).toContain("big.json");
  });
});
