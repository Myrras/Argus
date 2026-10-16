import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { PostResult } from "../ipc/types";
import {
  readStoredDrafts,
  useCommentStore,
  type CommentTarget,
} from "./commentStore";
import { useToastStore } from "./toastStore";

const mockInvoke = vi.mocked(invoke);
const REVIEW = "00000000-0000-4000-8000-000000000001";
const THREAD = "00000000-0000-4000-8000-000000001001";
const HEAD = "a".repeat(40);

const target = (line: number, overrides: Partial<CommentTarget> = {}) => ({
  reviewId: REVIEW,
  path: "src/cache.rs",
  side: "new" as const,
  line,
  startLine: null,
  headSha: HEAD,
  baseSha: "b".repeat(40),
  ...overrides,
});

/** Queue a draft the way the UI does: open, type, Add to review. */
function addDraft(line: number, body: string) {
  const s = useCommentStore.getState();
  expect(s.openComposer(target(line))).toBe(true);
  useCommentStore.getState().setComposerBody(body);
  useCommentStore.getState().saveComposer();
}

const drafts = () => useCommentStore.getState().drafts[REVIEW] ?? [];

const posted = (line: number): PostResult => ({
  status: "posted",
  url: "https://github.com/acme-fixture/widgets/pull/1#r1",
  providerThreadId: String(line),
  providerNoteId: String(line),
  path: "src/cache.rs",
  side: "new",
  line,
  startLine: null,
  headSha: HEAD,
});

beforeEach(() => {
  localStorage.clear();
  mockInvoke.mockReset();
  useCommentStore.setState({
    drafts: {},
    composer: null,
    replies: {},
    posting: {},
    focus: null,
  });
  useToastStore.setState({ toasts: [] });
});

describe("drafts", () => {
  it("are kept per review in localStorage, and never post themselves", () => {
    addDraft(3, "First");
    addDraft(7, "Second");
    expect(drafts().map((d) => d.body)).toEqual(["First", "Second"]);
    // A reload reads them back.
    expect(readStoredDrafts()[REVIEW].map((d) => [d.line, d.body])).toEqual([
      [3, "First"],
      [7, "Second"],
    ]);
    // Editing and deleting are local too.
    const [first, second] = drafts();
    useCommentStore.getState().editDraft(REVIEW, first.id);
    useCommentStore.getState().setComposerBody("First, edited");
    useCommentStore.getState().saveComposer();
    useCommentStore.getState().removeDraft(REVIEW, second.id);
    expect(readStoredDrafts()[REVIEW].map((d) => d.body)).toEqual([
      "First, edited",
    ]);
    useCommentStore.getState().removeDraft(REVIEW, first.id);
    expect(localStorage.getItem(`argus.drafts.v1.${REVIEW}`)).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("skip unreadable stored entries", () => {
    localStorage.setItem(`argus.drafts.v1.${REVIEW}`, "{not json");
    localStorage.setItem("argus.drafts.v1.other", JSON.stringify([{ id: 1 }]));
    expect(readStoredDrafts()).toEqual({});
  });

  it("Post all: posted ones go, failed ones stay with the reason", async () => {
    addDraft(3, "Fine");
    addDraft(9, "Rejected one");
    addDraft(11, "Also fine");
    mockInvoke.mockResolvedValueOnce([
      posted(3),
      { status: "failed", kind: "lineNotInDiff", message: "Line 9 is gone." },
      posted(11),
    ]);
    await useCommentStore.getState().postAll(REVIEW);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = mockInvoke.mock.calls[0];
    expect(cmd).toBe("post_comments");
    expect(
      (args as { comments: { line: number; headSha: string }[] }).comments.map(
        (c) => [c.line, c.headSha],
      ),
    ).toEqual([
      [3, HEAD],
      [9, HEAD],
      [11, HEAD],
    ]);
    expect(drafts()).toHaveLength(1);
    expect(drafts()[0]).toMatchObject({
      body: "Rejected one",
      error: { kind: "lineNotInDiff", message: "Line 9 is gone." },
    });
    // What survived is what a reload shows.
    expect(readStoredDrafts()[REVIEW]).toHaveLength(1);
    expect(useToastStore.getState().toasts[0]).toMatchObject({
      kind: "error",
      message: "1 of 3 comments not posted",
    });
  });

  it("a rejected batch loses nothing", async () => {
    addDraft(3, "One");
    addDraft(4, "Two");
    mockInvoke.mockRejectedValueOnce({
      kind: "githubUnavailable",
      message: "GitHub is not available",
    });
    await useCommentStore.getState().postAll(REVIEW);
    expect(drafts().map((d) => d.body)).toEqual(["One", "Two"]);
    expect(useCommentStore.getState().posting[REVIEW]).toBeUndefined();
  });

  it("retry keeps the text until it is posted", async () => {
    addDraft(9, "Try again");
    mockInvoke.mockResolvedValueOnce([
      { status: "failed", kind: "postRejected", message: "GitHub: no" },
    ]);
    await useCommentStore.getState().postAll(REVIEW);
    const id = drafts()[0].id;
    mockInvoke.mockResolvedValueOnce([
      { status: "failed", kind: "headMoved", message: "New commits." },
    ]);
    await useCommentStore.getState().retryDraft(REVIEW, id);
    expect(drafts()[0]).toMatchObject({
      body: "Try again",
      error: { kind: "headMoved" },
    });
    mockInvoke.mockResolvedValueOnce([posted(9)]);
    await useCommentStore.getState().retryDraft(REVIEW, id);
    expect(drafts()).toEqual([]);
    expect(mockInvoke).toHaveBeenCalledTimes(3);
  });
});

describe("the composer", () => {
  it("Comment now posts just that one; a failure keeps the text", async () => {
    addDraft(2, "A pending one");
    useCommentStore.getState().openComposer(target(5));
    useCommentStore.getState().setComposerBody("Right now");
    mockInvoke.mockResolvedValueOnce([
      { status: "failed", kind: "postRejected", message: "GitHub: refused" },
    ]);
    await useCommentStore.getState().commentNow();
    const [, args] = mockInvoke.mock.calls[0];
    expect((args as { comments: unknown[] }).comments).toHaveLength(1);
    expect(useCommentStore.getState().composer).toMatchObject({
      body: "Right now",
      busy: false,
      error: { message: "GitHub: refused" },
    });
    mockInvoke.mockResolvedValueOnce([posted(5)]);
    await useCommentStore.getState().commentNow();
    expect(useCommentStore.getState().composer).toBeNull();
    // The pending draft was never part of it.
    expect(drafts().map((d) => d.body)).toEqual(["A pending one"]);
  });

  it("keeps an open composer with text instead of replacing it", () => {
    useCommentStore.getState().openComposer(target(5));
    useCommentStore.getState().setComposerBody("unsaved");
    expect(useCommentStore.getState().openComposer(target(8))).toBe(false);
    expect(useCommentStore.getState().composer?.line).toBe(5);
  });

  it("extends a range within the hunk", () => {
    const lines = {
      reviewId: REVIEW,
      path: "src/cache.rs",
      baseSha: "b".repeat(40),
      headSha: HEAD,
      hunks: [{ old: [], new: [3, 4, 5, 6] }],
    };
    useCommentStore.getState().openComposer(target(4));
    useCommentStore.getState().extendComposer(lines, 9);
    expect(useCommentStore.getState().composer).toMatchObject({
      startLine: 4,
      line: 6,
    });
  });

  it("the Comment-on-line key opens the focused line only when commentable", () => {
    const s = useCommentStore.getState();
    s.setFocus({
      reviewId: REVIEW,
      path: "src/cache.rs",
      side: "new",
      line: 8,
      target: null,
    });
    expect(useCommentStore.getState().commentOnFocus(REVIEW)).toBe(false);
    s.setFocus({
      reviewId: REVIEW,
      path: "src/cache.rs",
      side: "new",
      line: 4,
      target: { ...target(4), anchor: 4 },
    });
    expect(useCommentStore.getState().commentOnFocus("other")).toBe(false);
    expect(useCommentStore.getState().commentOnFocus(REVIEW)).toBe(true);
    expect(useCommentStore.getState().composer?.line).toBe(4);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});

describe("replies", () => {
  it("post on send; an error keeps the text", async () => {
    const s = useCommentStore.getState();
    s.openReply(THREAD);
    s.setReplyBody(THREAD, "Fixed?");
    expect(mockInvoke).not.toHaveBeenCalled();
    mockInvoke.mockRejectedValueOnce({
      kind: "postRejected",
      message: "GitHub: locked",
    });
    await useCommentStore.getState().sendReply(REVIEW, THREAD);
    expect(mockInvoke).toHaveBeenCalledWith("reply_to_thread", {
      reviewId: REVIEW,
      threadId: THREAD,
      body: "Fixed?",
    });
    expect(useCommentStore.getState().replies[THREAD]).toMatchObject({
      body: "Fixed?",
      busy: false,
      error: { kind: "postRejected", message: "GitHub: locked" },
    });
    mockInvoke.mockResolvedValueOnce({
      url: "u",
      providerThreadId: "1",
      providerNoteId: "2",
    });
    await useCommentStore.getState().sendReply(REVIEW, THREAD);
    expect(useCommentStore.getState().replies[THREAD]).toBeUndefined();
  });
});
