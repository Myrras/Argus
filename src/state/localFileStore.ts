/**
 * Open in editor: the open review's files in its local clone (`local_files`,
 * one call per review and file list), so the file tree and the file cards
 * know whether to offer the editor, disable it ("Not in your working
 * tree") or hint that the clone is on another commit. `null` for a review
 * means no clone is set: nothing is offered. Re-read when the file list
 * changes and when the window gets the focus back (the user may have
 * checked out another branch meanwhile).
 *
 * Opening and revealing are core commands (`open_in_editor`,
 * `reveal_local_file`) keyed by review and repository path: the webview
 * never names a local path. Failures are toasted with the core's text.
 */

import { useEffect } from "react";
import { create } from "zustand";

import {
  localFiles,
  openInEditor,
  revealLocalFile,
  toIpcError,
} from "../ipc/commands";
import type { LocalFile, ReviewId } from "../ipc/types";
import { latestOnly } from "../lib/latest";
import { isMac, isWindows } from "../lib/platform";
import { toastError } from "./toastStore";

/** What the OS calls its file manager (Iris's `fileManagerName`). */
export const fileManagerName = isMac
  ? "Finder"
  : isWindows
    ? "Explorer"
    : "File Manager";

interface LocalFileState {
  /** Per review: path → file, or `null` without a clone. */
  byReview: Record<ReviewId, Record<string, LocalFile> | null>;
  load: (reviewId: ReviewId, paths: string[]) => Promise<void>;
  /** Forget a review (its clone was removed in Settings). */
  clear: (reviewId: ReviewId) => void;
}

const reads = latestOnly<ReviewId>();

export const useLocalFileStore = create<LocalFileState>((set) => ({
  byReview: {},

  load: async (reviewId, paths) => {
    const current = reads.begin(reviewId);
    try {
      const files = await localFiles(reviewId, paths);
      if (!current()) return;
      set((s) => ({
        byReview: {
          ...s.byReview,
          [reviewId]:
            files && Object.fromEntries(files.map((f) => [f.path, f])),
        },
      }));
    } catch {
      // Nothing offered then; the review itself is unaffected.
      if (!current()) return;
      set((s) => ({ byReview: { ...s.byReview, [reviewId]: null } }));
    }
  },

  clear: (reviewId) => {
    // A read still under way must not put the files back.
    reads.begin(reviewId);
    set((s) => {
      if (!(reviewId in s.byReview)) return s;
      const next = { ...s.byReview };
      delete next[reviewId];
      return { byReview: next };
    });
  },
}));

/** The review's file in its clone; undefined when none is offered. */
export function useLocalFile(
  reviewId: ReviewId,
  path: string,
): LocalFile | undefined {
  return useLocalFileStore((s) => s.byReview[reviewId]?.[path]);
}

export function localFile(
  reviewId: ReviewId,
  path: string,
): LocalFile | undefined {
  return useLocalFileStore.getState().byReview[reviewId]?.[path];
}

/**
 * Keep the review's files in its clone loaded while it shows: on open,
 * when the paths change, and when the window gets the focus back.
 * `enabled` is false without a clone for the repository.
 */
export function useLocalFiles(
  reviewId: ReviewId,
  paths: readonly string[] | null,
  enabled: boolean,
) {
  const key = paths ? paths.join("\n") : null;
  useEffect(() => {
    if (!enabled || key === null) {
      useLocalFileStore.getState().clear(reviewId);
      return;
    }
    const list = key === "" ? [] : key.split("\n");
    const load = () => void useLocalFileStore.getState().load(reviewId, list);
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [reviewId, key, enabled]);
}

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** Paths being opened, so a double click opens once (Iris's guard). */
const opening = new Set<string>();

/** Open the review's file in its editor; resolves true once asked. */
export async function openLocalFile(
  reviewId: ReviewId,
  path: string,
  useDefault = false,
): Promise<boolean> {
  const key = `${reviewId}\n${path}`;
  if (opening.has(key)) return false;
  opening.add(key);
  try {
    await openInEditor(reviewId, path, useDefault);
    return true;
  } catch (e) {
    toastError(`Could not open ${fileName(path)}`, toIpcError(e).message);
    return false;
  } finally {
    opening.delete(key);
  }
}

/** Show the review's file in the file manager. */
export async function revealFile(
  reviewId: ReviewId,
  path: string,
): Promise<boolean> {
  try {
    await revealLocalFile(reviewId, path);
    return true;
  } catch (e) {
    toastError(
      `Could not show ${fileName(path)} in ${fileManagerName}`,
      toIpcError(e).message,
    );
    return false;
  }
}
