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
import { aiRun, finding } from "../test/aiFixtures";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { useAppShortcuts } from "./useAppShortcuts";

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

/** The four writes; none may run from these keys. */
const WRITES = [
  "post_comments",
  "reply_to_thread",
  "approve_review",
  "post_findings",
];

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

describe("guards", () => {
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
    press({ key: "ArrowDown" });
    expect(useFocusStore.getState().zone).toBe("list");
    press({ key: "n", code: "KeyN" });
    expect(useFocusStore.getState().zone).toBe("page");
  });
});
