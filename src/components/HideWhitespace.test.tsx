import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
// CodeMirror needs a real layout engine (the diff itself is tested in
// `lib/whitespaceDiff.test.ts`); the stub shows what it was given.
vi.mock("./DiffEditor", () => ({
  DiffEditor: (p: {
    path: string;
    hideWhitespace?: boolean;
    placements: { line: number; side: string }[];
  }) => (
    <div
      data-testid={`editor-${p.path}`}
      data-hide-whitespace={String(!!p.hideWhitespace)}
      data-placements={p.placements.map((x) => `${x.side}${x.line}`).join(",")}
    />
  ),
}));

import { invoke } from "@tauri-apps/api/core";
import { answerBatches } from "../test/batches";

import type { FileEntry } from "../ipc/types";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { clearCommentableCache, useCommentStore } from "../state/commentStore";
import { useLayoutStore } from "../state/layoutStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { CommandPalette } from "./CommandPalette";
import { ReviewView } from "./ReviewView";

const mockInvoke = answerBatches(vi.mocked(invoke));
const NOW = Date.parse("2026-10-16T12:00:00Z");
const r = review({ headSha: "a".repeat(40) });
const BASE = "b".repeat(40);
const anchor = thread().anchor!;

/** The writes: toggling a view setting runs none. */
const WRITES = [
  "post_comments",
  "reply_to_thread",
  "approve_review",
  "submit_review",
  "post_findings",
  "resolve_thread",
  "set_file_viewed",
];

// Reindented only (tab → spaces, trailing spaces), and a real edit.
const files: FileEntry[] = [
  file({
    path: "src/indent.rs",
    additions: 2,
    deletions: 2,
    patch:
      "@@ -1,3 +1,3 @@\n fn a() {\n-\tb();\n-\tc(); \n+    b();\n+    c();\n }",
  }),
  file({
    path: "src/real.rs",
    additions: 2,
    deletions: 1,
    patch: "@@ -1,2 +1,3 @@\n-  x = 1;\n+x = 2;\n+y();\n z();",
  }),
  file({
    path: "src/talked.rs",
    additions: 1,
    deletions: 1,
    patch: "@@ -1,2 +1,2 @@\n-  q();\n+q();\n w();",
  }),
];

const CONTENTS: Record<string, [string, string]> = {
  "src/indent.rs": [
    "fn a() {\n\tb();\n\tc(); \n}",
    "fn a() {\n    b();\n    c();\n}",
  ],
  "src/real.rs": ["  x = 1;\nz();", "x = 2;\ny();\nz();"],
  "src/talked.rs": ["  q();\nw();", "q();\nw();"],
};

const threads = [
  thread({
    reviewId: r.id,
    author: "bob-fixture",
    anchor: {
      ...anchor,
      pathNew: "src/talked.rs",
      pathOld: "src/talked.rs",
      lineNew: 1,
    },
  }),
];

function backend(cmd: string, args?: unknown) {
  const a = (args ?? {}) as Record<string, unknown>;
  switch (cmd) {
    case "list_threads":
      return Promise.resolve(threads);
    case "list_files":
      return Promise.resolve(
        reviewFiles({ reviewId: r.id, baseSha: BASE, files }),
      );
    case "file_contents": {
      const [base, head] = CONTENTS[String(a.path)];
      return Promise.resolve(a.sha === BASE ? base : head);
    }
    case "commentable_lines":
      return Promise.resolve({
        reviewId: r.id,
        path: a.path,
        baseSha: BASE,
        headSha: r.headSha,
        hunks: [{ old: [1, 2], new: [1, 2] }],
      });
    default:
      return Promise.resolve(null);
  }
}

function Shell() {
  useAppShortcuts();
  return (
    <>
      <ReviewView review={r} now={NOW} />
      <CommandPalette />
    </>
  );
}

function press(init: KeyboardEventInit & { key: string }) {
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  });
}

const tree = () => screen.getByRole("tree", { name: "Changed files" });
const card = (path: string) =>
  screen.getByRole("region", { name: `Diff of ${path}` });
const toggle = () =>
  screen.getByRole("button", { name: "Hide whitespace changes" });
const noWrites = () => {
  for (const cmd of WRITES) {
    expect(mockInvoke).not.toHaveBeenCalledWith(cmd, expect.anything());
  }
};

beforeEach(() => {
  localStorage.clear();
  clearContentsCache();
  clearCommentableCache();
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(backend as typeof invoke);
  useSettingsStore.getState().resetShortcuts();
  useOverlayStore.setState({ open: null, confirm: null });
  useCommentStore.setState({
    drafts: {},
    composer: null,
    replies: {},
    focus: null,
  });
  useCodeViewStore.setState({ byReview: {} });
  useThreadStore.setState({ byReview: {} });
  useReviewStore.setState({ reviews: [r], selectedId: r.id, cursorId: r.id });
  useLayoutStore.setState({
    diffLayout: "unified",
    fileListMode: "flat",
    fileTreeOpen: true,
    hideWhitespace: false,
  });
});

describe("Hide whitespace changes", () => {
  it("is off by default and toggles from the toolbar, saying so while on", async () => {
    render(<Shell />);
    await screen.findByTestId("editor-src/indent.rs");
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByText("Whitespace hidden")).toBeNull();
    expect(toggle()).toHaveAttribute("title", expect.stringContaining("(W)"));
    // Off: the platform's counts, every file's editor shows whitespace.
    expect(
      within(tree()).getByLabelText("2 added, 2 removed"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("editor-src/real.rs")).toHaveAttribute(
      "data-hide-whitespace",
      "false",
    );

    fireEvent.click(toggle());
    expect(useLayoutStore.getState().hideWhitespace).toBe(true);
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("Whitespace hidden")).toBeInTheDocument();
    expect(screen.getByTestId("editor-src/real.rs")).toHaveAttribute(
      "data-hide-whitespace",
      "true",
    );
    // Saved for every review.
    expect(JSON.parse(localStorage.getItem("argus.layout")!)).toMatchObject({
      hideWhitespace: true,
    });
    noWrites();
  });

  it("toggles with W and from the palette", async () => {
    render(<Shell />);
    await screen.findByTestId("editor-src/indent.rs");
    press({ key: "w", code: "KeyW" });
    expect(useLayoutStore.getState().hideWhitespace).toBe(true);
    press({ key: "w", code: "KeyW" });
    expect(useLayoutStore.getState().hideWhitespace).toBe(false);

    press({ key: "k", code: "KeyK", ctrlKey: true });
    const field = screen.getByRole("combobox");
    fireEvent.change(field, { target: { value: "whitespace" } });
    const first = screen.getAllByRole("option")[0];
    expect(first).toHaveTextContent("Hide whitespace changes");
    expect(within(first).getByText("W")).toBeInTheDocument();
    fireEvent.keyDown(field, { key: "Enter" });
    expect(useLayoutStore.getState().hideWhitespace).toBe(true);
    noWrites();
  });

  it("folds a whitespace-only file to a card and marks it in the tree", async () => {
    useLayoutStore.setState({ hideWhitespace: true });
    render(<Shell />);
    await screen.findByRole("region", { name: "Diff of src/indent.rs" });
    const only = await within(card("src/indent.rs")).findByText(
      "Only whitespace changes",
    );
    expect(only).toBeInTheDocument();
    expect(screen.queryByTestId("editor-src/indent.rs")).toBeNull();
    // The tree: a quiet mark instead of +2 −2; the real edit's counts
    // without its whitespace (x = 1 → x = 2 stays, y() is new).
    const rows = within(tree()).getAllByRole("treeitem");
    const row = rows.find((x) => x.textContent?.includes("src/indent.rs"))!;
    expect(
      within(row).getByLabelText("Only whitespace changes"),
    ).toBeInTheDocument();
    expect(within(row).queryByLabelText(/added/)).toBeNull();
    expect(
      within(tree()).getByLabelText("2 added, 1 removed"),
    ).toBeInTheDocument();
    // The card's header counts nothing; viewed marks stay the platform's.
    expect(within(card("src/indent.rs")).queryByLabelText(/added/)).toBeNull();
    expect(
      within(card("src/indent.rs")).getByRole("checkbox"),
    ).not.toBeChecked();

    // "Show changes": this file's diff with whitespace.
    fireEvent.click(
      within(card("src/indent.rs")).getByRole("button", {
        name: "Show changes",
      }),
    );
    const editor = await screen.findByTestId("editor-src/indent.rs");
    expect(editor).toHaveAttribute("data-hide-whitespace", "false");
    expect(
      within(card("src/indent.rs")).getByRole("button", { name: "Hide them" }),
    ).toBeInTheDocument();
    // The other files keep whitespace hidden.
    expect(screen.getByTestId("editor-src/real.rs")).toHaveAttribute(
      "data-hide-whitespace",
      "true",
    );
    noWrites();
  });

  it("keeps the editor for a whitespace-only file with a thread on it", async () => {
    useLayoutStore.setState({ hideWhitespace: true });
    render(<Shell />);
    const editor = await screen.findByTestId("editor-src/talked.rs");
    await waitFor(() =>
      expect(editor).toHaveAttribute("data-placements", "b1"),
    );
    expect(editor).toHaveAttribute("data-hide-whitespace", "true");
    expect(
      within(card("src/talked.rs")).getByText(
        /Only whitespace changes in this file: folded around its comments/,
      ),
    ).toBeInTheDocument();
    noWrites();
  });
});
