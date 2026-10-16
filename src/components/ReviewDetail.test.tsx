import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
// CodeMirror needs a layout engine; ReviewView.test covers the diffs.
vi.mock("./DiffEditor", () => ({
  DiffEditor: (p: { path: string }) => <div data-testid={`editor-${p.path}`} />,
}));

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { ReviewDetail } from "./ReviewDetail";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialThreads = useThreadStore.getState();
const initialCode = useCodeViewStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");

const r = review({
  title: "Add widget caching",
  status: "needsReReview",
  round: 3,
  roundReasons: ["newCommits"],
  untouchedThreads: 1,
});

const threads = [
  thread({
    reviewId: r.id,
    author: "bob-fixture",
    createdAt: "2026-10-14T08:00:00Z",
    anchor: { ...thread().anchor!, pathNew: "src/other.rs", lineNew: 7 },
  }),
  thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    createdAt: "2026-10-15T08:00:00Z",
    outdated: true,
    anchor: { ...thread().anchor!, pathNew: "src/cache.rs", lineNew: 42 },
    notes: [
      {
        id: "n-outdated",
        author: "rev-fixture",
        body: "Cache this lookup.",
        createdAt: "2026-10-15T08:00:00Z",
        system: false,
      },
    ],
  }),
  thread({
    reviewId: r.id,
    mine: true,
    author: "rev-fixture",
    createdAt: "2026-10-13T08:00:00Z",
    resolved: true,
    anchor: null,
  }),
];

const files = reviewFiles({
  reviewId: r.id,
  files: [
    file({ path: "src/cache.rs" }),
    file({ path: "src/other.rs", additions: 1, deletions: 0 }),
  ],
});

describe("ReviewDetail", () => {
  beforeEach(() => {
    clearContentsCache();
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "list_threads"
          ? threads
          : cmd === "list_reviews"
            ? [r]
            : cmd === "list_files"
              ? files
              : cmd === "file_contents"
                ? "a\nb\nc"
                : r,
      ),
    );
    useCodeViewStore.setState({ ...initialCode, byReview: {} }, true);
    useReviewStore.setState(
      { ...initialReviews, reviews: [r], loaded: true, selectedId: r.id },
      true,
    );
    useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
  });

  it("shows a placeholder without a selection", () => {
    useReviewStore.setState({ ...initialReviews, reviews: [r] }, true);
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByText("Select a review to see its threads."),
    ).toBeInTheDocument();
  });

  it("shows one page: the conversation, then each file with its threads", async () => {
    render(<ReviewDetail now={NOW} />);
    expect(mockInvoke).toHaveBeenCalledWith("list_threads", {
      reviewId: r.id,
    });
    // No Threads / Files tabs any more.
    expect(screen.queryByRole("tablist")).toBeNull();
    const conversation = screen.getByRole("region", { name: "Conversation" });
    // The review-level thread sits in the Conversation, resolved.
    const general = await within(conversation).findByRole("article", {
      name: "Thread by rev-fixture",
    });
    expect(within(general).getByText("Resolved")).toBeInTheDocument();
    expect(general.querySelector('[data-slot="then-now"]')).toBeNull();
    // The outdated one at the top of its file, never hidden.
    const cache = await screen.findByRole("region", {
      name: "Diff of src/cache.rs",
    });
    expect(await within(cache).findByText("Outdated")).toBeInTheDocument();
    expect(within(cache).getByText("Cache this lookup.")).toBeInTheDocument();
    // Someone else's thread is inline in its file (the editor gets it).
    expect(
      screen.getByRole("region", { name: "Diff of src/other.rs" }),
    ).toBeInTheDocument();
    // Mine are open; the Conversation entry leads the tree.
    const rows = within(
      screen.getByRole("tree", { name: "Changed files" }),
    ).getAllByRole("treeitem");
    expect(rows[0]).toHaveTextContent("Conversation");
  });

  it("shows status, round and reasons, and marks the round reviewed", async () => {
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByRole("heading", { name: "Add widget caching" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Round 3")).toBeInTheDocument();
    expect(screen.getByText("Needs re-review")).toBeInTheDocument();
    expect(screen.getByText("New commits")).toBeInTheDocument();
    expect(screen.getByText("1 of your threads untouched")).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Mark round reviewed" }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("mark_reviewed", {
      reviewId: r.id,
    });
    await screen.findAllByRole("article");
  });

  it("opens the review in the browser", async () => {
    render(<ReviewDetail now={NOW} />);
    fireEvent.click(screen.getByRole("button", { name: /Open in GitHub/ }));
    expect(openUrl).toHaveBeenCalledWith(r.url);
    await screen.findAllByRole("article");
  });

  it("disables marking when nothing is new", async () => {
    const waiting = {
      ...r,
      status: "waitingOnAuthor" as const,
      roundReasons: [],
    };
    useReviewStore.setState(
      { ...initialReviews, reviews: [waiting], selectedId: waiting.id },
      true,
    );
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByRole("button", { name: "Mark round reviewed" }),
    ).toBeDisabled();
    await screen.findAllByRole("article");
  });
});
