/**
 * The local clone of the open review's repository (`clone_status`): set,
 * found, at which commit. Read-only. Re-read when the review opens, when its
 * head or the clone set in Settings changes, and when the window gets the
 * focus back (the user may have moved the clone's branch meanwhile). `null`
 * for a review is a failed read: the chip is then left out.
 *
 * "Show in folder" is the core's `reveal_clone`: the webview never names a
 * path. Failures are toasted with the core's text.
 */

import { useEffect } from "react";
import { create } from "zustand";

import { cloneStatus, revealClone, toIpcError } from "../ipc/commands";
import type { CloneStatus, ReviewId } from "../ipc/types";
import { latestOnly } from "../lib/latest";
import { fileManagerName } from "./localFileStore";
import { toastError } from "./toastStore";

interface CloneStatusState {
  byReview: Record<ReviewId, CloneStatus | null>;
  load: (reviewId: ReviewId) => Promise<void>;
}

const reads = latestOnly<ReviewId>();

export const useCloneStatusStore = create<CloneStatusState>((set) => ({
  byReview: {},

  load: async (reviewId) => {
    const current = reads.begin(reviewId);
    try {
      const status = await cloneStatus(reviewId);
      if (!current()) return;
      set((s) => ({ byReview: { ...s.byReview, [reviewId]: status } }));
    } catch {
      if (!current()) return;
      set((s) => ({ byReview: { ...s.byReview, [reviewId]: null } }));
    }
  },
}));

/** The review's clone status; undefined until the first read answers. */
export function useCloneStatus(
  reviewId: ReviewId,
  /** Anything that should make it re-read: the head, the clone's setting. */
  ...deps: unknown[]
): CloneStatus | null | undefined {
  useEffect(() => {
    const load = () => void useCloneStatusStore.getState().load(reviewId);
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reviewId, ...deps]);
  return useCloneStatusStore((s) => s.byReview[reviewId]);
}

/** Show the review's clone folder in the file manager. */
export async function revealCloneFolder(reviewId: ReviewId): Promise<boolean> {
  try {
    await revealClone(reviewId);
    return true;
  } catch (e) {
    toastError(
      `Could not show the clone in ${fileManagerName}`,
      toIpcError(e).message,
    );
    return false;
  }
}
