/**
 * Which file cards must have their diff mounted whatever the viewport
 * says. The review page mounts a card's diff only near the viewport
 * (`FileDiffCard`); a reveal (n/p, the next unresolved thread, the find
 * bar, an AI finding) that targets a card far away pins it here first, so
 * its editor exists to scroll to. A pin ends once the card came near (it
 * then stays by nearness) or after `PIN_MS`.
 */

import { create } from "zustand";

import type { ReviewId } from "../ipc/types";

/** How long a pin holds a card that never came near. */
export const PIN_MS = 10_000;

interface MountState {
  /** `reviewId|path` → when it was pinned. */
  pins: Record<string, number>;
  pin: (reviewId: ReviewId, path: string) => void;
  unpin: (reviewId: ReviewId, path: string) => void;
}

const keyOf = (reviewId: ReviewId, path: string) => `${reviewId}|${path}`;
const timers = new Map<string, ReturnType<typeof setTimeout>>();

export const useMountStore = create<MountState>((set, get) => ({
  pins: {},
  pin: (reviewId, path) => {
    const key = keyOf(reviewId, path);
    clearTimeout(timers.get(key));
    timers.set(
      key,
      setTimeout(() => get().unpin(reviewId, path), PIN_MS),
    );
    set((s) => ({ pins: { ...s.pins, [key]: Date.now() } }));
  },
  unpin: (reviewId, path) => {
    const key = keyOf(reviewId, path);
    clearTimeout(timers.get(key));
    timers.delete(key);
    if (!(key in get().pins)) return;
    set((s) => {
      const pins = { ...s.pins };
      delete pins[key];
      return { pins };
    });
  },
}));

/** Whether `path` of `reviewId` is pinned (a hook). */
export function usePinned(reviewId: ReviewId, path: string): boolean {
  return useMountStore((s) => keyOf(reviewId, path) in s.pins);
}

/** Pin a card: its diff mounts even far from the viewport. */
export function pinCard(reviewId: ReviewId, path: string): void {
  useMountStore.getState().pin(reviewId, path);
}
