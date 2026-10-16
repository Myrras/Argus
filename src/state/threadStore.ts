/**
 * Threads per review, loaded when a review is opened and re-read when the
 * backend reports that review changed. Only the newest read of a review
 * applies its answer.
 */

import { create } from "zustand";

import { listThreads, toIpcError } from "../ipc/commands";
import { onReviewChanged } from "../ipc/events";
import type { IpcError, ReviewId, StoredThread } from "../ipc/types";
import { latestOnly } from "../lib/latest";

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
}));

/** Follow `review:changed`; resolves to an unsubscribe. */
export function subscribeThreadEvents(): Promise<() => void> {
  return onReviewChanged(
    (e) => void useThreadStore.getState().applyChanged(e.reviewId),
  );
}
