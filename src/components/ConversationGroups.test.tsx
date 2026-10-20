/**
 * The Conversation's resolved / unresolved split: its groups, the counts in
 * the file tree and the review header, a resolve moving a thread across, and
 * the keys over a folded Resolved group. The whole detail island over a fake
 * backend (fictitious data only).
 */

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

import type { StoredThread } from "../ipc/types";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { clearCommentableCache, useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useHistoryStore } from "../state/historyStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { useToastStore } from "../state/toastStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { ReviewDetail } from "./ReviewDetail";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialThreads = useThreadStore.getState();
const initialCode = useCodeViewStore.getState();
const initialHistory = useHistoryStore.getState();
const NOW = Date.parse("2026-10-16T12:00:00Z");

const r = review({ provider: "gitlab", title: "Fixture cache change" });
const at = (path: string, line: number) => ({
  ...thread().anchor!,
  pathNew: path,
  pathOld: path,
  lineNew: line,
});
const note = (id: string, author: string, body: string) => ({
  id,
  author,
  body,
  createdAt: "2026-10-14T09:00:00Z",
  system: false,
});

const mineOpen = thread({
  reviewId: r.id,
  mine: true,
  resolvable: true,
  author: "rev-fixture",
  anchor: null,
  notes: [note("n-mine", "rev-fixture", "Please add a changelog entry.")],
});
const carolOpen = thread({
  reviewId: r.id,
  resolvable: true,
  author: "carol-fixture",
  anchor: null,
});
const daveComment = thread({
  reviewId: r.id,
  author: "dave-fixture",
  anchor: null,
});
const erinDone = thread({
  reviewId: r.id,
  resolvable: true,
  resolved: true,
  author: "erin-fixture",
  anchor: null,
  notes: [note("n-erin", "erin-fixture", "Settled, thanks.")],
});
const awayOpen = thread({
  reviewId: r.id,
  resolvable: true,
  author: "frank-fixture",
  anchor: at("src/gone.rs", 3),
});
const awayDone = thread({
  reviewId: r.id,
  resolvable: true,
  resolved: true,
  author: "gina-fixture",
  anchor: at("src/gone.rs", 5),
});
const onCache = thread({
  reviewId: r.id,
  resolvable: true,
  author: "hank-fixture",
  anchor: at("src/cache.rs", 2),
});
const all = [
  carolOpen,
  erinDone,
  daveComment,
  mineOpen,
  awayDone,
  awayOpen,
  onCache,
];

const files = reviewFiles({
  reviewId: r.id,
  files: [file({ path: "src/cache.rs" })],
});

let threads: StoredThread[];

beforeEach(() => {
  threads = all;
  clearContentsCache();
  clearCommentableCache();
  localStorage.clear();
  useCommentStore.setState({
    drafts: {},
    composer: null,
    replies: {},
    posting: {},
    focus: null,
  });
  mockInvoke.mockImplementation(((cmd: string, args?: unknown) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "list_threads":
        return Promise.resolve(threads);
      case "list_files":
        return Promise.resolve(files);
      case "file_contents":
        return Promise.resolve("a\nb\nc");
      case "resolve_thread": {
        // The platform's state after the write; the re-read shows it.
        threads = threads.map((t) =>
          t.threadId === a.threadId ? { ...t, resolved: !!a.resolved } : t,
        );
        return Promise.resolve(threads.find((t) => t.threadId === a.threadId));
      }
      default:
        // What ReviewDetail.test answers too: a record where one is read.
        return Promise.resolve(r);
    }
  }) as typeof invoke);
  useCodeViewStore.setState({ ...initialCode, byReview: {} }, true);
  useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
  useHistoryStore.setState({ ...initialHistory, open: {}, focused: null });
  useReviewStore.setState(
    { ...initialReviews, reviews: [r], loaded: true, selectedId: r.id },
    true,
  );
  useFocusStore.setState({ reviewId: r.id, item: null, zone: "page" });
  useSettingsStore.getState().resetShortcuts();
  useToastStore.setState({ toasts: [] });
});

function Shell() {
  useAppShortcuts();
  return <ReviewDetail now={NOW} />;
}

const conversation = () => screen.getByRole("region", { name: "Conversation" });
/** The Conversation's own part (not "On other files"). */
const talk = () => screen.getByTestId("conversation-threads");
const away = () => screen.getByTestId("elsewhere-threads");
const group = (root: HTMLElement, name: string) =>
  within(root).getByRole("group", { name });
const authors = (root: HTMLElement) =>
  within(root)
    .queryAllByRole("article")
    .map((a) => a.getAttribute("aria-label"));
const tree = () => screen.getByRole("tree", { name: "Changed files" });

async function opened() {
  render(<Shell />);
  await within(conversation()).findAllByRole("article");
  await screen.findByTestId("editor-src/cache.rs");
}

describe("the Conversation's resolved split", () => {
  it("groups unresolved, then comments, then resolved (folded)", async () => {
    await opened();
    const unresolved = group(talk(), "Unresolved");
    expect(authors(unresolved)).toEqual([
      "Thread by rev-fixture",
      "Thread by carol-fixture",
    ]);
    expect(within(unresolved).getByLabelText("2 threads")).toBeVisible();
    // Nothing to resolve: their own group, not counted as unresolved.
    expect(authors(group(talk(), "Comments"))).toEqual([
      "Thread by dave-fixture",
    ]);
    // Resolved: folded by default, its count on the toggle.
    const resolved = group(talk(), "Resolved");
    const toggle = within(resolved).getByRole("button", { name: /^Resolved/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(within(resolved).getByLabelText("1 thread")).toBeVisible();
    expect(authors(resolved)).toEqual([]);
    expect(screen.queryByText("Settled, thanks.")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(authors(resolved)).toEqual(["Thread by erin-fixture"]);
    // Fully readable once opened.
    fireEvent.click(within(resolved).getByRole("button", { expanded: false }));
    expect(screen.getByText("Settled, thanks.")).toBeInTheDocument();

    // "On other files" splits the same way, with its own fold.
    expect(authors(group(away(), "Unresolved"))).toEqual([
      "Thread by frank-fixture",
    ]);
    expect(
      within(away()).queryByRole("group", { name: "Comments" }),
    ).toBeNull();
    const awayResolved = group(away(), "Resolved");
    expect(authors(awayResolved)).toEqual([]);
    fireEvent.click(
      within(awayResolved).getByRole("button", { name: /^Resolved/ }),
    );
    expect(authors(awayResolved)).toEqual(["Thread by gina-fixture"]);
  });

  it("counts the open threads in the tree and the header", async () => {
    await opened();
    // The Conversation entry: its unresolved threads (the file's are not).
    const entry = within(tree()).getByRole("button", { name: /Conversation/ });
    expect(within(entry).getByText("3 open")).toBeInTheDocument();
    // The header: every thread of the review, anchored ones included.
    expect(screen.getByText("4 unresolved threads")).toBeVisible();
  });

  it("says all threads are resolved, and nothing without resolvable ones", async () => {
    threads = all.map((t) => ({ ...t, resolved: t.resolvable }));
    const { unmount } = render(<Shell />);
    expect(await screen.findByText("All threads resolved")).toBeVisible();
    // Nothing to go to: no arrows, only the line.
    expect(
      screen.queryByRole("button", { name: "Next unresolved thread" }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Previous unresolved thread" }),
    ).toBeNull();
    const entry = within(tree()).getByRole("button", { name: /Conversation/ });
    expect(within(entry).queryByText(/open$/)).toBeNull();
    // Nothing is unresolved: no Unresolved group at all.
    expect(
      within(talk()).queryByRole("group", { name: "Unresolved" }),
    ).toBeNull();
    unmount();

    useThreadStore.setState({ ...initialThreads, byReview: {} }, true);
    threads = [daveComment];
    render(<Shell />);
    await within(conversation()).findAllByRole("article");
    expect(screen.queryByText("All threads resolved")).toBeNull();
    expect(screen.queryByText(/unresolved thread/)).toBeNull();
  });

  it("moves a resolved thread to Resolved once the re-read lands", async () => {
    await opened();
    fireEvent.click(
      within(group(talk(), "Unresolved")).getByRole("button", {
        name: "Resolve carol-fixture's thread",
      }),
    );
    await waitFor(() =>
      expect(authors(group(talk(), "Unresolved"))).toEqual([
        "Thread by rev-fixture",
      ]),
    );
    const resolved = group(talk(), "Resolved");
    expect(within(resolved).getByLabelText("2 threads")).toBeVisible();
    expect(
      within(
        within(tree()).getByRole("button", { name: /Conversation/ }),
      ).getByText("2 open"),
    ).toBeInTheDocument();
    expect(screen.getByText("3 unresolved threads")).toBeVisible();

    // And back: reopened, it returns to the top group.
    fireEvent.click(
      within(resolved).getByRole("button", { name: /^Resolved/ }),
    );
    fireEvent.click(
      within(resolved).getByRole("button", {
        name: "Unresolve carol-fixture's thread",
      }),
    );
    await waitFor(() =>
      expect(authors(group(talk(), "Unresolved"))).toEqual([
        "Thread by rev-fixture",
        "Thread by carol-fixture",
      ]),
    );
    expect(screen.getByText("4 unresolved threads")).toBeVisible();
  });
});

describe("the keys over a folded Resolved group", () => {
  const press = (key: string, code?: string) =>
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key, code, bubbles: true }),
      );
    });
  const focused = () => {
    const item = useFocusStore.getState().item;
    return item?.kind === "thread" ? item.threadId : (item?.kind ?? null);
  };

  it("n unfolds Resolved when it lands on a thread in it; p as well", async () => {
    await opened();
    const seen: (string | null)[] = [];
    for (let i = 0; i < 4; i++) {
      press("n", "KeyN");
      seen.push(focused());
    }
    // Page order is the card's: unresolved, comments, then resolved.
    expect(seen).toEqual([
      mineOpen.threadId,
      carolOpen.threadId,
      daveComment.threadId,
      erinDone.threadId,
    ]);
    const resolved = group(talk(), "Resolved");
    expect(
      within(resolved).getByRole("button", { name: /^Resolved/ }),
    ).toHaveAttribute("aria-expanded", "true");
    const erin = await within(resolved).findByRole("article", {
      name: "Thread by erin-fixture",
    });
    expect(erin).toHaveAttribute("data-focused", "true");
    await waitFor(() => expect(document.activeElement).toBe(erin));

    // "On other files": n goes on into its folded Resolved group too.
    press("n", "KeyN");
    press("n", "KeyN");
    expect(focused()).toBe(awayDone.threadId);
    const awayResolved = group(away(), "Resolved");
    expect(
      within(awayResolved).getByRole("button", { name: /^Resolved/ }),
    ).toHaveAttribute("aria-expanded", "true");

    // Folded again by hand, p comes back into it and unfolds it again.
    fireEvent.click(
      within(resolved).getByRole("button", { name: /^Resolved/ }),
    );
    expect(authors(resolved)).toEqual([]);
    press("p", "KeyP");
    press("p", "KeyP");
    expect(focused()).toBe(erinDone.threadId);
    expect(authors(resolved)).toEqual(["Thread by erin-fixture"]);
  });

  it("the header's arrows walk the unresolved threads, Resolved stays folded", async () => {
    await opened();
    const down = screen.getByRole("button", { name: "Next unresolved thread" });
    const up = screen.getByRole("button", {
      name: "Previous unresolved thread",
    });
    expect(down).toBeEnabled();
    expect(down).toHaveAttribute("title", "Next unresolved thread");
    const seen: (string | null)[] = [];
    for (let i = 0; i < 4; i++) {
      fireEvent.click(down);
      seen.push(focused());
    }
    // Only what the count counts: resolved ones and the plain comment are
    // skipped, here and in "On other files".
    expect(seen).toEqual([
      mineOpen.threadId,
      carolOpen.threadId,
      awayOpen.threadId,
      onCache.threadId,
    ]);
    // At the end it stays and says so.
    fireEvent.click(down);
    expect(focused()).toBe(onCache.threadId);
    expect(
      useToastStore
        .getState()
        .toasts.some((t) => t.message === "No more open threads"),
    ).toBe(true);
    fireEvent.click(up);
    expect(focused()).toBe(awayOpen.threadId);
    expect(
      within(group(talk(), "Resolved")).getByRole("button", {
        name: /^Resolved/,
      }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(
      within(group(away(), "Resolved")).getByRole("button", {
        name: /^Resolved/,
      }),
    ).toHaveAttribute("aria-expanded", "false");
    // Nothing was written.
    expect(
      mockInvoke.mock.calls.filter(([cmd]) => cmd === "resolve_thread"),
    ).toEqual([]);
  });

  it("names a saved shortcut in the arrows' tooltips", async () => {
    act(() =>
      useSettingsStore.getState().setShortcut("nextOpenThread", ["KeyG"]),
    );
    await opened();
    expect(
      screen.getByRole("button", { name: "Next unresolved thread" }),
    ).toHaveAttribute("title", "Next unresolved thread (G)");
    // A bound key runs the same action.
    press("g", "KeyG");
    expect(focused()).toBe(mineOpen.threadId);
  });

  it("bare ↓ leaves the thread focus alone", async () => {
    await opened();
    press("ArrowDown");
    expect(focused()).toBeNull();
  });
});
