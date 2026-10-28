/**
 * "An action on an item never scrolls the page": dismissing, resolving,
 * selecting, editing, folding, then/now, viewed, from the button and from
 * the key. Only navigation (j/k/n/p, the open-thread jumps, Go to…, a tree
 * click) scrolls. Here: none of them calls a scroll API, and any focus
 * they move keeps the scroll (`preventScroll`).
 */

import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
vi.mock("./DiffEditor", () => ({ DiffEditor: () => <div>diff</div> }));

import { invoke } from "@tauri-apps/api/core";

import { findingItem, threadItem } from "../lib/focusModel";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import { usePageFocus } from "../lib/usePageFocus";
import { useAiStore } from "../state/aiStore";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import { useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useHistoryStore } from "../state/historyStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThreadStore } from "../state/threadStore";
import { aiRun, finding } from "../test/aiFixtures";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { FileDiffCard } from "./FileDiffCard";
import { FindingBlock } from "./FindingBlock";
import { ThreadBlock } from "./ThreadBlock";

const mockInvoke = vi.mocked(invoke);
const r = review({ title: "Fixture scroll review" });
const t = thread({
  reviewId: r.id,
  author: "bob-fixture",
  resolvable: true,
  anchor: { ...thread().anchor!, pathNew: "src/cache.rs", lineNew: 4 },
});
const f = finding({ reviewId: r.id, path: "src/cache.rs", line: 6 });

function Page() {
  useAppShortcuts();
  usePageFocus(r.id);
  return (
    <div data-slot="diff" aria-label="Review detail">
      <ThreadBlock thread={t} provider="github" now={0} />
      <FindingBlock reviewId={r.id} findingId={f.id} />
    </div>
  );
}

function press(code: string, key: string) {
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        key,
        code,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
}

const scrolls = vi.fn();
const focusCalls: (FocusOptions | undefined)[] = [];
const saved = {
  scrollIntoView: Element.prototype.scrollIntoView,
  scrollTo: Element.prototype.scrollTo,
  scrollBy: Element.prototype.scrollBy,
  focus: HTMLElement.prototype.focus,
};

beforeEach(() => {
  localStorage.clear();
  scrolls.mockReset();
  focusCalls.length = 0;
  Element.prototype.scrollIntoView = function (this: Element) {
    scrolls("scrollIntoView", this);
  };
  Element.prototype.scrollTo = function (this: Element) {
    scrolls("scrollTo", this);
  } as typeof Element.prototype.scrollTo;
  Element.prototype.scrollBy = function (this: Element) {
    scrolls("scrollBy", this);
  } as typeof Element.prototype.scrollBy;
  HTMLElement.prototype.focus = function (opts?: FocusOptions) {
    focusCalls.push(opts);
    saved.focus.call(this, opts);
  };
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    if (cmd === "dismiss_finding") {
      return Promise.resolve({ ...f, dismissed: a.dismissed as boolean });
    }
    if (cmd === "resolve_thread") {
      return Promise.resolve({ ...t, resolved: a.resolved as boolean });
    }
    if (cmd === "set_file_viewed") return Promise.resolve([a.path]);
    if (cmd === "list_threads") return Promise.resolve([t]);
    if (cmd === "list_findings") return Promise.resolve([f]);
    return Promise.resolve([]);
  });
  useSettingsStore.getState().resetShortcuts();
  useCommentStore.setState({ drafts: {}, composer: null, replies: {} });
  useReviewStore.setState({
    reviews: [r],
    selectedId: r.id,
    cursorId: r.id,
    tab: "needsReview",
  });
  useCodeViewStore.setState({
    byReview: {
      [r.id]: {
        since: false,
        files: reviewFiles({
          reviewId: r.id,
          files: [file({ path: "src/cache.rs" })],
        }),
        loading: false,
        error: null,
        selected: "src/cache.rs",
        order: [CONVERSATION, "src/cache.rs"],
      },
    },
  });
  useThreadStore.setState({
    byReview: { [r.id]: { threads: [t], loading: false, error: null } },
    resolving: {},
  });
  useAiStore.setState({
    runs: { [r.id]: aiRun({ reviewId: r.id }) },
    byReview: {
      [r.id]: {
        findings: [f],
        skipped: [],
        checklist: [],
        skippedChecklist: [],
        verdicts: [],
        loading: false,
        error: null,
      },
    },
    ui: {},
    focused: null,
  });
  useHistoryStore.setState({ open: {} });
  useFocusStore.setState({ reviewId: r.id, item: null, zone: "page" });
});

afterEach(() => {
  Element.prototype.scrollIntoView = saved.scrollIntoView;
  Element.prototype.scrollTo = saved.scrollTo;
  Element.prototype.scrollBy = saved.scrollBy;
  HTMLElement.prototype.focus = saved.focus;
});

/** Let the action's IPC calls and re-reads settle. */
async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });
}

function expectNoScroll() {
  expect(scrolls).not.toHaveBeenCalled();
  for (const opts of focusCalls) expect(opts?.preventScroll).toBe(true);
}

const focusThread = () =>
  act(() =>
    useFocusStore
      .getState()
      .focusItem(r.id, threadItem(t, "src/cache.rs", false)),
  );
const focusFinding = () =>
  act(() => useFocusStore.getState().focusItem(r.id, findingItem(f, false)));

describe("item actions keep the scroll", () => {
  it("Dismiss on a finding", async () => {
    render(<Page />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith("dismiss_finding", {
      findingId: f.id,
      dismissed: true,
    });
    expectNoScroll();
  });

  it("d on the focused finding", async () => {
    render(<Page />);
    focusFinding();
    press("KeyD", "d");
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith(
      "dismiss_finding",
      expect.anything(),
    );
    expectNoScroll();
  });

  it("Resolve on a thread", async () => {
    render(<Page />);
    fireEvent.click(screen.getByRole("button", { name: /^Resolve/ }));
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith(
      "resolve_thread",
      expect.anything(),
    );
    expectNoScroll();
  });

  it("s on the focused thread", async () => {
    render(<Page />);
    focusThread();
    press("KeyS", "s");
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith(
      "resolve_thread",
      expect.anything(),
    );
    expectNoScroll();
  });

  it("x, e on the focused finding and its Edit button", async () => {
    render(<Page />);
    focusFinding();
    press("KeyX", "x");
    expect(useAiStore.getState().ui[f.id]?.selected).toBe(true);
    press("KeyE", "e");
    await settle();
    expect(useAiStore.getState().ui[f.id]?.editor).not.toBeNull();
    act(() => useAiStore.getState().closeEditor(f.id));
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await settle();
    expectNoScroll();
  });

  it("t, Enter and h on the focused thread", async () => {
    render(<Page />);
    focusThread();
    const card = screen.getByRole("article", { name: "Thread by bob-fixture" });
    press("KeyT", "t");
    fireEvent.keyDown(card, { key: "Enter" });
    press("KeyH", "h");
    await settle();
    expectNoScroll();
  });

  it("v on the selected file", async () => {
    render(<Page />);
    press("KeyV", "v");
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith(
      "set_file_viewed",
      expect.objectContaining({ path: "src/cache.rs", viewed: true }),
    );
    expectNoScroll();
  });
});

describe("marking the file you are reading viewed", () => {
  /** The page column at the window's top, scrolled into a long file. */
  function column(cardTop: number) {
    const files = reviewFiles({ reviewId: r.id });
    const entry = files.files[0];
    const card = (viewed: boolean) => (
      <div data-slot="diff">
        <div className="toolbar">toolbar</div>
        <FileDiffCard
          review={files}
          file={{ ...entry, viewed }}
          threads={[]}
          provider="github"
          hasClone={false}
          layout="unified"
          selected
          onSelect={() => undefined}
          onToggleViewed={() => undefined}
          cardRef={() => undefined}
        />
      </div>
    );
    const view = render(card(false));
    const col =
      view.container.querySelector<HTMLElement>('[data-slot="diff"]')!;
    let top = 5000;
    Object.defineProperty(col, "scrollTop", {
      get: () => top,
      set: (v: number) => {
        top = v;
      },
    });
    col.getBoundingClientRect = () =>
      ({ top: 0, bottom: 800, height: 800, left: 0, width: 900 }) as DOMRect;
    const bar = col.firstElementChild as HTMLElement;
    Object.defineProperty(bar, "offsetHeight", { value: 38 });
    const section = col.querySelector<HTMLElement>("[data-file-card]")!;
    section.getBoundingClientRect = () =>
      ({ top: cardTop, bottom: cardTop + 4000 }) as DOMRect;
    return {
      view: () => view.rerender(card(true)),
      get scrollTop() {
        return top;
      },
    };
  }

  it("keeps its header where it was instead of jumping past the fold", () => {
    const page = column(-3000);
    page.view();
    // The folded card's header sits under the toolbar, as it did stuck.
    expect(page.scrollTop).toBe(5000 - 3000 - 38);
    expectNoScroll();
  });

  it("leaves the page alone when the file's top is in view", () => {
    const page = column(200);
    page.view();
    expect(page.scrollTop).toBe(5000);
  });
});
