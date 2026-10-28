/**
 * Open in editor: what the file tree, the file cards, the keys and the
 * palette offer for a review's file in its local clone (`localFileStore`).
 * The editor opens the file, not a line: Iris (whose mechanism this is)
 * has no line support, so a focused line or thread opens its file.
 */

import type { LocalFile, ReviewId } from "../ipc/types";
import type { ContextMenuItem } from "../components/ContextMenu";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import { useFocusStore } from "../state/focusStore";
import {
  fileManagerName,
  localFile,
  openLocalFile,
  revealFile,
} from "../state/localFileStore";
import { toastInfo } from "../state/toastStore";
import { CONVERSATION_CARD, cardOf } from "./focusModel";
import type { ActionId } from "./keymap";

export const NOT_IN_WORKING_TREE = "Not in your working tree";

/** "Your clone is on abc1234, not this review's head", or null. */
export function headHint(file: LocalFile): string | null {
  if (file.matchesReviewHead || !file.cloneHead) return null;
  return `Your clone is on ${file.cloneHead.slice(0, 7)}, not this review's head`;
}

/** The editor button's tooltip; `action` is "Open in editor (O)". */
export function openTitle(file: LocalFile, action: string): string {
  const head = headHint(file);
  const first = file.exists ? action : NOT_IN_WORKING_TREE;
  return head ? `${first}\n${head}` : first;
}

/** The file menu's items (Iris's Local pane menu), or none without a clone. */
export function localFileMenuItems(
  reviewId: ReviewId,
  path: string,
  hints: { open?: string; reveal?: string } = {},
): ContextMenuItem[] {
  const file = localFile(reviewId, path);
  if (!file) return [];
  return [
    {
      label: "Open in Editor",
      shortcut: hints.open,
      disabled: !file.exists,
      onSelect: () => void openLocalFile(reviewId, path),
    },
    {
      label: "Open With Default App",
      disabled: !file.exists,
      onSelect: () => void openLocalFile(reviewId, path, true),
    },
    {
      label: `Show in ${fileManagerName}`,
      shortcut: hints.reveal,
      disabled: !file.exists,
      onSelect: () => void revealFile(reviewId, path),
    },
  ];
}

/**
 * The file the keys act on: the focused item's (a file, a line in it, a
 * thread or finding on it), else the selected file. Null on the
 * Conversation.
 */
export function focusedFilePath(reviewId: ReviewId): string | null {
  const { item, reviewId: at } = useFocusStore.getState();
  if (item && at === reviewId) {
    const card = cardOf(item);
    return card === CONVERSATION_CARD ? null : card;
  }
  const selected = useCodeViewStore.getState().byReview[reviewId]?.selected;
  return selected && selected !== CONVERSATION ? selected : null;
}

/**
 * `openInEditor` / `showInFinder` on the focused file; null for any other
 * action, false when there is nothing to open (no clone, no file).
 */
export function runLocalFileAction(
  action: ActionId,
  reviewId: ReviewId,
): boolean | null {
  if (action !== "openInEditor" && action !== "showInFinder") return null;
  const path = focusedFilePath(reviewId);
  const file = path ? localFile(reviewId, path) : undefined;
  if (!path || !file) return false;
  if (!file.exists) {
    toastInfo(NOT_IN_WORKING_TREE, path);
    return true;
  }
  if (action === "openInEditor") void openLocalFile(reviewId, path);
  else void revealFile(reviewId, path);
  return true;
}
