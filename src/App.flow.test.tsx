/**
 * The whole app over a stateful fake backend (mocked `invoke` + events):
 * the list, opening a review, its conversation, file and threads (an
 * outdated one included), composing drafts, Post all with a partial
 * failure, approving (the review leaves the list), the AI panel with the AI
 * pass on and off, and error toasts. CodeMirror is stubbed: the stub renders
 * the threads and blocks it is given and a button per commentable line.
 */

import {
  act,
  configure,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const events = vi.hoisted(() => {
  const handlers = new Map<string, ((e: { payload: unknown }) => void)[]>();
  return {
    listen: (name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.set(name, [...(handlers.get(name) ?? []), cb]);
      return Promise.resolve(() => {
        handlers.set(
          name,
          (handlers.get(name) ?? []).filter((h) => h !== cb),
        );
      });
    },
    emit: (name: string, payload: unknown) =>
      (handlers.get(name) ?? []).forEach((h) => h({ payload })),
    clear: () => handlers.clear(),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
  info: vi.fn(() => Promise.resolve()),
}));
vi.mock("./components/DiffEditor", () => ({
  DiffEditor: (p: {
    path: string;
    placements: { thread: StoredThread; line: number }[];
    renderThread: (t: StoredThread, onResize: () => void) => ReactNode;
    items?: { key: string; render: (onResize: () => void) => ReactNode }[];
    comment?: {
      lines: Record<"a" | "b", ReadonlySet<number>>;
      onComment: (s: "a" | "b", a: number, l: number, e: boolean) => void;
    } | null;
  }) => (
    <div data-testid={`editor-${p.path}`}>
      {p.placements.map((pl) => (
        <div key={pl.thread.threadId}>
          {p.renderThread(pl.thread, () => undefined)}
        </div>
      ))}
      {(p.items ?? []).map((i) => (
        <div key={i.key}>{i.render(() => undefined)}</div>
      ))}
      {[...(p.comment?.lines.b ?? [])].map((n) => (
        <button
          key={n}
          type="button"
          onClick={() => p.comment?.onComment("b", n, n, false)}
        >
          {`Comment on new line ${n}`}
        </button>
      ))}
    </div>
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import App from "./App";
import type {
  AiRunState,
  Finding,
  NewComment,
  PostResult,
  ReviewRecord,
  StoredThread,
} from "./ipc/types";
import { useAiStore } from "./state/aiStore";
import { useBackendSettingsStore } from "./state/backendSettingsStore";
import { clearContentsCache, useCodeViewStore } from "./state/codeViewStore";
import { clearCommentableCache, useCommentStore } from "./state/commentStore";
import { useHistoryStore } from "./state/historyStore";
import { useLayoutStore } from "./state/layoutStore";
import { APPROVED_LINGER_MS, useReviewStore } from "./state/reviewStore";
import { useSettingsRequestStore } from "./state/settingsRequestStore";
import { useThreadStore } from "./state/threadStore";
import { useToastStore } from "./state/toastStore";
import { aiRun, finding } from "./test/aiFixtures";
import {
  file,
  review,
  reviewFiles,
  settings,
  status,
  thread,
} from "./test/fixtures";
import { historyFor } from "./test/historyFixtures";

const mockInvoke = vi.mocked(invoke);
const initial = {
  reviews: useReviewStore.getState(),
  threads: useThreadStore.getState(),
  code: useCodeViewStore.getState(),
  comments: useCommentStore.getState(),
  history: useHistoryStore.getState(),
  ai: useAiStore.getState(),
  settings: useBackendSettingsStore.getState(),
  layout: useLayoutStore.getState(),
};

const WRITES = [
  "post_comments",
  "reply_to_thread",
  "approve_review",
  "submit_review",
  "post_findings",
];

/** A backend with two reviews; `A` has a conversation and one file. */
function makeBackend(opts: { ai: boolean }) {
  const A = review({ title: "Fixture A" });
  const B = review({
    title: "Fixture B",
    status: "needsReReview",
    round: 2,
    roundReasons: ["authorReplied"],
    lastReviewedSha: "c".repeat(40),
    lastReviewedAt: "2026-10-15T08:00:00Z",
  });
  const reviews: ReviewRecord[] = [A, B];
  const general = thread({
    reviewId: A.id,
    author: "alice-fixture",
    anchor: null,
    createdAt: "2026-10-14T08:00:00Z",
    notes: [
      {
        id: "g1",
        author: "alice-fixture",
        body: "Ready for review.",
        createdAt: "2026-10-14T08:00:00Z",
        system: false,
      },
    ],
  });
  const outdated = thread({
    reviewId: A.id,
    mine: true,
    author: "rev-fixture",
    outdated: true,
    createdAt: "2026-10-15T08:00:00Z",
    anchor: { ...thread().anchor!, pathNew: "src/cache.rs", lineNew: 42 },
    notes: [
      {
        id: "o1",
        author: "rev-fixture",
        body: "Cache this lookup.",
        createdAt: "2026-10-15T08:00:00Z",
        system: false,
      },
      {
        id: "o2",
        author: "alice-fixture",
        body: "Moved it into the loader.",
        createdAt: "2026-10-15T09:00:00Z",
        system: false,
      },
    ],
  });
  const current = thread({
    reviewId: A.id,
    author: "bob-fixture",
    createdAt: "2026-10-15T10:00:00Z",
    anchor: { ...thread().anchor!, pathNew: "src/cache.rs", lineNew: 2 },
    notes: [
      {
        id: "c1",
        author: "bob-fixture",
        body: "Is the second line needed?",
        createdAt: "2026-10-15T10:00:00Z",
        system: false,
      },
    ],
  });
  const threads: Record<string, StoredThread[]> = {
    [A.id]: [general, outdated, current],
    [B.id]: [],
  };
  const files = reviewFiles({
    reviewId: A.id,
    baseSha: A.baseSha,
    headSha: A.headSha,
    files: [
      file({ path: "src/cache.rs", patch: "@@ -1,2 +1,3 @@\n a\n-b\n+B\n+c" }),
    ],
  });
  const findings: Finding[] = [
    finding({ reviewId: A.id, line: 3, title: "Unbounded cache" }),
  ];
  const run: AiRunState = aiRun({ reviewId: A.id, enabled: opts.ai });
  const posted: NewComment[][] = [];

  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_reviews: () => reviews.filter((r) => r.status !== "approved"),
    get_review: (a) => reviews.find((r) => r.id === a.reviewId),
    last_refresh: () => null,
    refresh_reviews: () => ({
      changed: [],
      errors: [],
      refreshedAt: new Date().toISOString(),
    }),
    get_settings: () =>
      settings({
        ai: { enabled: opts.ai, adapter: "claude", args: [], binary: null },
      }),
    provider_status: () => status(),
    gitlab_token_present: () => true,
    list_threads: (a) => threads[a.reviewId as string] ?? [],
    list_files: (a) =>
      a.reviewId === A.id ? files : reviewFiles({ files: [] }),
    file_contents: (a) => (a.sha === A.baseSha ? "a\nb\n" : "a\nB\nc\n"),
    commentable_lines: (a) => ({
      reviewId: a.reviewId,
      path: a.path,
      baseSha: A.baseSha,
      headSha: A.headSha,
      hunks: [{ old: [1, 2], new: [1, 2, 3] }],
    }),
    review_histories: (a) =>
      (threads[a.reviewId as string] ?? []).map((t) => historyFor(t)),
    thread_history: (a) =>
      historyFor(
        Object.values(threads)
          .flat()
          .find((t) => t.threadId === a.threadId)!,
      ),
    ai_state: (a) => ({ ...run, reviewId: a.reviewId }),
    list_findings: (a) => (opts.ai && a.reviewId === A.id ? findings : []),
    list_checklist: () => [],
    list_verdicts: () => [],
    post_comments: (a) => {
      const comments = a.comments as NewComment[];
      posted.push(comments);
      return comments.map((c, i): PostResult => {
        if (i > 0) {
          return {
            status: "failed",
            kind: "lineNotInDiff",
            message: "That line is not in the diff any more.",
          };
        }
        const t = thread({
          reviewId: A.id,
          mine: true,
          author: "rev-fixture",
          anchor: { ...thread().anchor!, lineNew: c.line },
          notes: [
            {
              id: "p1",
              author: "rev-fixture",
              body: c.body,
              createdAt: "2026-10-16T11:00:00Z",
              system: false,
            },
          ],
        });
        threads[A.id] = [...threads[A.id], t];
        setTimeout(() => events.emit("review:changed", { reviewId: A.id }));
        return {
          status: "posted",
          url: `${A.url}#discussion_r1`,
          providerThreadId: t.id,
          providerNoteId: "p1",
          path: c.path,
          side: c.side,
          line: c.line,
          startLine: null,
          headSha: A.headSha,
        };
      });
    },
    submit_review: (a) => {
      if (a.headSha !== A.headSha) {
        throw { kind: "headMoved", message: "The review got new commits." };
      }
      const i = reviews.findIndex((r) => r.id === a.reviewId);
      reviews[i] = {
        ...reviews[i],
        status: "approved",
        approvedAt: new Date().toISOString(),
      };
      setTimeout(() => events.emit("review:changed", { reviewId: A.id }));
      return reviews[i];
    },
    post_findings: (a) =>
      (a.posts as { findingId: string }[]).map((p) => ({
        findingId: p.findingId,
        status: "posted",
        postedUrl: `${A.url}#discussion_r2`,
        postedAt: new Date().toISOString(),
      })),
  };

  mockInvoke.mockImplementation(((
    cmd: string,
    args?: Record<string, unknown>,
  ) => {
    const h = handlers[cmd];
    if (!h) return Promise.resolve(null);
    try {
      return Promise.resolve(h(args ?? {}));
    } catch (e) {
      return Promise.reject(e);
    }
  }) as typeof invoke);

  return { A, B, handlers, posted };
}

const writes = () =>
  mockInvoke.mock.calls.filter(([cmd]) => WRITES.includes(cmd as string));

const sidebar = () => screen.getByRole("complementary", { name: "Sidebar" });
const detail = () => screen.getByRole("region", { name: "Review detail" });

async function openReview(title: string) {
  fireEvent.click(await within(sidebar()).findByText(title));
  return screen.findByRole("heading", { name: title });
}

// Each case walks many screens, every step waiting on the fake backend's
// promises and events; on a loaded machine (the whole suite in parallel, a
// cargo build next to it) the defaults (5 s per case, 1 s per wait) are too
// tight. Generous limits only slow down a failure, never a pass.
configure({ asyncUtilTimeout: 5_000 });

describe("App, end to end over a fake backend", { timeout: 30_000 }, () => {
  beforeEach(() => {
    localStorage.clear();
    events.clear();
    clearContentsCache();
    clearCommentableCache();
    useReviewStore.setState({ ...initial.reviews }, true);
    useThreadStore.setState({ ...initial.threads, byReview: {} }, true);
    useCodeViewStore.setState({ ...initial.code, byReview: {} }, true);
    useCommentStore.setState(
      { ...initial.comments, drafts: {}, composer: null, replies: {} },
      true,
    );
    useHistoryStore.setState({ ...initial.history }, true);
    useAiStore.setState({ ...initial.ai }, true);
    useBackendSettingsStore.setState({ ...initial.settings }, true);
    useLayoutStore.setState({ ...initial.layout, aiPanelOpen: true }, true);
    useSettingsRequestStore.getState().clear();
    useToastStore.setState({ toasts: [] });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("walks from the list to posted comments and an approval", async () => {
    const { A, posted } = makeBackend({ ai: false });
    render(<App />);

    // The list: A needs review; B is back with the reason why.
    expect(await within(sidebar()).findByText("Fixture A")).toBeInTheDocument();
    fireEvent.click(
      within(sidebar()).getByRole("tab", { name: /Needs re-review/ }),
    );
    expect(within(sidebar()).getByText("Fixture B")).toBeInTheDocument();
    expect(within(sidebar()).getByText("Author replied")).toBeInTheDocument();
    fireEvent.click(
      within(sidebar()).getByRole("tab", { name: /^Needs review/ }),
    );

    await openReview("Fixture A");
    expect(within(detail()).getByText("Needs review")).toBeInTheDocument();

    // The conversation, then the file with its threads.
    const conversation = screen.getByRole("region", { name: "Conversation" });
    expect(
      await within(conversation).findByText("Ready for review."),
    ).toBeInTheDocument();
    const diff = await screen.findByRole("region", {
      name: "Diff of src/cache.rs",
    });
    // The outdated thread: never hidden, every note readable.
    expect(await within(diff).findByText("Outdated")).toBeInTheDocument();
    expect(within(diff).getByText("Cache this lookup.")).toBeInTheDocument();
    expect(
      within(diff).getByText("Moved it into the loader."),
    ).toBeInTheDocument();
    // The current one, inline in the editor.
    expect(
      await within(diff).findByText("Is the second line needed?"),
    ).toBeInTheDocument();

    // Two drafts from the gutter; nothing is written yet.
    fireEvent.click(
      await within(diff).findByRole("button", {
        name: "Comment on new line 2",
      }),
    );
    fireEvent.change(
      await within(diff).findByRole("textbox", { name: /Comment on/ }),
      {
        target: { value: "Why uppercase?" },
      },
    );
    fireEvent.click(
      within(diff).getByRole("button", { name: "Add to review" }),
    );
    fireEvent.click(
      within(diff).getByRole("button", { name: "Comment on new line 3" }),
    );
    fireEvent.change(
      within(diff).getByRole("textbox", { name: /Comment on/ }),
      {
        target: { value: "And this one?" },
      },
    );
    fireEvent.click(
      within(diff).getByRole("button", { name: "Add to review" }),
    );
    expect(await screen.findByText("2 pending comments")).toBeInTheDocument();
    expect(writes()).toEqual([]);

    // Post all: one call, one result per draft; the refused one stays.
    fireEvent.click(screen.getByRole("button", { name: "Post all" }));
    await waitFor(() => expect(posted).toHaveLength(1));
    expect(posted[0].map((c) => [c.line, c.body])).toEqual([
      [2, "Why uppercase?"],
      [3, "And this one?"],
    ]);
    expect(await screen.findByText("1 pending comment")).toBeInTheDocument();
    const notifications = screen.getByRole("region", { name: "Notifications" });
    expect(
      await within(notifications).findByText("1 of 2 comments not posted"),
    ).toBeInTheDocument();
    expect(
      within(diff).getByText("That line is not in the diff any more."),
    ).toBeInTheDocument();
    // The posted one came back as a thread (the store re-read on the event).
    expect(await within(diff).findByText("Why uppercase?")).toBeInTheDocument();

    // Submit review → Approve, leaving the refused draft out: the review
    // shows "Approved ✓" for a moment, then leaves the list.
    fireEvent.click(screen.getByRole("button", { name: "Submit review…" }));
    const dialog = screen.getByRole("dialog", {
      name: `Submit review — acme-fixture/widgets#${A.number}`,
    });
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
    expect(
      await within(sidebar()).findByText("Approved ✓"),
    ).toBeInTheDocument();
    expect(within(sidebar()).getByText("Fixture A")).toBeInTheDocument();
    await waitFor(
      () => expect(within(sidebar()).queryByText("Fixture A")).toBeNull(),
      { timeout: APPROVED_LINGER_MS + 2000 },
    );
    expect(writes().map(([cmd]) => cmd)).toEqual([
      "post_comments",
      "submit_review",
    ]);
    expect(mockInvoke).toHaveBeenCalledWith("submit_review", {
      reviewId: A.id,
      headSha: A.headSha,
      verdict: "approve",
      summary: null,
    });
  });

  it("shows no AI pass at all with the AI pass off", async () => {
    makeBackend({ ai: false });
    render(<App />);
    await openReview("Fixture A");
    await screen.findByRole("region", { name: "Diff of src/cache.rs" });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("ai_state", expect.anything()),
    );
    expect(
      screen.queryByRole("complementary", { name: "AI review" }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: /Run again|Retry/ }),
    ).toBeNull();
    expect(screen.queryByText("Unbounded cache")).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalledWith("run_ai", expect.anything());
    // Tracking still shows: status and threads.
    expect(within(detail()).getByText("Needs review")).toBeInTheDocument();
    expect(
      within(detail()).getByText("Cache this lookup."),
    ).toBeInTheDocument();
  });

  it("shows the AI panel with the pass on; only Post selected posts", async () => {
    const { A } = makeBackend({ ai: true });
    render(<App />);
    await openReview("Fixture A");
    const panel = await screen.findByRole("complementary", {
      name: "AI review",
    });
    const row = await within(panel).findByRole("listitem", {
      name: "Finding: Unbounded cache",
    });
    expect(writes()).toEqual([]);
    fireEvent.click(
      within(row).getByRole("checkbox", {
        name: 'Select "Unbounded cache" for posting',
      }),
    );
    expect(writes()).toEqual([]);
    fireEvent.click(
      within(panel).getByRole("button", { name: "Post selected (1)" }),
    );
    await waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0][0]).toBe("post_findings");
    expect(writes()[0][1]).toMatchObject({ reviewId: A.id });
  });

  it("reports backend failures as error toasts", async () => {
    const { handlers } = makeBackend({ ai: false });
    handlers.refresh_reviews = () => {
      throw { kind: "githubUnavailable", message: "GitHub is not available." };
    };
    handlers.submit_review = () => {
      throw { kind: "headMoved", message: "The review got new commits." };
    };
    render(<App />);
    await within(sidebar()).findByText("Fixture A");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    const notifications = await screen.findByRole("region", {
      name: "Notifications",
    });
    const toast = await within(notifications).findByRole("alert");
    expect(toast).toHaveTextContent("Could not refresh");
    expect(toast).toHaveTextContent("GitHub is not available.");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Refresh" })).toBeEnabled(),
    );

    await openReview("Fixture A");
    fireEvent.click(screen.getByRole("button", { name: "Submit review…" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The review got new commits.",
    );
    expect(
      await within(notifications).findByText("Could not submit the review"),
    ).toBeInTheDocument();
    // Still listed: nothing was approved.
    await waitFor(() => expect(useReviewStore.getState().approving).toBeNull());
    expect(within(sidebar()).getByText("Fixture A")).toBeInTheDocument();
  });

  it("dismisses a toast and lets it expire", async () => {
    makeBackend({ ai: false });
    render(<App />);
    await within(sidebar()).findByText("Fixture A");
    vi.useFakeTimers();
    act(() => {
      useToastStore
        .getState()
        .push({ kind: "error", message: "Fixture failure" });
      useToastStore.getState().push({ kind: "info", message: "Fixture info" });
    });
    const notifications = screen.getByRole("region", { name: "Notifications" });
    const [first] = within(notifications).getAllByRole("button", {
      name: "Dismiss notification",
    });
    fireEvent.click(first);
    act(() => vi.advanceTimersByTime(1000));
    expect(within(notifications).queryByText("Fixture failure")).toBeNull();
    expect(within(notifications).getByText("Fixture info")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(6000));
    // The last one gone takes the region with it.
    expect(screen.queryByText("Fixture info")).toBeNull();
    expect(screen.queryByRole("region", { name: "Notifications" })).toBeNull();
  });
});
