/**
 * Reveal a thread or finding after a keyboard move (n/p, the next/previous
 * open thread, the palette): scroll the page column so the item's own card
 * sits at the top, under the sticky headers, and give it the DOM focus.
 *
 * The item may not be in the DOM yet, for three reasons, handled in turn
 * each frame until its element shows:
 * - its file's contents are still loading (the card loads them when
 *   selected): the reveal starts the same cached load and does not count
 *   the wait against its bound;
 * - CodeMirror renders only the lines near the viewport, so a block under
 *   line 900 has no DOM: the file's editor (`lib/diffEditors`) is asked to
 *   scroll to that line, which renders it;
 * - the block's React root mounts a moment after.
 *
 * Threads listed at the top of a card (outdated ones, those in the
 * Conversation, large or binary files) are in the DOM at once. Only when no
 * element shows within the bound does the card scroll into view instead.
 * Pure over an injected page (`RevealEnv`) so the rules test without
 * layout; `pageRevealEnv` is the real one.
 */

import { prefersReducedMotion } from "./motion";
import { pauseScrollAnchor } from "./scrollAnchor";
import {
  diffEditorFor,
  findingBlockKey,
  threadBlockKey,
  type DiffRevealer,
} from "./diffEditors";
import { CONVERSATION_CARD, cardOf, type FocusItem } from "./focusModel";

/** How long to wait for the element once the contents are in. */
export const REVEAL_TIMEOUT_MS = 2000;
/** Space left above the item under the sticky headers. */
const GAP_PX = 8;

export interface RevealEnv {
  /** The item's element (`thread-…` / `finding-…`), if rendered. */
  element: () => HTMLElement | null;
  /** The item's file editor, once mounted. */
  editor: () => DiffRevealer | null;
  /** Start (or join) the load of the item's file; null if none applies. */
  load: () => Promise<unknown> | null;
  /** The item's card: the fallback. */
  card: () => HTMLElement | null;
  /** Scroll `el` to the top of the page column. */
  place: (el: HTMLElement, behavior: ScrollBehavior) => void;
  /** Scroll the card to the top instead (no element could show). */
  scrollCard: (card: HTMLElement, behavior: ScrollBehavior) => void;
  /** Run `fn` on the next frame; returns a cancel. */
  next: (fn: () => void) => () => void;
  now: () => number;
  reducedMotion: () => boolean;
}

/** The editor block key of a thread or finding. */
function blockKey(item: FocusItem): string | null {
  if (item.kind === "thread") return threadBlockKey(item.threadId);
  if (item.kind === "finding") return findingBlockKey(item.findingId);
  return null;
}

/**
 * Reveal `item` (a thread or finding) on the page; returns a cancel (a
 * newer move, the page going away).
 */
export function revealItem(item: FocusItem, env: RevealEnv): () => void {
  const card = cardOf(item);
  const key = blockKey(item);
  const inFile = card !== CONVERSATION_CARD;
  // Under a line: the editor may have to scroll there first.
  const inline =
    inFile &&
    (item.kind === "thread" || item.kind === "finding") &&
    item.line > 0;
  const behavior: ScrollBehavior = env.reducedMotion() ? "auto" : "smooth";

  let cancelled = false;
  let cancelFrame: (() => void) | null = null;
  let loading = false;
  let since = env.now();
  let asked: DiffRevealer | null = null;

  if (inFile) {
    const load = env.load();
    if (load) {
      loading = true;
      const done = () => {
        loading = false;
        since = env.now();
      };
      load.then(done, done);
    }
  }

  const tick = () => {
    cancelFrame = null;
    if (cancelled) return;
    const el = env.element();
    if (el) {
      env.place(el, behavior);
      el.focus({ preventScroll: true });
      return;
    }
    if (inline && key) {
      const editor = env.editor();
      // Once per editor: a remount (another layout) is asked again.
      if (editor && editor !== asked && editor.reveal(key)) asked = editor;
    }
    if (!loading && env.now() - since >= REVEAL_TIMEOUT_MS) {
      const c = env.card();
      if (c) env.scrollCard(c, behavior);
      return;
    }
    cancelFrame = env.next(tick);
  };
  tick();

  return () => {
    cancelled = true;
    cancelFrame?.();
  };
}

/** The height the sticky toolbar and card header cover at the top. */
export function coveredTop(el: HTMLElement, column: HTMLElement): number {
  const header = el
    .closest<HTMLElement>("[data-file-card]")
    ?.querySelector<HTMLElement>(":scope > header");
  if (header && getComputedStyle(header).position === "sticky") {
    return (
      (parseFloat(getComputedStyle(header).top) || 0) + header.offsetHeight
    );
  }
  const toolbar = column.firstElementChild as HTMLElement | null;
  return toolbar?.offsetHeight ?? 0;
}

/** Scroll the page column so `el`'s top sits under the sticky headers. */
export function placeAtTop(el: HTMLElement, behavior: ScrollBehavior): void {
  const column = el.closest<HTMLElement>('[data-slot="diff"]');
  if (!column || typeof column.scrollTo !== "function") {
    el.scrollIntoView?.({ block: "start", behavior });
    return;
  }
  const offset =
    el.getBoundingClientRect().top - column.getBoundingClientRect().top;
  const top = column.scrollTop + offset - coveredTop(el, column) - GAP_PX;
  column.scrollTo({ top: Math.max(0, top), behavior });
}

/** The card (`data-file-card`) of a file path or the Conversation. */
export function cardElement(card: string): HTMLElement | null {
  if (card === CONVERSATION_CARD) {
    return document.querySelector<HTMLElement>(
      '[data-file-card="conversation"]',
    );
  }
  const cards = document.querySelectorAll<HTMLElement>(
    '[data-file-card="file"]',
  );
  for (const el of cards) if (el.dataset.path === card) return el;
  return null;
}

/** The page's own `RevealEnv` for `item`. */
export function pageRevealEnv(
  item: FocusItem,
  opts: {
    element: (item: FocusItem) => HTMLElement | null;
    load: (path: string) => Promise<unknown> | null;
  },
): RevealEnv {
  const card = cardOf(item);
  return {
    element: () => opts.element(item),
    editor: () => diffEditorFor(card),
    load: () => opts.load(card),
    card: () => cardElement(card),
    // Navigation: the page's scroll anchoring stands aside meanwhile.
    place: (el, behavior) => {
      pauseScrollAnchor();
      placeAtTop(el, behavior);
    },
    scrollCard: (c, behavior) => {
      pauseScrollAnchor();
      c.scrollIntoView?.({ block: "start", behavior });
    },
    next: (fn) => {
      if (typeof requestAnimationFrame === "function") {
        const id = requestAnimationFrame(fn);
        return () => cancelAnimationFrame(id);
      }
      const id = setTimeout(fn, 16);
      return () => clearTimeout(id);
    },
    now: () => Date.now(),
    reducedMotion: prefersReducedMotion,
  };
}
