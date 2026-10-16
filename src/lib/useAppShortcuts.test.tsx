import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// jsdom has no platform: pin one (Ctrl) so the tests do not depend on it.
vi.mock("./platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
}));

import { invoke } from "@tauri-apps/api/core";

import { useCommentStore } from "../state/commentStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { review } from "../test/fixtures";
import { useAppShortcuts } from "./useAppShortcuts";

const initial = useReviewStore.getState();
const a = review({ title: "A" });
const b = review({ title: "B" });

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

beforeEach(() => {
  localStorage.clear();
  useSettingsStore.getState().resetShortcuts();
  useReviewStore.setState({ ...initial, reviews: [a, b], loaded: true }, true);
});

describe("useAppShortcuts", () => {
  it("c comments on the focused line: it opens a composer, posts nothing", () => {
    useReviewStore.setState({ selectedId: a.id });
    useCommentStore.setState({ composer: null, focus: null });
    renderHook(() => useAppShortcuts());
    // No focused line: the key is left alone.
    expect(press({ key: "c", code: "KeyC" }).defaultPrevented).toBe(false);
    const target = {
      reviewId: a.id,
      path: "src/cache.rs",
      side: "new" as const,
      anchor: 4,
      line: 4,
      startLine: null,
      headSha: a.headSha,
      baseSha: a.baseSha,
    };
    useCommentStore.setState({
      focus: {
        reviewId: a.id,
        path: "src/cache.rs",
        side: "new",
        line: 4,
        target,
      },
    });
    expect(press({ key: "c", code: "KeyC" }).defaultPrevented).toBe(true);
    expect(useCommentStore.getState().composer).toMatchObject({
      line: 4,
      body: "",
    });
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });

  it("moves through the list, opens and switches lists", () => {
    renderHook(() => useAppShortcuts());
    press({ key: "ArrowDown" });
    press({ key: "ArrowDown" });
    expect(useReviewStore.getState().cursorId).toBe(b.id);
    press({ key: "Enter" });
    expect(useReviewStore.getState().selectedId).toBe(b.id);
    press({ key: "3" });
    expect(useReviewStore.getState().tab).toBe("waitingOnAuthor");
  });

  it("follows a rebinding", () => {
    renderHook(() => useAppShortcuts());
    act(() =>
      useSettingsStore.getState().setShortcut("showNeedsReReview", ["KeyN"]),
    );
    press({ key: "2" });
    expect(useReviewStore.getState().tab).toBe("needsReview");
    press({ key: "n" });
    expect(useReviewStore.getState().tab).toBe("needsReReview");
  });

  it("leaves the list keys to fields, dialogs and focused buttons", () => {
    renderHook(() => useAppShortcuts());
    const input = document.createElement("input");
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    const inDialog = document.createElement("span");
    dialog.appendChild(inDialog);
    const button = document.createElement("button");
    document.body.append(input, dialog, button);
    expect(press({ key: "3" }, input).defaultPrevented).toBe(false);
    expect(press({ key: "3" }, inDialog).defaultPrevented).toBe(false);
    expect(useReviewStore.getState().tab).toBe("needsReview");
    useReviewStore.setState({ cursorId: a.id });
    expect(press({ key: "Enter" }, button).defaultPrevented).toBe(false);
    expect(useReviewStore.getState().selectedId).toBeNull();
    input.remove();
    dialog.remove();
    button.remove();
  });

  it("opens Settings off macOS, also from a field", () => {
    const onOpenSettings = vi.fn();
    renderHook(() => useAppShortcuts({ onOpenSettings }));
    const e = press({ key: ",", code: "Comma", ctrlKey: true });
    expect(e.defaultPrevented).toBe(true);
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    const input = document.createElement("input");
    document.body.appendChild(input);
    press({ key: ",", code: "Comma", ctrlKey: true }, input);
    expect(onOpenSettings).toHaveBeenCalledTimes(2);
    input.remove();
  });

  it("swallows reload, print and find wherever the focus is", () => {
    renderHook(() => useAppShortcuts());
    const input = document.createElement("input");
    document.body.appendChild(input);
    for (const init of [
      { key: "F5" },
      { key: "R", ctrlKey: true, shiftKey: true },
      { key: "p", ctrlKey: true },
      { key: "f", ctrlKey: true },
    ]) {
      expect(press(init).defaultPrevented, JSON.stringify(init)).toBe(true);
      expect(press(init, input).defaultPrevented).toBe(true);
    }
    // Plain typing is left alone.
    expect(press({ key: "r" }, input).defaultPrevented).toBe(false);
    input.remove();
  });
});
