import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { useCommentStore } from "../state/commentStore";
import { InlineComposer, ReplyComposer } from "./CommentBlocks";

const REVIEW = "00000000-0000-4000-8000-000000000001";
const THREAD = "00000000-0000-4000-8000-000000001001";

beforeEach(() => {
  useCommentStore.setState({
    drafts: {},
    composer: null,
    replies: {},
    posting: {},
    focus: null,
  });
});

/** Something else the user moved on to. */
function elsewhere() {
  const button = document.createElement("button");
  button.textContent = "elsewhere";
  document.body.appendChild(button);
  button.focus();
  return button;
}

describe("composer focus", () => {
  it("takes the focus when opened, not when its widget is rebuilt", async () => {
    useCommentStore.getState().openComposer({
      reviewId: REVIEW,
      path: "src/cache.rs",
      side: "new",
      line: 4,
      startLine: null,
      headSha: "a".repeat(40),
      baseSha: null,
    });
    const first = render(<InlineComposer />);
    expect(screen.getByRole("textbox")).toHaveFocus();
    first.unmount();
    await new Promise((r) => setTimeout(r, 0));
    // Scrolled out and back in: a new mount of the same composer.
    const other = elsewhere();
    render(<InlineComposer />);
    expect(other).toHaveFocus();
    other.remove();
  });

  it("does the same for a reply", async () => {
    useCommentStore.getState().openReply(THREAD);
    const first = render(<ReplyComposer reviewId={REVIEW} threadId={THREAD} />);
    expect(screen.getByRole("textbox", { name: "Reply" })).toHaveFocus();
    first.unmount();
    await new Promise((r) => setTimeout(r, 0));
    const other = elsewhere();
    render(<ReplyComposer reviewId={REVIEW} threadId={THREAD} />);
    expect(other).toHaveFocus();
    other.remove();
  });
});

describe("a draft's edit while its review posts", () => {
  it("cannot Save until the post finishes", () => {
    useCommentStore.setState({
      drafts: {
        [REVIEW]: [
          {
            id: "draft-fixture-1",
            path: "src/cache.rs",
            side: "new",
            line: 4,
            startLine: null,
            body: "sent text",
            headSha: "a".repeat(40),
            baseSha: null,
            createdAt: "2026-01-01T00:00:00Z",
            error: null,
          },
        ],
      },
    });
    useCommentStore.getState().editDraft(REVIEW, "draft-fixture-1");
    useCommentStore.setState({ posting: { [REVIEW]: true } });
    const view = render(<InlineComposer />);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    useCommentStore.setState({ posting: {} });
    view.rerender(<InlineComposer />);
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });
});
