/**
 * The submit scripts' last runs, from `scripts:done`: the latest result per
 * script (and the review it ran for), for the Scripts tab's cards. Memory
 * only: a restart starts empty. Each `scripts:done` also toasts (failures
 * with the script's name and the tail of its stderr or Argus' message).
 */

import { create } from "zustand";

import { onScriptsDone } from "../ipc/events";
import type { ReviewId, ScriptResult, ScriptsDone } from "../ipc/types";
import { doneToasts } from "../lib/submitScripts";
import { toastError, toastInfo, toastSuccess } from "./toastStore";

export interface LastRun {
  result: ScriptResult;
  reviewId: ReviewId;
}

interface ScriptResultsState {
  /** Script id → its latest run. */
  last: Record<string, LastRun>;
  apply: (done: ScriptsDone) => void;
}

export const useScriptResultsStore = create<ScriptResultsState>((set) => ({
  last: {},
  apply: (done) =>
    set((s) => ({
      last: {
        ...s.last,
        ...Object.fromEntries(
          done.results.map((result) => [
            result.scriptId,
            { result, reviewId: done.reviewId },
          ]),
        ),
      },
    })),
}));

/** Store one `scripts:done` and show its toasts. */
export function applyScriptsDone(done: ScriptsDone): void {
  useScriptResultsStore.getState().apply(done);
  for (const t of doneToasts(done)) {
    if (t.kind === "error") toastError(t.message, t.detail);
    else if (t.kind === "success") toastSuccess(t.message);
    else toastInfo(t.message, t.detail);
  }
}

/** Listen for `scripts:done`; resolves to an unsubscribe. */
export function subscribeScriptEvents(): Promise<() => void> {
  return onScriptsDone(applyScriptsDone);
}
