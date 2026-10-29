/**
 * The rules of a comment drag from the diff's "+" (`DiffEditor`): how far a
 * range reaches toward the pointer, and what the floating label says. Pure.
 */

/** A commentable line's region, as the caller sees it (`CommentGutter.regionOf`). */
export interface LineRegionInfo {
  /** Equal for lines one range may span. */
  key: string;
  /** Unchanged code outside the diff's hunks. */
  outside: boolean;
}

/** The tooltip of the "+": what a click and a drag do. */
export const PLUS_HINT = "Click to comment, drag to select lines";

/**
 * Where a drag from `anchor` reaches when the pointer is over `target`:
 * as far toward it as the anchor's region goes. `blocked` is the target
 * when the range stops short of it.
 */
export function dragReach(
  region: (line: number) => LineRegionInfo | null,
  anchor: number,
  target: number,
): { to: number; blocked: number | null } {
  const home = region(anchor)?.key;
  if (home === undefined) return { to: anchor, blocked: null };
  const step = target > anchor ? 1 : -1;
  let to = anchor;
  while (to !== target && region(to + step)?.key === home) to += step;
  return { to, blocked: to === target ? null : target };
}

/** Why a drag's range stops short of the pointer. */
export type DragStop = "leaveHunk" | "enterDiff" | "otherPart" | "folded";

/**
 * Why a range from `anchor` stops before `blocked`: `region` as the
 * caller sees it (`folded`: the target is in the anchor's region, so only
 * folded lines in between stopped it).
 */
export function dragStop(
  region: (line: number) => LineRegionInfo | null,
  anchor: number,
  blocked: number,
): DragStop {
  const home = region(anchor);
  const there = region(blocked);
  if (home && there && home.key === there.key) return "folded";
  const anchorOutside = home?.outside ?? false;
  const targetOutside = there?.outside ?? !anchorOutside;
  if (anchorOutside === targetOutside) return "otherPart";
  return anchorOutside ? "enterDiff" : "leaveHunk";
}

const STOP_WORDS: Record<DragStop, string> = {
  leaveHunk: "a range in the diff can't leave its hunk",
  enterDiff: "a range outside the diff can't enter it",
  otherPart: "a range stays in one part of the diff",
  folded: "unfold the lines in between first",
};

/** "Comment on lines 12–18", and why a range stops where it does. */
export function dragLabel(
  anchor: number,
  to: number,
  stop: DragStop | null,
): string {
  const from = Math.min(anchor, to);
  const end = Math.max(anchor, to);
  const what =
    from === end ? `Comment on line ${end}` : `Comment on lines ${from}–${end}`;
  return stop ? `${what} · ${STOP_WORDS[stop]}` : what;
}
