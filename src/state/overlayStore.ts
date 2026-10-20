/**
 * The keyboard overlays (M6): the command palette (⌘K) and the shortcut
 * help (?), at most one at a time, and the dialogs the palette and keys
 * open for the actions that write to a platform (Submit review…, Approve…,
 * Post all…) — neither ever posts by itself.
 */

import { create } from "zustand";

import type { ReviewId, ReviewVerdict } from "../ipc/types";

export type Overlay = "palette" | "help";

export type Confirm =
  | {
      /** The Submit review dialog, with `verdict` chosen (Approve…). */
      kind: "submit";
      reviewId: ReviewId;
      verdict?: ReviewVerdict;
    }
  | { kind: "postAll"; reviewId: ReviewId };

interface OverlayState {
  open: Overlay | null;
  confirm: Confirm | null;
  /** Open `overlay` (replacing the other one). */
  show: (overlay: Overlay) => void;
  close: () => void;
  ask: (confirm: Confirm) => void;
  closeConfirm: () => void;
}

export const useOverlayStore = create<OverlayState>((set) => ({
  open: null,
  confirm: null,
  show: (overlay) => set({ open: overlay }),
  close: () => set({ open: null }),
  ask: (confirm) => set({ open: null, confirm }),
  closeConfirm: () => set({ confirm: null }),
}));
