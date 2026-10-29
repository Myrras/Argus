/**
 * Between the find bar's store (`state/findStore`) and the CodeMirror
 * editors (`lib/findHighlight`), without either importing the other:
 * CodeMirror loads lazily with the first diff, the store at startup.
 *
 * - The highlight state (the query's expression, the scope, the current
 *   match) the store publishes and every editor follows.
 * - The opener the editors call on Mod+F (their own key handling runs
 *   before the window's), which the store sets.
 */

import type { Side } from "../ipc/types";
import type { FindScope } from "./findInReview";

/** The current match, as an editor recognises it. */
export interface CurrentFind {
  /** `findInReview.codeUnit(path, side)` or a comment's unit. */
  unit: string;
  /** Code: the line (1-based) on its side. */
  line?: number;
  /** In the line (code) or the unit's text. */
  start: number;
  end: number;
  side?: Side;
}

export interface FindHighlightState {
  /** Null: nothing to highlight (bar closed, empty or invalid query). */
  re: RegExp | null;
  scope: FindScope;
  current: CurrentFind | null;
}

const NONE: FindHighlightState = {
  re: null,
  scope: "everything",
  current: null,
};

let state: FindHighlightState = NONE;
const listeners = new Set<(s: FindHighlightState) => void>();

export function findHighlightState(): FindHighlightState {
  return state;
}

export function publishFindHighlight(next: FindHighlightState): void {
  if (
    next.re === state.re &&
    next.scope === state.scope &&
    sameCurrent(next.current, state.current)
  ) {
    return;
  }
  state = next;
  for (const fn of listeners) fn(state);
}

function sameCurrent(a: CurrentFind | null, b: CurrentFind | null): boolean {
  if (!a || !b) return a === b;
  return (
    a.unit === b.unit &&
    a.line === b.line &&
    a.start === b.start &&
    a.end === b.end
  );
}

/** Follow the highlight state; returns the unsubscribe. */
export function onFindHighlight(
  fn: (s: FindHighlightState) => void,
): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

let opener: ((target: EventTarget | null) => boolean) | null = null;

export function setFindOpener(
  fn: ((target: EventTarget | null) => boolean) | null,
) {
  opener = fn;
}

/** Open the find bar as Mod+F from `target` would; false without an opener. */
export function openFindFrom(target: EventTarget | null): boolean {
  return opener?.(target) ?? false;
}
