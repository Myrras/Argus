import { describe, expect, it, vi } from "vitest";

import type { FocusItem } from "./focusModel";
import { CONVERSATION_CARD } from "./focusModel";
import type { DiffRevealer } from "./diffEditors";
import { REVEAL_TIMEOUT_MS, revealItem, type RevealEnv } from "./revealItem";

const inline: FocusItem = {
  kind: "thread",
  threadId: "00000000-0000-4000-8000-000000000900",
  mine: false,
  card: "src/big.rs",
  line: 900,
  side: "new",
};

/** A scripted page: frames run on `frame()`, the clock moves on `advance`. */
function page() {
  let queued: (() => void)[] = [];
  let clock = 0;
  const el = document.createElement("article");
  el.tabIndex = -1;
  const card = document.createElement("section");
  const state = {
    element: null as HTMLElement | null,
    editor: null as DiffRevealer | null,
    load: null as Promise<unknown> | null,
    reduced: false,
  };
  const env: RevealEnv = {
    element: vi.fn(() => state.element),
    editor: vi.fn(() => state.editor),
    load: vi.fn(() => state.load),
    card: () => card,
    place: vi.fn(),
    scrollCard: vi.fn(),
    next: (fn) => {
      queued.push(fn);
      return () => {
        queued = queued.filter((f) => f !== fn);
      };
    },
    now: () => clock,
    reducedMotion: () => state.reduced,
  };
  return {
    env,
    el,
    card,
    state,
    frame() {
      const run = queued;
      queued = [];
      for (const f of run) f();
    },
    advance(ms: number) {
      clock += ms;
    },
    pending: () => queued.length,
  };
}

/** Let resolved promises' callbacks run. */
const settle = () => new Promise((r) => setTimeout(r, 0));

describe("revealItem", () => {
  it("loads the thread's file", () => {
    const p = page();
    revealItem(inline, p.env);
    expect(p.env.load).toHaveBeenCalledTimes(1);
  });

  it("does not load anything for a Conversation thread", () => {
    const p = page();
    revealItem(
      { ...inline, card: CONVERSATION_CARD, line: 0, side: null },
      p.env,
    );
    expect(p.env.load).not.toHaveBeenCalled();
  });

  it("asks the file's editor once to bring the thread's line into view", () => {
    const p = page();
    revealItem(inline, p.env);
    p.frame();
    // The editor mounts later (the lazy chunk, the contents).
    const reveal = vi.fn(() => true);
    p.state.editor = { reveal };
    p.frame();
    p.frame();
    p.frame();
    expect(reveal).toHaveBeenCalledTimes(1);
    expect(reveal).toHaveBeenCalledWith(`t:${inline.threadId}`);
  });

  it("asks again while the editor has not placed the block yet", () => {
    const p = page();
    const reveal = vi.fn(() => false);
    p.state.editor = { reveal };
    revealItem(inline, p.env);
    p.frame();
    p.frame();
    expect(reveal).toHaveBeenCalledTimes(3);
  });

  it("asks a remounted editor (another layout) again", () => {
    const p = page();
    const first = { reveal: vi.fn(() => true) };
    const second = { reveal: vi.fn(() => true) };
    p.state.editor = first;
    revealItem(inline, p.env);
    p.frame();
    p.state.editor = second;
    p.frame();
    expect(first.reveal).toHaveBeenCalledTimes(1);
    expect(second.reveal).toHaveBeenCalledTimes(1);
  });

  it("waits for the thread's element, then places and focuses it, not the card", () => {
    const p = page();
    const focus = vi.spyOn(p.el, "focus");
    p.state.editor = { reveal: () => true };
    revealItem(inline, p.env);
    p.frame();
    p.frame();
    expect(p.env.place).not.toHaveBeenCalled();
    // CodeMirror rendered the line, the widget's React root mounted.
    p.state.element = p.el;
    p.frame();
    expect(p.env.place).toHaveBeenCalledWith(p.el, "smooth");
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(p.env.scrollCard).not.toHaveBeenCalled();
    expect(p.pending()).toBe(0);
  });

  it("places a thread listed at the top of its card at once, without the editor", () => {
    const p = page();
    const reveal = vi.fn(() => true);
    p.state.editor = { reveal };
    p.state.element = p.el;
    revealItem({ ...inline, line: 0, side: null }, p.env);
    expect(p.env.place).toHaveBeenCalledWith(p.el, "smooth");
    expect(reveal).not.toHaveBeenCalled();
  });

  it("scrolls without animation under reduced motion", () => {
    const p = page();
    p.state.reduced = true;
    p.state.element = p.el;
    revealItem(inline, p.env);
    expect(p.env.place).toHaveBeenCalledWith(p.el, "auto");
  });

  it("does not give up while the file is still loading", async () => {
    const p = page();
    let done!: () => void;
    p.state.load = new Promise<void>((r) => (done = r));
    revealItem(inline, p.env);
    p.advance(REVEAL_TIMEOUT_MS * 3);
    p.frame();
    expect(p.env.scrollCard).not.toHaveBeenCalled();
    done();
    await settle();
    p.frame();
    p.state.element = p.el;
    p.frame();
    expect(p.env.place).toHaveBeenCalledWith(p.el, "smooth");
    expect(p.env.scrollCard).not.toHaveBeenCalled();
  });

  it("falls back to the card only when no thread element appears", async () => {
    const p = page();
    p.state.load = Promise.resolve();
    revealItem(inline, p.env);
    await settle();
    p.frame();
    p.advance(REVEAL_TIMEOUT_MS - 1);
    p.frame();
    expect(p.env.scrollCard).not.toHaveBeenCalled();
    p.advance(2);
    p.frame();
    expect(p.env.scrollCard).toHaveBeenCalledWith(p.card, "smooth");
    expect(p.env.place).not.toHaveBeenCalled();
    expect(p.pending()).toBe(0);
  });

  it("stops when cancelled (another move came first)", () => {
    const p = page();
    const cancel = revealItem(inline, p.env);
    cancel();
    p.state.element = p.el;
    p.frame();
    expect(p.env.place).not.toHaveBeenCalled();
  });

  it("uses a finding's block key for a finding", () => {
    const p = page();
    const reveal = vi.fn(() => true);
    p.state.editor = { reveal };
    revealItem(
      {
        kind: "finding",
        findingId: "00000000-0000-4000-a000-000000000101",
        card: "src/big.rs",
        line: 880,
        side: "new",
      },
      p.env,
    );
    expect(reveal).toHaveBeenCalledWith(
      "finding:00000000-0000-4000-a000-000000000101",
    );
  });
});
