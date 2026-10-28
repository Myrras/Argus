/**
 * Threads per review, loaded when a review is opened and re-read when the
 * backend reports that review changed. Only the newest read of a review
 * applies its answer.
 *
 * `setResolved` resolves or reopens a thread on its platform: a write, so
 * only ever called from the thread's button, its key (`s`) or the palette.
 * The thread then shows the state the backend reports after its refresh.
 */

import { create } from "zustand";

import { listThreads, resolveThread, toIpcError } from "../ipc/commands";
import { onReviewChanged } from "../ipc/events";
import type { IpcError, ReviewId, StoredThread, ThreadId } from "../ipc/types";
import { latestOnly } from "../lib/latest";
import { toastError } from "./toastStore";

export interface ReviewThreads {
  threads: StoredThread[];
  loading: boolean;
  error: IpcError | null;
}

interface ThreadStoreState {
  byReview: Record<ReviewId, ReviewThreads>;
  load: (id: ReviewId) => Promise<void>;
  /** `review:changed`: reload when that review's threads are loaded. */
  applyChanged: (id: ReviewId) => Promise<void>;
  /** Threads with a resolve or reopen on its way. */
  resolving: Record<ThreadId, boolean>;
  /**
   * Resolve (`resolved: true`) or reopen `thread` on its platform, then
   * re-read the review's threads. A failure is a toast with the
   * platform's reason. False when nothing was written (not resolvable,
   * already under way, or failed).
   */
  setResolved: (thread: StoredThread, resolved: boolean) => Promise<boolean>;
}

const EMPTY: ReviewThreads = { threads: [], loading: false, error: null };
const reads = latestOnly<ReviewId>();

export const useThreadStore = create<ThreadStoreState>((set, get) => ({
  byReview: {},

  load: async (id) => {
    const current = reads.begin(id);
    set((s) => ({
      byReview: {
        ...s.byReview,
        [id]: { ...(s.byReview[id] ?? EMPTY), loading: true },
      },
    }));
    try {
      const threads = await listThreads(id);
      if (!current()) return;
      set((s) => ({
        byReview: {
          ...s.byReview,
          [id]: { threads, loading: false, error: null },
        },
      }));
    } catch (e) {
      if (!current()) return;
      set((s) => ({
        byReview: {
          ...s.byReview,
          [id]: {
            ...(s.byReview[id] ?? EMPTY),
            loading: false,
            error: toIpcError(e),
          },
        },
      }));
    }
  },

  applyChanged: async (id) => {
    if (get().byReview[id]) await get().load(id);
  },

  resolving: {},

  setResolved: async (t, resolved) => {
    if (!t.resolvable || get().resolving[t.threadId]) return false;
    const busy = (on: boolean) =>
      set((s) => {
        const resolving = { ...s.resolving };
        if (on) resolving[t.threadId] = true;
        else delete resolving[t.threadId];
        return { resolving };
      });
    busy(true);
    try {
      await resolveThread(t.reviewId, t.threadId, resolved);
    } catch (e) {
      busy(false);
      const err = toIpcError(e);
      toastError(
        resolved
          ? "Could not resolve the thread"
          : "Could not reopen the thread",
        err.message,
      );
      return false;
    }
    // The badge follows the backend's own answer, not a guess.
    await get().load(t.reviewId);
    busy(false);
    return true;
  },
}));

/** Follow `review:changed`; resolves to an unsubscribe. */
export function subscribeThreadEvents(): Promise<() => void> {
  return onReviewChanged(
    (e) => void useThreadStore.getState().applyChanged(e.reviewId),
  );
}
