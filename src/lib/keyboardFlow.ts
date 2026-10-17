/**
 * The keyboard review flow's actions (M6), dispatched by `useAppShortcuts`
 * and the command palette: move the page focus (`n`/`p`), and act on the
 * focused item — open or fold a thread (`t`; Enter on the card), reply to it (`r`), edit or
 * dismiss a finding (`e`/`d`), comment on its line (`c`) — plus opening the
 * palette and the shortcut help.
 *
 * Nothing here posts: a reply or a comment only opens a composer (posted
 * from its own button or ⌘↩), editing and dismissing a finding are local.
 */

import type { CommentTarget } from "../state/commentStore";
import type { ReviewId, Side } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { useCodeViewStore } from "../state/codeViewStore";
import { loadCommentable, useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useLayoutStore } from "../state/layoutStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { toastInfo } from "../state/toastStore";
import { clampRange, commentableSet } from "./commentRanges";
import { ownsEnter, type ActionId } from "./keymap";

/** Where a new comment on `line` of `path` goes; null when it cannot. */
export async function lineTarget(
  reviewId: ReviewId,
  path: string,
  side: Side,
  line: number,
): Promise<CommentTarget | null> {
  const files = useCodeViewStore.getState().byReview[reviewId]?.files;
  if (!files || line < 1) return null;
  // The unified view comments on new-side lines only; "since" mode's left
  // side is not the base old-side lines refer to.
  if (
    side === "old" &&
    (files.sinceLastReview || useLayoutStore.getState().diffLayout !== "split")
  ) {
    return null;
  }
  let lines;
  try {
    lines = await loadCommentable(reviewId, files.headSha, path);
  } catch {
    return null;
  }
  if (lines.headSha !== files.headSha) return null;
  if (!commentableSet(lines, side).has(line)) return null;
  const range = clampRange(lines, side, line, line);
  if (!range) return null;
  return {
    reviewId,
    path,
    side,
    anchor: line,
    ...range,
    headSha: files.headSha,
    baseSha: files.sinceLastReview ? null : files.baseSha,
  };
}

/**
 * `c` on a focused thread or finding: a composer on its line. False when
 * the focused item has no line (the key then falls back to the line last
 * clicked).
 */
export function commentOnFocusedItem(reviewId: ReviewId): boolean {
  const { item, reviewId: at } = useFocusStore.getState();
  if (!item || at !== reviewId) return false;
  if (item.kind !== "thread" && item.kind !== "finding") return false;
  const side = item.side;
  if (!side || item.line < 1) return false;
  void lineTarget(reviewId, item.card, side, item.line).then((target) => {
    // The lines came in after the user moved to another review.
    if (useReviewStore.getState().selectedId !== reviewId) return;
    if (!target) {
      toastInfo("That line cannot take a comment in this view");
      return;
    }
    useCommentStore.getState().openComposer(target);
  });
  return true;
}

function focusedFinding(reviewId: ReviewId) {
  const { item, reviewId: at } = useFocusStore.getState();
  if (at !== reviewId || item?.kind !== "finding") return null;
  const ai = useAiStore.getState();
  if (!ai.runs[reviewId]?.enabled) return null;
  return (
    ai.byReview[reviewId]?.findings.find((f) => f.id === item.findingId) ?? null
  );
}

function focusedThread(reviewId: ReviewId) {
  const { item, reviewId: at } = useFocusStore.getState();
  return at === reviewId && item?.kind === "thread" ? item : null;
}

/**
 * Run one of the flow's review-page actions; null when `action` is not
 * one of them, false when there is nothing to act on (the key is then left
 * to the next scope or its target).
 */
export function runFlowAction(
  action: ActionId,
  reviewId: ReviewId,
  target: EventTarget | null,
  key: string | null = null,
): boolean | null {
  const focus = useFocusStore.getState();
  switch (action) {
    case "nextItem":
    case "prevItem":
      return focus.move(reviewId, action === "nextItem" ? 1 : -1);
    case "toggleThread": {
      // A focused button or link handles its own Enter and Space (bound
      // here by a user); other keys (`t`) still act on the focused thread.
      if ((key === "Enter" || key === " ") && ownsEnter(target)) return false;
      const t = focusedThread(reviewId);
      if (!t) return false;
      focus.commandThread(t.threadId, "toggle");
      return true;
    }
    case "replyThread": {
      const t = focusedThread(reviewId);
      if (!t) return false;
      focus.commandThread(t.threadId, "reply");
      return true;
    }
    case "editFinding": {
      const f = focusedFinding(reviewId);
      if (!f || f.postedAt || f.dismissed) return false;
      useAiStore.getState().openEditor(f);
      return true;
    }
    case "dismissFinding": {
      const f = focusedFinding(reviewId);
      if (!f || f.postedAt) return false;
      void useAiStore.getState().dismiss(f, !f.dismissed);
      return true;
    }
    default:
      return null;
  }
}

/** The palette and the help: list scope, so outside fields and dialogs. */
export function runOverlayAction(action: ActionId): boolean | null {
  switch (action) {
    case "openPalette":
      useOverlayStore.getState().show("palette");
      return true;
    case "showShortcuts":
      useOverlayStore.getState().show("help");
      return true;
    default:
      return null;
  }
}
