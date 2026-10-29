/**
 * Writing suggestions: "Insert suggestion" (button and Mod+G) in the new
 * comment and reply composers, its off states with their reasons, the
 * preview as a `SuggestedChange`, editing help inside a block, the range
 * mismatch warning and its fix, and that nothing posts on the way.
 */

import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";
import { answerBatches } from "../test/batches";

import type { ProviderId, StoredThread } from "../ipc/types";
import { isMac } from "../lib/platform";
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { useCommentStore } from "../state/commentStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { DraftBlock, InlineComposer } from "./CommentBlocks";
import { ThreadBlock } from "./ThreadBlock";

const mockInvoke = answerBatches(vi.mocked(invoke));
const REVIEW = "00000000-0000-4000-8000-000000000001";
const HEAD = "a".repeat(40);
const FILE =
  "fn a() {\n    let one = 1;\n    let two = 2;\n    let three = 3;\n}\n";
const NOW = Date.parse("2026-10-16T12:00:00Z");

const modG = { key: "g", code: "KeyG", metaKey: isMac, ctrlKey: !isMac };

/** Calls that write to the platform. */
const writes = () =>
  mockInvoke.mock.calls.filter(([cmd]) =>
    [
      "post_comments",
      "reply_to_thread",
      "submit_review",
      "post_findings",
    ].includes(cmd),
  );

function setup(
  provider: ProviderId,
  files = reviewFiles({ reviewId: REVIEW }),
) {
  useReviewStore.setState({
    reviews: [review({ id: REVIEW, provider, headSha: HEAD })],
  });
  useCodeViewStore.setState({
    byReview: {
      [REVIEW]: {
        since: false,
        files,
        loading: false,
        error: null,
        selected: null,
        order: [],
      },
    },
  });
}

function openComposer(
  over: Partial<{
    side: "old" | "new";
    line: number;
    startLine: number | null;
    outside: "native" | "fallback" | null;
  }> = {},
) {
  act(() => {
    useCommentStore.getState().openComposer({
      reviewId: REVIEW,
      path: "src/cache.rs",
      side: "new",
      line: 3,
      startLine: null,
      headSha: HEAD,
      baseSha: null,
      ...over,
    });
  });
}

const field = () => screen.getByRole("textbox") as HTMLTextAreaElement;
const button = () => screen.getByRole("button", { name: "Insert suggestion" });
const body = () => useCommentStore.getState().composer?.body;

/** The button once the file is in. */
async function ready() {
  await vi.waitFor(() =>
    expect(button()).toHaveAttribute("aria-disabled", "false"),
  );
}

beforeEach(() => {
  clearContentsCache();
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(((cmd: string) => {
    if (cmd === "file_contents") return Promise.resolve(FILE);
    if (cmd === "post_comments") return Promise.resolve([{ status: "posted" }]);
    return Promise.resolve(null);
  }) as typeof invoke);
  useCommentStore.setState({
    drafts: {},
    composer: null,
    replies: {},
    posting: {},
    focus: null,
  });
  useSettingsStore.setState({ shortcuts: {} });
});

describe("Insert suggestion in a new comment", () => {
  it("GitHub: the button inserts the line at the head in a bare fence", async () => {
    setup("github");
    openComposer();
    render(<InlineComposer />);
    await ready();
    fireEvent.click(button());
    expect(body()).toBe("```suggestion\n    let two = 2;\n```");
    // Read at the review head.
    expect(mockInvoke).toHaveBeenCalledWith("file_contents", {
      reviewId: REVIEW,
      sha: HEAD,
      path: "src/cache.rs",
    });
    expect(writes()).toEqual([]);
  });

  it("GitLab: Mod+G inserts -N+0 for a range, at the caret", async () => {
    setup("gitlab");
    openComposer({ line: 4, startLine: 2 });
    render(<InlineComposer />);
    await ready();
    act(() => useCommentStore.getState().setComposerBody("Simpler:"));
    field().setSelectionRange(8, 8);
    fireEvent.keyDown(field(), modG);
    expect(body()).toBe(
      "Simpler:\n```suggestion:-2+0\n    let one = 1;\n    let two = 2;\n    let three = 3;\n```",
    );
    expect(writes()).toEqual([]);
  });

  it("is a rebindable key", async () => {
    setup("github");
    useSettingsStore.getState().setShortcut("insertSuggestion", ["Alt+KeyS"]);
    openComposer();
    render(<InlineComposer />);
    await ready();
    fireEvent.keyDown(field(), modG);
    expect(body()).toBe("");
    fireEvent.keyDown(field(), { key: "ß", code: "KeyS", altKey: true });
    expect(body()).toContain("```suggestion");
  });

  it("is off with the reason on an old-side line", () => {
    setup("github");
    openComposer({ side: "old" });
    render(<InlineComposer />);
    expect(button()).toHaveAttribute("aria-disabled", "true");
    expect(button()).toHaveAttribute(
      "title",
      expect.stringContaining("Suggestions apply to the new code"),
    );
    fireEvent.click(button());
    fireEvent.keyDown(field(), modG);
    expect(body()).toBe("");
  });

  it("is off for GitHub's file comment outside the diff, on for GitLab's line comment", async () => {
    setup("github");
    openComposer({ outside: "fallback" });
    const view = render(<InlineComposer />);
    expect(button()).toHaveAttribute(
      "title",
      expect.stringContaining("GitHub takes no suggestion outside the diff"),
    );
    view.unmount();
    act(() => useCommentStore.getState().closeComposer());
    setup("gitlab");
    openComposer({ outside: "native" });
    render(<InlineComposer />);
    await ready();
  });

  it("is off for a file without a diff, and when the file can't be read", async () => {
    setup(
      "github",
      reviewFiles({
        reviewId: REVIEW,
        files: [file({ tooLarge: true, patch: null })],
      }),
    );
    openComposer();
    const view = render(<InlineComposer />);
    expect(button()).toHaveAttribute(
      "title",
      expect.stringContaining("no diff for this file"),
    );
    view.unmount();
    setup("github");
    mockInvoke.mockImplementation((() =>
      Promise.reject({ kind: "provider", message: "nope" })) as typeof invoke);
    render(<InlineComposer />);
    await vi.waitFor(() =>
      expect(button()).toHaveAttribute(
        "title",
        expect.stringContaining("Couldn't read the file"),
      ),
    );
    expect(button()).toHaveAttribute("aria-disabled", "true");
  });
});

describe("preview and drafts", () => {
  it("shows the block as the suggested change against the head's lines", async () => {
    setup("gitlab");
    openComposer({ line: 4, startLine: 3 });
    render(<InlineComposer />);
    await ready();
    fireEvent.click(button());
    act(() =>
      useCommentStore
        .getState()
        .setComposerBody(
          "Merge:\n```suggestion:-1+0\n    let both = (2, 3);\n```",
        ),
    );
    fireEvent.click(screen.getByRole("tab", { name: "Preview" }));
    const box = await screen.findByRole("figure", { name: "Suggested change" });
    expect(within(box).getByText("lines 3–4")).toBeInTheDocument();
    await within(box).findByText(/let two = 2;/);
    const rows = (kind: string) =>
      [...box.querySelectorAll(`[data-kind="${kind}"]`)].map((r) =>
        r.textContent?.replace(/^\d*[-+]/, ""),
      );
    expect(rows("del")).toEqual(["    let two = 2;", "    let three = 3;"]);
    expect(rows("add")).toEqual(["    let both = (2, 3);"]);

    // Saved as a draft, it shows the same way.
    fireEvent.click(screen.getByRole("button", { name: "Add to review" }));
    const id = useCommentStore.getState().drafts[REVIEW]![0]!.id;
    render(<DraftBlock reviewId={REVIEW} id={id} />);
    const draftBox = await screen.findByRole("figure", {
      name: "Suggested change",
    });
    expect(within(draftBox).getByText("lines 3–4")).toBeInTheDocument();
    expect(writes()).toEqual([]);
  });
});

describe("editing inside a suggestion", () => {
  it("Enter keeps the indentation, Tab adds the file's step, outside both are plain", async () => {
    setup("github");
    openComposer();
    render(<InlineComposer />);
    await ready();
    fireEvent.click(button());
    const text = body()!;
    const end = text.indexOf("2;") + 2;
    field().setSelectionRange(end, end);
    fireEvent.keyDown(field(), { key: "Enter", code: "Enter" });
    expect(body()).toBe("```suggestion\n    let two = 2;\n    \n```");
    const caret = body()!.indexOf("\n```");
    field().setSelectionRange(caret, caret);
    fireEvent.keyDown(field(), { key: "Tab", code: "Tab" });
    expect(body()).toBe("```suggestion\n    let two = 2;\n        \n```");
    // Outside the block Tab is left alone (no preventDefault, no change).
    act(() => useCommentStore.getState().setComposerBody("plain"));
    field().setSelectionRange(5, 5);
    const ev = fireEvent.keyDown(field(), { key: "Tab", code: "Tab" });
    expect(ev).toBe(true);
    expect(body()).toBe("plain");
  });
});

describe("a suggestion that no longer covers the comment's lines", () => {
  it("GitLab: warns and updates the offsets", async () => {
    setup("gitlab");
    openComposer({ line: 4, startLine: 2 });
    render(<InlineComposer />);
    await ready();
    act(() =>
      useCommentStore
        .getState()
        .setComposerBody("```suggestion:-0+0\nlet x = 1;\n```"),
    );
    expect(screen.getByTestId("suggestion-mismatch")).toHaveTextContent(
      "The suggestion covers 1 line but the comment is on 3",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Update suggestion range" }),
    );
    expect(body()).toBe("```suggestion:-2+0\nlet x = 1;\n```");
    expect(screen.queryByTestId("suggestion-mismatch")).toBeNull();
  });

  it("GitHub: a range changed after inserting puts the comment back", async () => {
    setup("github");
    openComposer({ line: 4, startLine: 3 });
    render(<InlineComposer />);
    await ready();
    fireEvent.click(button());
    expect(screen.queryByTestId("suggestion-mismatch")).toBeNull();
    // Shift-click to line 2 after inserting.
    act(() =>
      useCommentStore.setState((s) => ({
        composer: { ...s.composer!, startLine: 2 },
      })),
    );
    expect(screen.getByTestId("suggestion-mismatch")).toHaveTextContent(
      "The suggestion covers 2 lines but the comment is on 3",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Comment on lines 3–4" }),
    );
    const c = useCommentStore.getState().composer!;
    expect([c.startLine, c.line]).toEqual([3, 4]);
    expect(screen.queryByTestId("suggestion-mismatch")).toBeNull();
    expect(writes()).toEqual([]);
  });

  it("posting sends the body unchanged, only from its button", async () => {
    setup("github");
    openComposer({ line: 4, startLine: 3 });
    render(<InlineComposer />);
    await ready();
    fireEvent.click(button());
    const sent = body();
    expect(writes()).toEqual([]);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Comment now" }));
    });
    expect(writes()).toEqual([
      [
        "post_comments",
        {
          reviewId: REVIEW,
          comments: [
            expect.objectContaining({ body: sent, line: 4, startLine: 3 }),
          ],
        },
      ],
    ]);
  });
});

describe("Insert suggestion in a reply", () => {
  function replyTo(t: StoredThread, provider: ProviderId) {
    act(() => useCommentStore.getState().openReply(t.threadId));
    render(
      <ThreadBlock
        thread={{ ...t, mine: true }}
        provider={provider}
        now={NOW}
      />,
    );
  }

  it("GitHub: the thread's current lines, in a bare fence", async () => {
    setup("github");
    const base = thread({ reviewId: REVIEW });
    // Written on lines 1–2 at an older commit; at the head they are 2–3.
    replyTo(
      {
        ...base,
        anchor: { ...base.anchor!, lineNew: 2, startLine: 1, currentLine: 3 },
      },
      "github",
    );
    await ready();
    fireEvent.click(button());
    const reply = Object.values(useCommentStore.getState().replies)[0]!;
    expect(reply.body).toBe(
      "```suggestion\n    let one = 1;\n    let two = 2;\n```",
    );
    expect(writes()).toEqual([]);
  });

  it("GitLab: -N+0 from the discussion's last line", async () => {
    setup("gitlab");
    const base = thread({ reviewId: REVIEW });
    replyTo(
      {
        ...base,
        anchor: { ...base.anchor!, lineNew: 4, startLine: 3, headSha: HEAD },
      },
      "gitlab",
    );
    await ready();
    fireEvent.keyDown(field(), modG);
    const reply = Object.values(useCommentStore.getState().replies)[0]!;
    expect(reply.body).toBe(
      "```suggestion:-1+0\n    let two = 2;\n    let three = 3;\n```",
    );
    expect(writes()).toEqual([]);
  });

  it("is off on a review-level thread", () => {
    setup("github");
    replyTo(thread({ reviewId: REVIEW, anchor: null }), "github");
    expect(button()).toHaveAttribute("aria-disabled", "true");
    expect(button()).toHaveAttribute(
      "title",
      expect.stringContaining("whole review"),
    );
  });

  it("is off on an outdated thread", () => {
    setup("gitlab");
    replyTo(thread({ reviewId: REVIEW, outdated: true }), "gitlab");
    expect(button()).toHaveAttribute(
      "title",
      expect.stringContaining("outdated"),
    );
  });
});
