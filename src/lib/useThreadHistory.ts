/**
 * A thread's history from `historyStore` (loading and errors per thread,
 * falling back to its review's read), for `ThreadBlock`. Asks for the
 * thread on its own when the review's last read did not have it (a thread
 * newer than the read).
 */

import { useEffect } from "react";

import type { IpcError, StoredThread, ThreadHistory } from "../ipc/types";
import { useHistoryStore } from "../state/historyStore";

export interface ThreadHistoryState {
  history: ThreadHistory | null;
  loading: boolean;
  error: IpcError | null;
  retry: () => void;
}

export function useThreadHistory(t: StoredThread): ThreadHistoryState {
  const entry = useHistoryStore((s) => s.byThread[t.threadId]);
  const review = useHistoryStore((s) => s.byReview[t.reviewId]);
  const loadThread = useHistoryStore((s) => s.loadThread);
  // The review's read succeeded without this thread (newer than the read).
  const missing =
    !entry && !!review?.loaded && !review.loading && !review.error;
  useEffect(() => {
    if (missing) void loadThread(t.threadId);
  }, [missing, loadThread, t.threadId]);
  return {
    history: entry?.history ?? null,
    loading: !!entry?.loading || (!entry?.history && !!review?.loading),
    error: entry?.error ?? (entry?.history ? null : (review?.error ?? null)),
    retry: () => void loadThread(t.threadId),
  };
}
