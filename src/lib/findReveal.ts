/**
 * The find bar's page side: bring a match into view and say where the
 * reader is.
 *
 * `revealMatch` works like `lib/revealItem` (a thread's reveal after
 * `n`/`p`), frame by frame until the match's highlight shows: it joins the
 * file's load, asks the file's editor (`lib/diffEditors`) to scroll to the
 * line (`revealLine`, which opens a collapsed stretch) or to the thread or
 * finding block under it (CodeMirror draws only lines near the viewport),
 * then puts the highlight in the reading area if it is not there already.
 * It never takes the DOM focus: that stays in the find bar. The current
 * highlight is recognised by its unit (`data-find-unit` around a
 * comment's marks, the card's path around an editor's), so a mark the
 * previous render left behind is never taken for it.
 *
 * A removed line in the unified view has no element of its own (the
 * deletion block is the library's; its highlight is a CSS highlight):
 * the editor's own centred scroll is the reveal there.
 *
 * Pure over an injected page (`MatchRevealEnv`); `pageMatchEnv` is the real
 * one.
 */

import { cardElement, coveredTop } from "./revealItem";
import {
  diffEditorFor,
  findingBlockKey,
  threadBlockKey,
  type DiffRevealer,
} from "./diffEditors";
import { CONVERSATION_CARD } from "./focusModel";
import type { FindMatch, PagePlace } from "./findInReview";
import { pauseScrollAnchor, stickyBarsHeight } from "./scrollAnchor";

/** How long to wait for the highlight once the contents are in. */
export const FIND_REVEAL_TIMEOUT_MS = 2000;
/** How long a code reveal waits for its highlight before it is done. */
const SETTLE_MS = 400;

export interface MatchRevealEnv {
  /** The match's current highlight, once rendered. */
  element: () => HTMLElement | null;
  /** The thread or finding that holds a comment match (the fallback). */
  holder: () => HTMLElement | null;
  /** The file's editor, once mounted. */
  editor: () => DiffRevealer | null;
  /** Start (or join) the file's load; null when none applies. */
  load: () => Promise<unknown> | null;
  card: () => HTMLElement | null;
  /** Put `el` in the reading area (no scroll when it is there already). */
  place: (el: HTMLElement) => void;
  scrollCard: (card: HTMLElement) => void;
  next: (fn: () => void) => () => void;
  now: () => number;
}

/** Reveal `m`; returns a cancel (a newer reveal, the bar closing). */
export function revealMatch(m: FindMatch, env: MatchRevealEnv): () => void {
  const inFile = m.card !== CONVERSATION_CARD;
  const code = m.kind === "code";
  // A thread or finding under a line (not listed at the card's top).
  const blockKey = m.threadId
    ? threadBlockKey(m.threadId)
    : m.findingId
      ? findingBlockKey(m.findingId)
      : null;
  const underLine = inFile && !code && m.row >= 0;

  let cancelled = false;
  let cancelFrame: (() => void) | null = null;
  let loading = false;
  let since = env.now();
  let asked: DiffRevealer | null = null;
  let askedAt = 0;

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
      env.place(el);
      return;
    }
    if (code || (underLine && blockKey)) {
      const editor = env.editor();
      if (editor && editor !== asked) {
        const ok = code
          ? (editor.revealLine?.(m.side ?? "new", m.line ?? 1) ?? false)
          : editor.reveal(blockKey!);
        if (ok) {
          asked = editor;
          askedAt = env.now();
        }
      }
      // The editor scrolled there; a removed line of the unified view has
      // no element to wait for.
      if (code && asked && env.now() - askedAt >= SETTLE_MS) return;
    }
    if (!loading && env.now() - since >= FIND_REVEAL_TIMEOUT_MS) {
      const holder = env.holder();
      if (holder) {
        env.place(holder);
        return;
      }
      const c = env.card();
      if (c) env.scrollCard(c);
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

/** The current highlight of `m` in the page, if rendered. */
export function currentHighlight(m: FindMatch): HTMLElement | null {
  if (m.kind === "code") {
    const card = cardElement(m.card);
    return (
      card?.querySelector<HTMLElement>(".cm-editor .cm-findCurrent") ?? null
    );
  }
  for (const holder of document.querySelectorAll<HTMLElement>(
    "[data-find-unit]",
  )) {
    if (holder.dataset.findUnit !== m.unit) continue;
    const mark = holder.querySelector<HTMLElement>(
      `[data-find-current="true"][data-find-match="${m.ordinal}"], .cm-findCurrent`,
    );
    if (mark) return mark;
  }
  return null;
}

/** The thread or finding card holding a comment match. */
function holderOf(m: FindMatch): HTMLElement | null {
  if (m.threadId) return document.getElementById(`thread-${m.threadId}`);
  if (m.findingId) return document.getElementById(`finding-${m.findingId}`);
  return null;
}

/**
 * Scroll the page column so `el` sits a third down the reading area,
 * unless it is in view already (each Enter must not shift the page).
 */
export function placeInView(el: HTMLElement): void {
  const column = el.closest<HTMLElement>('[data-slot="diff"]');
  if (!column || typeof column.scrollTo !== "function") {
    el.scrollIntoView?.({ block: "center" });
    return;
  }
  const box = column.getBoundingClientRect();
  const rect = el.getBoundingClientRect();
  const top =
    box.top + Math.max(coveredTop(el, column), stickyBarsHeight(column));
  const bottom = box.bottom - 16;
  if (rect.top >= top + 4 && rect.bottom <= bottom) return;
  pauseScrollAnchor();
  const third = Math.max(0, (bottom - top) / 3);
  column.scrollTo({
    top: Math.max(0, column.scrollTop + rect.top - top - third),
    behavior: "auto",
  });
}

/** The page's own `MatchRevealEnv` for `m`. */
export function pageMatchEnv(
  m: FindMatch,
  load: (path: string) => Promise<unknown> | null,
): MatchRevealEnv {
  return {
    element: () => currentHighlight(m),
    holder: () => holderOf(m),
    editor: () => diffEditorFor(m.card),
    load: () => load(m.card),
    card: () => cardElement(m.card),
    place: placeInView,
    scrollCard: (c) => {
      pauseScrollAnchor();
      c.scrollIntoView?.({ block: "start" });
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
  };
}

/**
 * Where the reader is: the first card under the sticky headers (its index
 * in `order`) and, inside a diff, the head line at the reading edge.
 */
export function readingPlace(order: readonly string[]): PagePlace | null {
  const column = document.querySelector<HTMLElement>('[data-slot="diff"]');
  if (!column) return null;
  const edge = column.getBoundingClientRect().top + stickyBarsHeight(column);
  for (const card of column.querySelectorAll<HTMLElement>("[data-file-card]")) {
    const rect = card.getBoundingClientRect();
    if (rect.bottom <= edge) continue;
    const key =
      card.dataset.fileCard === "conversation"
        ? CONVERSATION_CARD
        : (card.dataset.path ?? "");
    const index = order.indexOf(key);
    if (index < 0) continue;
    if (rect.top >= edge - 1 || key === CONVERSATION_CARD) {
      return { card: index, row: -Infinity };
    }
    const line = diffEditorFor(key)?.readingLine?.() ?? null;
    return { card: index, row: line ?? -Infinity };
  }
  return null;
}
