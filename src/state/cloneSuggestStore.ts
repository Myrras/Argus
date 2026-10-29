/**
 * The clone Argus found on disk for a review whose repository has none set
 * (`suggest_clone`). Read-only towards the disk, and asked for only when
 * the review is open and the backend says nothing is configured.
 *
 * "Use it" (`use_clone_suggestion`) stores the folder as that repository's
 * clone, "No thanks" (`ignore_clone_suggestion`) is remembered in the
 * settings, so neither comes back; both re-read the backend settings
 * afterwards, which makes the header's `CloneChip` follow. The × only
 * hides it for this session (`dismissed`, in memory): it may come back
 * next time Argus runs.
 */

import { create } from "zustand";

import {
  acceptCloneSuggestion,
  ignoreCloneSuggestion,
  suggestClone,
  toIpcError,
} from "../ipc/commands";
import type { CloneSuggestion, ReviewId } from "../ipc/types";
import { latestOnly } from "../lib/latest";
import { useBackendSettingsStore } from "./backendSettingsStore";
import { toastError, toastSuccess } from "./toastStore";

interface CloneSuggestState {
  /** Per review: the clone found, or `null` when there is none. */
  byReview: Record<ReviewId, CloneSuggestion | null>;
  /** Repositories (`host/repo`) the × hid, this session only. */
  dismissed: Record<string, true>;
  /** Reviews whose "Use it" or "No thanks" is still running. */
  busy: Record<ReviewId, boolean>;
  load: (reviewId: ReviewId) => Promise<void>;
  use: (reviewId: ReviewId, path: string) => Promise<void>;
  ignore: (reviewId: ReviewId) => Promise<void>;
  dismiss: (repo: string) => void;
}

const reads = latestOnly<ReviewId>();

/** The settings the chip and the next suggestion read from. */
async function reloadSettings(): Promise<void> {
  await useBackendSettingsStore.getState().load();
}

export const useCloneSuggestStore = create<CloneSuggestState>((set, get) => ({
  byReview: {},
  dismissed: {},
  busy: {},

  load: async (reviewId) => {
    const current = reads.begin(reviewId);
    let found: CloneSuggestion | null = null;
    try {
      found = await suggestClone(reviewId);
    } catch {
      // A failed scan is no news: the chip already says there is no clone.
      found = null;
    }
    if (!current()) return;
    set((s) => ({ byReview: { ...s.byReview, [reviewId]: found } }));
  },

  use: async (reviewId, path) => {
    if (get().busy[reviewId]) return;
    set((s) => ({ busy: { ...s.busy, [reviewId]: true } }));
    try {
      await acceptCloneSuggestion(reviewId, path);
      set((s) => ({ byReview: { ...s.byReview, [reviewId]: null } }));
      toastSuccess("Local clone set for this repository");
      await reloadSettings();
    } catch (e) {
      toastError("Could not use that clone", toIpcError(e).message);
    } finally {
      set((s) => ({ busy: { ...s.busy, [reviewId]: false } }));
    }
  },

  ignore: async (reviewId) => {
    if (get().busy[reviewId]) return;
    set((s) => ({ busy: { ...s.busy, [reviewId]: true } }));
    try {
      await ignoreCloneSuggestion(reviewId);
      set((s) => ({ byReview: { ...s.byReview, [reviewId]: null } }));
      await reloadSettings();
    } catch (e) {
      toastError("Could not store that choice", toIpcError(e).message);
    } finally {
      set((s) => ({ busy: { ...s.busy, [reviewId]: false } }));
    }
  },

  dismiss: (repo) =>
    set((s) => ({ dismissed: { ...s.dismissed, [repo]: true } })),
}));
