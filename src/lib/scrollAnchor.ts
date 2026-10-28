/**
 * Scroll anchoring for the review page's column: what is under the top edge
 * stays where it is when content above it changes height — a thread
 * resolved and re-read, a finding dismissed, a diff's widgets rebuilt, the
 * AI panel opened (text wraps anew). WKWebView (the macOS webview) has no
 * CSS scroll anchoring, and CodeMirror turns it off inside its editors and
 * corrects the outer scroller only right after a wheel event or with the
 * editor focused, so the column does it itself.
 *
 * On every scroll it notes a few elements under the top edge and where
 * they are; when the content's size changes (and nothing scrolled the
 * column since) it scrolls by however far the first one still in the page
 * moved. Navigation (a reveal, a tree click,
 * "show in the code") pauses it while its own scroll runs
 * (`pauseScrollAnchor`): only navigation scrolls the page.
 */

const PAUSE_MS = 1000;
let pausedUntil = 0;

/**
 * A navigation scroll starts: leave the page alone for `ms` (it notes the
 * anchors again once the scroll is over).
 */
export function pauseScrollAnchor(
  ms: number = PAUSE_MS,
  now: number = Date.now(),
): void {
  pausedUntil = now + ms;
}

export interface ScrollAnchorEnv {
  /** Elements under the column's top edge, topmost first. */
  pick: () => Element[];
  /** Call `onResize` when the content changes size; returns the stop. */
  observe: (onResize: () => void) => () => void;
  now: () => number;
}

interface Noted {
  el: Element;
  top: number;
}

export function createScrollAnchor(
  column: HTMLElement,
  env: ScrollAnchorEnv,
): { capture: () => void; restore: () => void; destroy: () => void } {
  let noted: Noted[] = [];
  let at = column.scrollTop;
  let stopped = false;

  const capture = () => {
    at = column.scrollTop;
    noted = env
      .pick()
      .map((el) => ({ el, top: el.getBoundingClientRect().top }));
  };

  const restore = () => {
    if (stopped) return;
    // Paused for navigation, or the column was scrolled since (CodeMirror
    // scrolling to a line, the browser clamping a shorter page) before its
    // scroll event came: what moved is that scroll, not to be undone.
    if (env.now() < pausedUntil || Math.abs(column.scrollTop - at) >= 1) {
      capture();
      return;
    }
    const kept = noted.find((n) => n.el.isConnected);
    if (kept) {
      const delta = kept.el.getBoundingClientRect().top - kept.top;
      if (Math.abs(delta) >= 1) column.scrollTop += delta;
    }
    capture();
  };

  capture();
  column.addEventListener("scroll", capture, { passive: true });
  const stop = env.observe(restore);
  return {
    capture,
    restore,
    destroy: () => {
      stopped = true;
      column.removeEventListener("scroll", capture);
      stop();
    },
  };
}

/** A card's sticky header under the toolbar (`FileDiffCard` 34px) and a gap. */
const HEADER_PX = 40;
const PROBES = [0, 160, 360];

/**
 * Where the reader's eye is in the page column: just under its sticky
 * toolbar and the card header stuck beneath it (viewport y).
 */
export function readingEdge(column: HTMLElement): number {
  const toolbar = column.firstElementChild as HTMLElement | null;
  return (
    column.getBoundingClientRect().top +
    (toolbar?.offsetHeight ?? 0) +
    HEADER_PX
  );
}

/**
 * The page's own anchors: elements at a few heights just under the sticky
 * headers, inside the column (not a sticky header itself).
 */
export function pickUnderTop(column: HTMLElement): Element[] {
  const box = column.getBoundingClientRect();
  if (box.height === 0 || typeof document.elementFromPoint !== "function") {
    return [];
  }
  const x = box.left + Math.min(box.width / 2, 240);
  const edge = readingEdge(column);
  const out: Element[] = [];
  for (const dy of PROBES) {
    const y = edge + dy;
    if (y >= box.bottom) break;
    const el = document.elementFromPoint(x, y);
    if (!el || !column.contains(el) || el === column) continue;
    if (el.closest("header")) continue;
    if (!out.includes(el)) out.push(el);
  }
  return out;
}

/** Anchor `column`, watching `content` (its scrolled stack) for size changes. */
export function anchorColumn(
  column: HTMLElement,
  content: HTMLElement,
): () => void {
  if (typeof ResizeObserver === "undefined") return () => undefined;
  const anchor = createScrollAnchor(column, {
    pick: () => pickUnderTop(column),
    observe: (onResize) => {
      const ro = new ResizeObserver(onResize);
      ro.observe(content);
      return () => ro.disconnect();
    },
    now: () => Date.now(),
  });
  return anchor.destroy;
}

/** The page column a card sits in, and the height its toolbar covers. */
function columnOf(card: HTMLElement) {
  const column = card.closest<HTMLElement>('[data-slot="diff"]');
  if (!column) return null;
  const toolbar = column.firstElementChild as HTMLElement | null;
  return {
    column,
    edge: column.getBoundingClientRect().top + (toolbar?.offsetHeight ?? 0),
  };
}

/** The reader is inside `card`: its top is above the toolbar (its header stuck). */
export function readingInside(card: HTMLElement | null): boolean {
  const at = card ? columnOf(card) : null;
  return !!at && card!.getBoundingClientRect().top < at.edge;
}

/**
 * After `card` folded while being read (marked viewed): put its header
 * back where it was stuck, under the toolbar, instead of leaving the page
 * scrolled past the fold into whatever follows.
 */
export function keepFoldedCardInPlace(card: HTMLElement | null): void {
  const at = card ? columnOf(card) : null;
  if (!at) return;
  const delta = card!.getBoundingClientRect().top - at.edge;
  if (Math.abs(delta) >= 1) at.column.scrollTop += delta;
}
