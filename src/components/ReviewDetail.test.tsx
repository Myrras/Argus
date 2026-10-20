import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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

import type { PostResult } from "../ipc/types";
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { useCommentStore, type Draft } from "../state/commentStore";
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

const WRITES = [
  "post_comments",
  "reply_to_thread",
  "approve_review",
  "submit_review",
];
const writes = () =>
  mockInvoke.mock.calls.filter(([cmd]) => WRITES.includes(cmd as string));

const draft = (id: string, line: number, body: string): Draft => ({
  id,
  path: "src/cache.rs",
  side: "new",
  line,
  startLine: null,
  body,
  headSha: r.headSha,
  baseSha: r.baseSha,
  createdAt: "2026-10-16T11:00:00Z",
  error: null,
});

/** The default backend, with `extra` answering some commands. */
function backend(extra: Record<string, (args: unknown) => unknown> = {}) {
  mockInvoke.mockImplementation(((cmd: string, args?: unknown) => {
    if (extra[cmd]) {
      try {
        return Promise.resolve(extra[cmd](args));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    return Promise.resolve(
      cmd === "list_threads"
        ? threads
        : cmd === "list_reviews"
          ? [r]
          : cmd === "list_files"
            ? files
            : cmd === "file_contents"
              ? "a\nb\nc"
              : r,
    );
  }) as typeof invoke);
}

describe("ReviewDetail", () => {
  beforeEach(() => {
    clearContentsCache();
    localStorage.clear();
    useCommentStore.setState({
      drafts: {},
      composer: null,
      replies: {},
      posting: {},
      focus: null,
    });
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
    // The review-level thread sits in the Conversation, resolved: in the
    // Resolved group, folded until clicked.
    fireEvent.click(
      await within(conversation).findByRole("button", { name: /^Resolved/ }),
    );
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

  it("shows status, round, reasons and the user's review state", async () => {
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByRole("heading", { name: "Add widget caching" }),
    ).toBeInTheDocument();
    // The round sits on the status badge: "R3" for the eye, named in full.
    expect(screen.getByText("Round 3")).toBeInTheDocument();
    expect(screen.getByTitle("Round 3")).toHaveTextContent("R3");
    expect(screen.getByText("Needs re-review")).toBeInTheDocument();
    expect(screen.getByText("New commits")).toBeInTheDocument();
    expect(screen.getByText("1 of your threads untouched")).toBeInTheDocument();
    expect(screen.getByText("Awaiting your review")).toBeInTheDocument();
    // The local-only "Mark round reviewed" is gone.
    expect(
      screen.queryByRole("button", { name: "Mark round reviewed" }),
    ).toBeNull();
    // Submit review only opens the dialog.
    fireEvent.click(screen.getByRole("button", { name: "Submit review…" }));
    expect(
      screen.getByRole("dialog", {
        name: `Submit review — acme-fixture/widgets#${r.number}`,
      }),
    ).toBeInTheDocument();
    await screen.findAllByRole("article");
    expect(writes()).toEqual([]);
  });

  it("opens the review in the browser", async () => {
    render(<ReviewDetail now={NOW} />);
    fireEvent.click(screen.getByRole("button", { name: /Open in GitHub/ }));
    expect(openUrl).toHaveBeenCalledWith(r.url);
    await screen.findAllByRole("article");
  });

  it("offers Submit review only on an open review", async () => {
    const merged = { ...r, state: "merged" as const };
    useReviewStore.setState(
      { ...initialReviews, reviews: [merged], selectedId: merged.id },
      true,
    );
    render(<ReviewDetail now={NOW} />);
    expect(
      screen.getByRole("button", { name: "Submit review…" }),
    ).toBeDisabled();
    await screen.findAllByRole("article");
  });

  it("counts pending comments and posts them all only on Post all", async () => {
    useCommentStore.setState({
      drafts: {
        [r.id]: [draft("d1", 1, "Posts fine"), draft("d2", 2, "Gets refused")],
      },
    });
    const results: PostResult[] = [
      {
        status: "posted",
        url: "https://github.com/acme-fixture/widgets/pull/1#r9",
        providerThreadId: "9",
        providerNoteId: "9",
        path: "src/cache.rs",
        side: "new",
        line: 1,
        startLine: null,
        headSha: r.headSha,
      },
      { status: "failed", kind: "postRejected", message: "GitHub: refused" },
    ];
    backend({ post_comments: () => results });
    render(<ReviewDetail now={NOW} />);
    expect(screen.getByText("2 pending comments")).toBeInTheDocument();
    await screen.findAllByRole("article");
    expect(writes()).toEqual([]);

    fireEvent.click(screen.getByRole("button", { name: "Post all" }));
    expect(writes()).toHaveLength(1);
    // The posted one is gone, the refused one stays with its reason.
    expect(await screen.findByText("(1 not posted)")).toBeInTheDocument();
    expect(screen.getByText("1 pending comment")).toBeInTheDocument();
    expect(useCommentStore.getState().drafts[r.id]).toMatchObject([
      { id: "d2", body: "Gets refused", error: { message: "GitHub: refused" } },
    ]);
  });

  it("replies in a thread; an error shows inline and keeps the text", async () => {
    let fail = true;
    backend({
      reply_to_thread: () => {
        if (fail) throw { kind: "postRejected", message: "GitHub: locked" };
        return { url: "u", providerThreadId: "1", providerNoteId: "2" };
      },
    });
    render(<ReviewDetail now={NOW} />);
    const conversation = screen.getByRole("region", { name: "Conversation" });
    // Resolved, so folded in its group first.
    fireEvent.click(
      await within(conversation).findByRole("button", { name: /^Resolved/ }),
    );
    fireEvent.click(
      await within(conversation).findByRole("button", {
        name: "Reply to rev-fixture's thread",
      }),
    );
    const field = within(conversation).getByRole("textbox", { name: "Reply" });
    fireEvent.change(field, { target: { value: "Still needed?" } });
    expect(writes()).toEqual([]);
    fireEvent.click(
      within(conversation).getByRole("button", { name: "Reply" }),
    );
    expect(await within(conversation).findByRole("alert")).toHaveTextContent(
      "GitHub: locked",
    );
    expect(field).toHaveValue("Still needed?");
    fail = false;
    fireEvent.click(
      within(conversation).getByRole("button", { name: "Reply" }),
    );
    await waitFor(() =>
      expect(
        within(conversation).queryByRole("textbox", { name: "Reply" }),
      ).toBeNull(),
    );
    expect(writes().map(([cmd, args]) => [cmd, args])).toEqual([
      [
        "reply_to_thread",
        {
          reviewId: r.id,
          threadId: threads[2].threadId,
          body: "Still needed?",
        },
      ],
      [
        "reply_to_thread",
        {
          reviewId: r.id,
          threadId: threads[2].threadId,
          body: "Still needed?",
        },
      ],
    ]);
  });

  /** Open Submit review from the header; the dialog. */
  async function openSubmit() {
    render(<ReviewDetail now={NOW} />);
    await screen.findAllByRole("article");
    fireEvent.click(screen.getByRole("button", { name: "Submit review…" }));
    return screen.getByRole("dialog", {
      name: `Submit review — acme-fixture/widgets#${r.number}`,
    });
  }

  const submitCalls = () => writes().filter(([cmd]) => cmd === "submit_review");

  it("approves at the head it names; a refusal shows inline", async () => {
    let fail = true;
    backend({
      submit_review: () => {
        if (fail) {
          throw { kind: "headMoved", message: "The review got new commits." };
        }
        return { ...r, approvedAt: "2026-10-16T12:00:00Z", status: "approved" };
      },
    });
    const dialog = await openSubmit();
    expect(within(dialog).getByText(r.headSha.slice(0, 7))).toBeInTheDocument();
    // Nothing chosen yet: nothing to submit.
    expect(
      within(dialog).getByRole("button", { name: "Submit review" }),
    ).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The review got new commits.",
    );
    fail = false;
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(writes()).toEqual([
      [
        "submit_review",
        {
          reviewId: r.id,
          headSha: r.headSha,
          verdict: "approve",
          summary: null,
        },
      ],
      [
        "submit_review",
        {
          reviewId: r.id,
          headSha: r.headSha,
          verdict: "approve",
          summary: null,
        },
      ],
    ]);
  });

  it("requests changes with a summary, which GitHub requires", async () => {
    backend({ submit_review: () => ({ ...r, status: "waitingOnAuthor" }) });
    const dialog = await openSubmit();
    // GitHub offers a comment review too.
    expect(
      within(dialog).getByRole("radio", { name: /Comment/ }),
    ).toBeInTheDocument();
    fireEvent.click(
      within(dialog).getByRole("radio", { name: /Request changes/ }),
    );
    const button = within(dialog).getByRole("button", {
      name: "Request changes",
    });
    expect(button).toBeDisabled();
    expect(within(dialog).getByText("Required")).toBeInTheDocument();
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Summary" }), {
      target: { value: "Please **add** a test." },
    });
    fireEvent.click(within(dialog).getByRole("tab", { name: "Preview" }));
    expect(within(dialog).getByText("add").tagName).toBe("STRONG");
    fireEvent.click(button);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(submitCalls()).toEqual([
      [
        "submit_review",
        {
          reviewId: r.id,
          headSha: r.headSha,
          verdict: "requestChanges",
          summary: "Please **add** a test.",
        },
      ],
    ]);
  });

  it("offers GitLab only approve or request changes, summary optional", async () => {
    const gl = {
      ...r,
      provider: "gitlab" as const,
      host: "gitlab.example.test",
    };
    useReviewStore.setState(
      { ...initialReviews, reviews: [gl], selectedId: gl.id },
      true,
    );
    backend({ submit_review: () => ({ ...gl, status: "waitingOnAuthor" }) });
    render(<ReviewDetail now={NOW} />);
    await screen.findAllByRole("article");
    fireEvent.click(screen.getByRole("button", { name: "Submit review…" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).queryByRole("radio", { name: /Comment/ })).toBeNull();
    fireEvent.click(
      within(dialog).getByRole("radio", { name: /Request changes/ }),
    );
    expect(
      within(dialog).getByRole("button", { name: "Request changes" }),
    ).toBeEnabled();
  });

  it("posts the pending comments first, then submits", async () => {
    useCommentStore.setState({
      drafts: { [r.id]: [draft("d1", 1, "Posts fine")] },
    });
    const posted: PostResult = {
      status: "posted",
      url: "https://github.com/acme-fixture/widgets/pull/1#r9",
      providerThreadId: "9",
      providerNoteId: "9",
      path: "src/cache.rs",
      side: "new",
      line: 1,
      startLine: null,
      headSha: r.headSha,
    };
    backend({
      post_comments: () => [posted],
      submit_review: () => ({ ...r, status: "approved" }),
    });
    const dialog = await openSubmit();
    const box = within(dialog).getByRole("checkbox", {
      name: "Post 1 pending comment with it",
    });
    expect(box).toBeChecked();
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(writes().map(([cmd]) => cmd)).toEqual([
      "post_comments",
      "submit_review",
    ]);
    expect(useCommentStore.getState().drafts[r.id] ?? []).toEqual([]);
  });

  it("a refused draft aborts the submit and says why", async () => {
    useCommentStore.setState({
      drafts: {
        [r.id]: [draft("d1", 1, "Posts fine"), draft("d2", 2, "Gets refused")],
      },
    });
    backend({
      post_comments: () => [
        {
          status: "posted",
          url: "u",
          providerThreadId: "9",
          providerNoteId: "9",
          path: "src/cache.rs",
          side: "new",
          line: 1,
          startLine: null,
          headSha: r.headSha,
        },
        {
          status: "failed",
          kind: "lineNotInDiff",
          message: "That line is not in the diff any more.",
        },
      ],
    });
    const dialog = await openSubmit();
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent(
      "Not submitted: 1 pending comment was not posted.",
    );
    expect(alert).toHaveTextContent(
      "src/cache.rs:2 — That line is not in the diff any more.",
    );
    expect(submitCalls()).toEqual([]);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("can leave the drafts out", async () => {
    useCommentStore.setState({
      drafts: { [r.id]: [draft("d1", 1, "Later")] },
    });
    backend({ submit_review: () => ({ ...r, status: "approved" }) });
    const dialog = await openSubmit();
    fireEvent.click(
      within(dialog).getByRole("checkbox", {
        name: "Post 1 pending comment with it",
      }),
    );
    expect(
      within(dialog).getByText("1 pending comment stays unposted."),
    ).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(writes().map(([cmd]) => cmd)).toEqual(["submit_review"]);
    expect(useCommentStore.getState().drafts[r.id]).toHaveLength(1);
  });

  it("cancelling Submit review writes nothing", async () => {
    backend();
    const dialog = await openSubmit();
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(writes()).toEqual([]);
  });
});
