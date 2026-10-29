import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Ctrl is Mod here.
vi.mock("../lib/platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import { ACTIONS, DEFAULT_KEYMAP } from "../lib/keymap";
import { CONVERSATION_CARD } from "../lib/focusModel";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { resetFindForTests, useFindStore } from "../state/findStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { FindBar } from "./FindBar";

const r = review();

function note(id: string, body: string) {
  return {
    id,
    author: "carol-fixture",
    body,
    createdAt: "2026-10-15T09:00:00Z",
    system: false,
  };
}

function Page({ filter = false }: { filter?: boolean }) {
  useAppShortcuts();
  return (
    <>
      <aside aria-label="Sidebar">
        <button type="button">A review row</button>
        {filter && <input type="search" data-list-filter aria-label="Filter" />}
      </aside>
      <textarea aria-label="Comment" defaultValue="a cache comment" />
      <FindBar />
    </>
  );
}

function press(target: Element | Window, init: KeyboardEventInit) {
  const e = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
}

const modF = { key: "f", code: "KeyF", ctrlKey: true };
const field = () => screen.getByRole("textbox", { name: "Find in review" });

async function results(n: number) {
  await vi.waitFor(() => {
    const s = useFindStore.getState();
    expect(s.searching).toBe(false);
    expect(s.matches).toHaveLength(n);
  });
}

beforeEach(() => {
  resetFindForTests();
  clearContentsCache();
  localStorage.clear();
  useSettingsStore.getState().resetShortcuts();
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(() => Promise.resolve(""));
  useReviewStore.setState({ reviews: [r], selectedId: r.id });
  useCodeViewStore.setState({
    byReview: {
      [r.id]: {
        since: false,
        files: reviewFiles({ reviewId: r.id, files: [file({ path: "a.rs" })] }),
        loading: false,
        error: null,
        selected: null,
        order: [CONVERSATION_CARD, "a.rs"],
      },
    },
  });
  useThreadStore.setState({
    byReview: {
      [r.id]: {
        threads: [
          thread({
            reviewId: r.id,
            anchor: null,
            notes: [note("n1", "cache one"), note("n2", "cache two")],
          }),
        ],
        loading: false,
        error: null,
      },
    },
  });
});

afterEach(() => {
  act(() => resetFindForTests());
});

describe("the find bar", () => {
  it("opens on Mod+F from a comment's field, prefilled with its selection; the comment keeps its text", async () => {
    render(<Page />);
    const comment = screen.getByRole("textbox", { name: "Comment" });
    comment.focus();
    (comment as HTMLTextAreaElement).setSelectionRange(2, 7);
    expect(press(comment, modF).defaultPrevented).toBe(true);
    expect(field()).toHaveValue("cache");
    expect(field()).toHaveFocus();
    expect(comment).toHaveValue("a cache comment");
    await results(2);
    expect(screen.getByText("1 of 2")).toBeInTheDocument();
  });

  it("Enter and Shift+Enter go next and previous, wrapping", async () => {
    render(<Page />);
    press(window, modF);
    fireEvent.change(field(), { target: { value: "cache" } });
    await results(2);
    expect(useFindStore.getState().current).toBe(0);
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(useFindStore.getState().current).toBe(1);
    fireEvent.keyDown(field(), { key: "Enter" });
    expect(useFindStore.getState().current).toBe(0);
    expect(screen.getByText("Wrapped")).toBeInTheDocument();
    fireEvent.keyDown(field(), { key: "Enter", shiftKey: true });
    expect(useFindStore.getState().current).toBe(1);
  });

  it("Mod+G and Mod+Shift+G go next and previous only while the bar is open", async () => {
    render(<Page />);
    press(window, { key: "g", code: "KeyG", ctrlKey: true });
    expect(useFindStore.getState().open).toBe(false);
    press(window, modF);
    fireEvent.change(field(), { target: { value: "cache" } });
    await results(2);
    expect(
      press(field(), { key: "g", code: "KeyG", ctrlKey: true })
        .defaultPrevented,
    ).toBe(true);
    expect(useFindStore.getState().current).toBe(1);
    press(field(), { key: "G", code: "KeyG", ctrlKey: true, shiftKey: true });
    expect(useFindStore.getState().current).toBe(0);
  });

  it("leaves Mod+G to a comment that handles it (its own key wins)", async () => {
    render(<Page />);
    press(window, modF);
    fireEvent.change(field(), { target: { value: "cache" } });
    await results(2);
    const comment = screen.getByRole("textbox", { name: "Comment" });
    comment.addEventListener("keydown", (e) => e.preventDefault(), {
      once: true,
    });
    press(comment, { key: "g", code: "KeyG", ctrlKey: true });
    expect(useFindStore.getState().current).toBe(0);
  });

  it("Escape closes it and gives the focus back", async () => {
    render(<Page />);
    const comment = screen.getByRole("textbox", { name: "Comment" });
    comment.focus();
    press(comment, modF);
    expect(field()).toHaveFocus();
    fireEvent.keyDown(field(), { key: "Escape" });
    expect(useFindStore.getState().open).toBe(false);
    expect(screen.queryByRole("search")).toBeNull();
    expect(comment).toHaveFocus();
  });

  it("shows an invalid regex inline", async () => {
    render(<Page />);
    press(window, modF);
    fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
    fireEvent.change(field(), { target: { value: "(oops" } });
    await vi.waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/Invalid regex/),
    );
    expect(field()).toHaveAttribute("aria-invalid", "true");
  });

  it("from the sidebar: the list's filter when it has one, else the bar", () => {
    const { unmount } = render(<Page filter />);
    const row = screen.getByRole("button", { name: "A review row" });
    row.focus();
    press(row, modF);
    expect(screen.getByRole("searchbox", { name: "Filter" })).toHaveFocus();
    expect(useFindStore.getState().open).toBe(false);
    unmount();
    render(<Page />);
    const row2 = screen.getByRole("button", { name: "A review row" });
    row2.focus();
    press(row2, modF);
    expect(useFindStore.getState().open).toBe(true);
  });

  it("is a rebindable keymap action, listed as Find in review…", () => {
    expect(DEFAULT_KEYMAP.find).toEqual(["Mod+KeyF"]);
    expect(DEFAULT_KEYMAP.findNext).toEqual(["Mod+KeyG"]);
    expect(DEFAULT_KEYMAP.findPrevious).toEqual(["Mod+Shift+KeyG"]);
    expect(ACTIONS.find((a) => a.id === "find")?.label).toBe("Find in review…");
    useSettingsStore.getState().setShortcut("find", ["Mod+Shift+KeyF"]);
    render(<Page />);
    press(window, modF);
    expect(useFindStore.getState().open).toBe(false);
    press(window, { key: "F", code: "KeyF", ctrlKey: true, shiftKey: true });
    expect(useFindStore.getState().open).toBe(true);
  });
});
