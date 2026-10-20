import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import type { StoredThread } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import { clearCommentableCache, useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useHistoryStore } from "../state/historyStore";
import { useLayoutStore } from "../state/layoutStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThreadStore } from "../state/threadStore";
import { useToastStore } from "../state/toastStore";
import { aiRun, finding } from "../test/aiFixtures";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import {
  commentOnFocusedItem,
  lineTarget,
  runFlowAction,
} from "./keyboardFlow";
import { runShortcut, useAppShortcuts } from "./useAppShortcuts";

const r = review();
const anchor = (path: string, line: number) => ({
  pathOld: path,
  pathNew: path,
  lineOld: null,
  lineNew: line,
  startLine: null,
  side: "new" as const,
  baseSha: null,
  startSha: null,
  headSha: r.headSha,
  diffHunk: null,
  currentLine: null,
});
const talk = thread({ reviewId: r.id, anchor: null, author: "carol-fixture" });
const onA = thread({ reviewId: r.id, anchor: anchor("src/a.rs", 3) });
const onB = thread({
  reviewId: r.id,
  mine: true,
  anchor: anchor("src/b.rs", 9),
});
const findA = finding({ reviewId: r.id, path: "src/a.rs", line: 5 });

/** The writes; none may run from these keys (but `s`, its own write). */
const WRITES = [
  "post_comments",
  "reply_to_thread",
  "approve_review",
  "post_findings",
  "resolve_thread",
];

/** The write commands invoked so far, with their arguments. */
const writes = () =>
  vi
    .mocked(invoke)
    .mock.calls.filter(([cmd]) => WRITES.includes(cmd as string));

function press(
  init: KeyboardEventInit & { key: string },
  target: EventTarget = window,
) {
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

function seed(threads: StoredThread[] = [talk, onA, onB]) {
  useReviewStore.setState({ reviews: [r], selectedId: r.id, cursorId: r.id });
  useCodeViewStore.setState({
    byReview: {
      [r.id]: {
        since: false,
        files: reviewFiles({
          reviewId: r.id,
          headSha: r.headSha,
          files: [file({ path: "src/a.rs" }), file({ path: "src/b.rs" })],
        }),
        loading: false,
        error: null,
        selected: null,
        order: [CONVERSATION, "src/a.rs", "src/b.rs"],
      },
    },
  });
  useThreadStore.setState({
    byReview: { [r.id]: { threads, loading: false, error: null } },
  });
  useAiStore.setState({
    runs: { [r.id]: aiRun({ reviewId: r.id, enabled: true }) },
    byReview: {
      [r.id]: {
        findings: [findA],
        checklist: [],
        verdicts: [],
        loading: false,
        error: null,
      },
    },
    ui: {},
    focused: null,
  });
  useFocusStore.setState({ reviewId: r.id, item: null, zone: "page" });
}

/** A review list row inside the sidebar, as the app renders it. */
function sidebar() {
  const aside = document.createElement("aside");
  aside.setAttribute("aria-label", "Sidebar");
  const inList = document.createElement("button");
  aside.appendChild(inList);
  document.body.appendChild(aside);
  return { inList, remove: () => aside.remove() };
}

const focusedName = () => {
  const item = useFocusStore.getState().item;
  if (!item) return null;
  if (item.kind === "thread") return `thread ${item.threadId}`;
  if (item.kind === "finding") return `finding ${item.findingId}`;
  if (item.kind === "file") return `file ${item.path}`;
  return item.kind;
};

beforeEach(() => {
  localStorage.clear();
  clearCommentableCache();
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd === "commentable_lines") {
      return Promise.resolve({
        reviewId: r.id,
        path: "src/a.rs",
        baseSha: r.baseSha,
        headSha: r.headSha,
        hunks: [{ old: [], new: [1, 2, 3, 4, 5] }],
      });
    }
    if (cmd === "dismiss_finding") return Promise.resolve(findA);
    return Promise.resolve([]);
  });
  useSettingsStore.getState().resetShortcuts();
  useCommentStore.setState({ composer: null, focus: null, replies: {} });
  useHistoryStore.setState({ focused: null, expanded: {}, open: {} });
  useOverlayStore.setState({ open: null, confirm: null });
  useLayoutStore.setState({ diffLayout: "unified" });
  seed();
});

describe("n / p: the page focus", () => {
  it("walks the threads and findings in page order", () => {
    renderHook(() => useAppShortcuts());
    const seen: (string | null)[] = [];
    for (let i = 0; i < 4; i++) {
      expect(press({ key: "n", code: "KeyN" }).defaultPrevented).toBe(true);
      seen.push(focusedName());
    }
    expect(seen).toEqual([
      `thread ${talk.threadId}`,
      `thread ${onA.threadId}`,
      `finding ${findA.id}`,
      `thread ${onB.threadId}`,
    ]);
    // The end of the page: the key is left alone.
    expect(press({ key: "n", code: "KeyN" }).defaultPrevented).toBe(false);
    press({ key: "p", code: "KeyP" });
    expect(focusedName()).toBe(`finding ${findA.id}`);
  });

  it("selects the item's card and keeps the per-kind focus in step", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    press({ key: "n", code: "KeyN" });
    expect(useCodeViewStore.getState().byReview[r.id].selected).toBe(
      "src/a.rs",
    );
    expect(useHistoryStore.getState().focused?.threadId).toBe(onA.threadId);
    expect(useAiStore.getState().focused).toBeNull();
    press({ key: "n", code: "KeyN" });
    expect(useAiStore.getState().focused).toBe(findA.id);
    expect(useHistoryStore.getState().focused).toBeNull();
  });

  it("j/k focus the file; n continues inside it", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "j", code: "KeyJ" });
    press({ key: "j", code: "KeyJ" });
    press({ key: "j", code: "KeyJ" });
    expect(focusedName()).toBe("file src/b.rs");
    press({ key: "n", code: "KeyN" });
    expect(focusedName()).toBe(`thread ${onB.threadId}`);
  });

  it("follows clicks: a thread, a finding, a diff line", () => {
    act(() =>
      useHistoryStore
        .getState()
        .focus({ threadId: onA.threadId, reviewId: r.id, mine: false }),
    );
    expect(focusedName()).toBe(`thread ${onA.threadId}`);
    act(() => useAiStore.getState().focus(findA.id));
    expect(focusedName()).toBe(`finding ${findA.id}`);
    act(() =>
      useCommentStore.getState().setFocus({
        reviewId: r.id,
        path: "src/a.rs",
        side: "new",
        line: 4,
        target: null,
      }),
    );
    expect(useFocusStore.getState().item).toMatchObject({
      kind: "line",
      line: 4,
    });
    expect(useAiStore.getState().focused).toBeNull();
    // From the clicked line n goes to the next item below it.
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    expect(focusedName()).toBe(`finding ${findA.id}`);
  });
});

describe("keys on the focused item", () => {
  it("h toggles the focused thread's then/now", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    press({ key: "n", code: "KeyN" });
    expect(press({ key: "h", code: "KeyH" }).defaultPrevented).toBe(true);
    expect(useHistoryStore.getState().expanded[onA.threadId]).toBe(true);
  });

  it("t and r ask the focused thread to open or reply", () => {
    renderHook(() => useAppShortcuts());
    // Nothing focused: left alone (r is no longer Refresh).
    expect(press({ key: "r", code: "KeyR" }).defaultPrevented).toBe(false);
    press({ key: "n", code: "KeyN" });
    press({ key: "t", code: "KeyT" });
    expect(useFocusStore.getState().command).toMatchObject({
      threadId: talk.threadId,
      action: "toggle",
    });
    press({ key: "r", code: "KeyR" });
    expect(useFocusStore.getState().command).toMatchObject({
      threadId: talk.threadId,
      action: "reply",
    });
  });

  it("x, e and d act on the focused finding; none posts", async () => {
    renderHook(() => useAppShortcuts());
    for (let i = 0; i < 3; i++) press({ key: "n", code: "KeyN" });
    expect(focusedName()).toBe(`finding ${findA.id}`);
    press({ key: "x", code: "KeyX" });
    expect(useAiStore.getState().ui[findA.id]?.selected).toBe(true);
    press({ key: "e", code: "KeyE" });
    expect(useAiStore.getState().ui[findA.id]?.editor).toContain(findA.title);
    press({ key: "d", code: "KeyD" });
    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith("dismiss_finding", {
        findingId: findA.id,
        dismissed: true,
      }),
    );
    for (const cmd of WRITES) {
      expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
        cmd,
        expect.anything(),
      );
    }
  });

  it("x on a thread does nothing; e and d need a finding", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    expect(press({ key: "x", code: "KeyX" }).defaultPrevented).toBe(false);
    expect(press({ key: "e", code: "KeyE" }).defaultPrevented).toBe(false);
    expect(press({ key: "d", code: "KeyD" }).defaultPrevented).toBe(false);
  });

  it("c opens a composer on the focused item's line, posting nothing", async () => {
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    press({ key: "n", code: "KeyN" });
    expect(press({ key: "c", code: "KeyC" }).defaultPrevented).toBe(true);
    await waitFor(() =>
      expect(useCommentStore.getState().composer).toMatchObject({
        path: "src/a.rs",
        line: 3,
        side: "new",
        body: "",
      }),
    );
    for (const cmd of WRITES) {
      expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
        cmd,
        expect.anything(),
      );
    }
  });
});

describe("since my last review", () => {
  it("c still opens a composer on a new line", async () => {
    // The compare's left side is the watermark; the commentable lines are
    // the review's own diff, at its base.
    const base = useCodeViewStore.getState().byReview[r.id]!;
    useCodeViewStore.setState({
      byReview: {
        [r.id]: {
          ...base,
          since: true,
          files: {
            ...base.files!,
            sinceLastReview: true,
            baseSha: "c".repeat(40),
            reviewBaseSha: r.baseSha,
          },
        },
      },
    });
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    press({ key: "n", code: "KeyN" });
    expect(press({ key: "c", code: "KeyC" }).defaultPrevented).toBe(true);
    await waitFor(() =>
      expect(useCommentStore.getState().composer).toMatchObject({
        path: "src/a.rs",
        line: 3,
        side: "new",
        baseSha: null,
      }),
    );
  });
});

describe("removed lines", () => {
  const withOldLines = () =>
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === "commentable_lines"
        ? Promise.resolve({
            reviewId: r.id,
            path: "src/a.rs",
            baseSha: r.baseSha,
            headSha: r.headSha,
            hunks: [{ old: [2], new: [1, 2, 3] }],
          })
        : Promise.resolve([]),
    );
  const approximate = () => {
    const base = useCodeViewStore.getState().byReview[r.id]!;
    useCodeViewStore.setState({
      byReview: {
        [r.id]: {
          ...base,
          files: { ...base.files!, baseApproximate: true },
        },
      },
    });
  };

  it("take a comment in the split view against an exact base", async () => {
    withOldLines();
    useLayoutStore.setState({ diffLayout: "split" });
    await expect(lineTarget(r.id, "src/a.rs", "old", 2)).resolves.toMatchObject(
      { side: "old", anchor: 2, baseSha: r.baseSha },
    );
  });

  it("take none while the base is only approximate, as in the code view", async () => {
    withOldLines();
    useLayoutStore.setState({ diffLayout: "split" });
    approximate();
    await expect(lineTarget(r.id, "src/a.rs", "old", 2)).resolves.toBeNull();
    // The new side does not depend on the left side.
    await expect(lineTarget(r.id, "src/a.rs", "new", 2)).resolves.toMatchObject(
      { side: "new", anchor: 2 },
    );

    // `c` on a focused old-side thread: a notice, no composer, no post.
    useToastStore.setState({ toasts: [] });
    useFocusStore.setState({
      reviewId: r.id,
      item: {
        kind: "thread",
        threadId: onA.id,
        mine: false,
        card: "src/a.rs",
        line: 2,
        side: "old",
      },
    });
    expect(commentOnFocusedItem(r.id)).toBe(true);
    await waitFor(() =>
      expect(useToastStore.getState().toasts).toHaveLength(1),
    );
    expect(useCommentStore.getState().composer).toBeNull();
    for (const cmd of WRITES) {
      expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
        cmd,
        expect.anything(),
      );
    }
  });
});

describe("guards", () => {
  it("c opens nothing once another review is open", async () => {
    let answer: (v: unknown) => void = () => {};
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === "commentable_lines"
        ? new Promise((r) => (answer = r))
        : Promise.resolve([]),
    );
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    press({ key: "n", code: "KeyN" });
    expect(press({ key: "c", code: "KeyC" }).defaultPrevented).toBe(true);
    // The lines arrive after the user moved on.
    act(() => useReviewStore.setState({ selectedId: "another-review" }));
    await act(async () => {
      answer({
        reviewId: r.id,
        path: "src/a.rs",
        baseSha: r.baseSha,
        headSha: r.headSha,
        hunks: [{ old: [], new: [1, 2, 3, 4, 5] }],
      });
      await Promise.resolve();
    });
    expect(useCommentStore.getState().composer).toBeNull();
  });

  it("held keys repeat moves only, never toggles", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "j", code: "KeyJ" });
    press({ key: "j", code: "KeyJ", repeat: true });
    expect(useCodeViewStore.getState().byReview[r.id]?.selected).toBe(
      "src/a.rs",
    );
    vi.mocked(invoke).mockClear();
    press({ key: "v", code: "KeyV" });
    press({ key: "v", code: "KeyV", repeat: true });
    press({ key: "v", code: "KeyV", repeat: true });
    expect(
      vi.mocked(invoke).mock.calls.filter(([c]) => c === "set_file_viewed"),
    ).toHaveLength(1);
  });

  it("no page key fires while typing or with a modal open", () => {
    renderHook(() => useAppShortcuts());
    const input = document.createElement("textarea");
    document.body.appendChild(input);
    expect(press({ key: "n", code: "KeyN" }, input).defaultPrevented).toBe(
      false,
    );
    expect(focusedName()).toBeNull();
    input.remove();

    const modal = document.createElement("div");
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-modal", "true");
    document.body.appendChild(modal);
    // The focus fell out of the dialog: still the dialog's keys.
    expect(press({ key: "n", code: "KeyN" }).defaultPrevented).toBe(false);
    expect(
      press({ key: "k", code: "KeyK", ctrlKey: true }).defaultPrevented,
    ).toBe(false);
    expect(useOverlayStore.getState().open).toBeNull();
    modal.remove();
  });

  it("follows a rebinding by action id", () => {
    renderHook(() => useAppShortcuts());
    act(() =>
      useSettingsStore.getState().setShortcut("nextItem", ["BracketRight"]),
    );
    expect(press({ key: "n", code: "KeyN" }).defaultPrevented).toBe(false);
    press({ key: "]", code: "BracketRight" });
    expect(focusedName()).toBe(`thread ${talk.threadId}`);
  });

  it("⌘K / Ctrl+K opens the palette, ? the help", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "k", code: "KeyK", ctrlKey: true });
    expect(useOverlayStore.getState().open).toBe("palette");
    act(() => useOverlayStore.getState().close());
    press({ key: "?", code: "Slash", shiftKey: true });
    expect(useOverlayStore.getState().open).toBe("help");
  });

  it("Refresh moved to ⇧R; a saved override still wins", async () => {
    renderHook(() => useAppShortcuts());
    press({ key: "R", code: "KeyR", shiftKey: true });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("refresh_reviews");
    await waitFor(() =>
      expect(useReviewStore.getState().refreshing).toBe(false),
    );
    vi.mocked(invoke).mockClear();
    act(() => useSettingsStore.getState().setShortcut("refresh", ["KeyR"]));
    // No thread focused: r falls through to the user's own Refresh key.
    press({ key: "r", code: "KeyR" });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("refresh_reviews");
  });
});

describe("Enter and the zone", () => {
  it("↑/↓ in the list hands Enter back to the list", () => {
    useReviewStore.setState({ reviews: [r, review()] });
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    expect(useFocusStore.getState().zone).toBe("page");
    // From the sidebar: the list's ↓, not the page's.
    const { inList, remove } = sidebar();
    press({ key: "ArrowDown" }, inList);
    expect(useFocusStore.getState().zone).toBe("list");
    remove();
    press({ key: "n", code: "KeyN" });
    expect(useFocusStore.getState().zone).toBe("page");
  });

  it("leaves Enter and Space to a focused button; other keys still toggle", () => {
    act(() =>
      useHistoryStore
        .getState()
        .focus({ threadId: onA.threadId, reviewId: r.id, mine: false }),
    );
    const button = document.createElement("button");
    document.body.appendChild(button);
    expect(runFlowAction("toggleThread", r.id, button, "Enter")).toBe(false);
    expect(runFlowAction("toggleThread", r.id, button, " ")).toBe(false);
    expect(runFlowAction("toggleThread", r.id, button, "t")).toBe(true);
    expect(runFlowAction("toggleThread", r.id, window, " ")).toBe(true);
    button.remove();
  });
});

describe("next / previous unresolved thread", () => {
  const openTalk = { ...talk, resolvable: true };
  const resolved = { ...onA, resolvable: true, resolved: true };
  const openB = { ...onB, resolvable: true };
  const later = thread({
    reviewId: r.id,
    resolvable: true,
    anchor: anchor("src/b.rs", 20),
  });
  // A user's own binding: the actions have no default key.
  const bindG = () =>
    act(() => {
      useSettingsStore.getState().setShortcut("nextOpenThread", ["KeyG"]);
      useSettingsStore.getState().setShortcut("prevOpenThread", ["Shift+KeyG"]);
    });
  const next = () => press({ key: "g", code: "KeyG" });
  const prev = () => press({ key: "G", code: "KeyG", shiftKey: true });

  it("have no default key: bare ↑/↓ on the review page scroll natively", () => {
    seed([openTalk, openB]);
    useReviewStore.setState({ reviews: [r, review()], cursorId: r.id });
    renderHook(() => useAppShortcuts());
    const detail = document.createElement("section");
    detail.setAttribute("aria-label", "Review detail");
    const inPage = document.createElement("button");
    detail.appendChild(inPage);
    document.body.appendChild(detail);
    for (const target of [inPage, window]) {
      const e = press({ key: "ArrowDown" }, target);
      // Left to the browser: no thread focus, no list cursor move.
      expect(e.defaultPrevented).toBe(false);
      expect(focusedName()).toBeNull();
      expect(useReviewStore.getState().cursorId).toBe(r.id);
    }
    expect(press({ key: "ArrowUp" }, inPage).defaultPrevented).toBe(false);
    detail.remove();
  });

  it("step through the unresolved threads in page order, no wrap", () => {
    seed([openTalk, resolved, openB, later, onA]);
    bindG();
    renderHook(() => useAppShortcuts());
    const seen: (string | null)[] = [];
    for (let i = 0; i < 3; i++) {
      expect(next().defaultPrevented).toBe(true);
      seen.push(focusedName());
    }
    // The resolved thread, the plain comment (`onA` here: nothing to
    // resolve) and the finding are skipped; mine and others' count.
    expect(seen).toEqual([
      `thread ${openTalk.threadId}`,
      `thread ${openB.threadId}`,
      `thread ${later.threadId}`,
    ]);
    // Opened, its card selected.
    expect(useHistoryStore.getState().open[later.threadId]).toBe(true);
    expect(useCodeViewStore.getState().byReview[r.id].selected).toBe(
      "src/b.rs",
    );
    // The end: it stays and says so once.
    expect(next().defaultPrevented).toBe(true);
    press({ key: "g", code: "KeyG", repeat: true });
    expect(focusedName()).toBe(`thread ${later.threadId}`);
    expect(
      useToastStore
        .getState()
        .toasts.filter((t) => t.message === "No more open threads"),
    ).toHaveLength(1);
    prev();
    expect(focusedName()).toBe(`thread ${openB.threadId}`);
    // The palette and the header's buttons run the same action.
    expect(runShortcut("prevOpenThread")).toBe(true);
    expect(focusedName()).toBe(`thread ${openTalk.threadId}`);
    expect(writes()).toEqual([]);
  });

  it("count an outdated thread that is still open", () => {
    const old = { ...onA, resolvable: true, outdated: true };
    seed([old, { ...openB, resolved: true }]);
    renderHook(() => useAppShortcuts());
    runShortcut("nextOpenThread");
    expect(focusedName()).toBe(`thread ${old.threadId}`);
    runShortcut("nextOpenThread");
    expect(focusedName()).toBe(`thread ${old.threadId}`);
  });

  it("leave the review list's ↑/↓ alone", () => {
    const other = review();
    seed([openTalk]);
    useReviewStore.setState({ reviews: [r, other], cursorId: r.id });
    renderHook(() => useAppShortcuts());
    // From the sidebar: the list cursor moves, the page focus does not.
    const { inList, remove } = sidebar();
    expect(press({ key: "ArrowDown" }, inList).defaultPrevented).toBe(true);
    expect(useReviewStore.getState().cursorId).toBe(other.id);
    expect(focusedName()).toBeNull();
    press({ key: "ArrowUp" }, inList);
    expect(useReviewStore.getState().cursorId).toBe(r.id);
    remove();
    // After ↑/↓ in the list, a press from nowhere in particular is the
    // list's again.
    press({ key: "ArrowDown" });
    expect(useReviewStore.getState().cursorId).toBe(other.id);
  });

  it("never fire while typing or inside a modal", () => {
    seed([openTalk]);
    bindG();
    renderHook(() => useAppShortcuts());
    const input = document.createElement("input");
    document.body.appendChild(input);
    press({ key: "g", code: "KeyG" }, input);
    expect(focusedName()).toBeNull();
    input.remove();
    const modal = document.createElement("div");
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-modal", "true");
    document.body.appendChild(modal);
    next();
    expect(focusedName()).toBeNull();
    modal.remove();
  });
});

describe("s: resolve the focused thread", () => {
  const open = thread({
    reviewId: r.id,
    resolvable: true,
    anchor: anchor("src/a.rs", 3),
  });
  const next = thread({
    reviewId: r.id,
    resolvable: true,
    anchor: anchor("src/b.rs", 9),
  });

  /** A backend that resolves: `list_threads` reports what it was told. */
  function resolvingBackend(threads: StoredThread[]) {
    let current = threads;
    vi.mocked(invoke).mockImplementation(
      (cmd: string, raw?: unknown): Promise<unknown> => {
        const args = raw as Record<string, unknown> | undefined;
        if (cmd === "resolve_thread") {
          current = current.map((t) =>
            t.threadId === args?.threadId
              ? { ...t, resolved: args?.resolved as boolean }
              : t,
          );
          return Promise.resolve(
            current.find((t) => t.threadId === args?.threadId),
          );
        }
        if (cmd === "list_threads") return Promise.resolve(current);
        return Promise.resolve([]);
      },
    );
  }

  it("resolves it once, and ↓ then goes to the next open one", async () => {
    seed([talk, open, next]);
    resolvingBackend([talk, open, next]);
    renderHook(() => useAppShortcuts());
    runShortcut("nextOpenThread");
    expect(focusedName()).toBe(`thread ${open.threadId}`);
    expect(press({ key: "s", code: "KeyS" }).defaultPrevented).toBe(true);
    // Held, it does not flap.
    press({ key: "s", code: "KeyS", repeat: true });
    await waitFor(() =>
      expect(
        useThreadStore
          .getState()
          .byReview[r.id].threads.find((t) => t.threadId === open.threadId)
          ?.resolved,
      ).toBe(true),
    );
    expect(writes()).toEqual([
      [
        "resolve_thread",
        { reviewId: r.id, threadId: open.threadId, resolved: true },
      ],
    ]);
    runShortcut("nextOpenThread");
    expect(focusedName()).toBe(`thread ${next.threadId}`);
    // Back up skips the one just resolved (and `talk`: nothing to resolve).
    expect(runShortcut("prevOpenThread")).toBe(true);
    expect(focusedName()).toBe(`thread ${next.threadId}`);
  });

  it("does nothing on a thread its platform cannot resolve", () => {
    seed([talk, open]);
    renderHook(() => useAppShortcuts());
    press({ key: "n", code: "KeyN" });
    expect(focusedName()).toBe(`thread ${talk.threadId}`);
    expect(press({ key: "s", code: "KeyS" }).defaultPrevented).toBe(false);
    expect(writes()).toEqual([]);
  });
});
