/**
 * The current time for relative ages ("5m", "2d"), one clock for the whole
 * window: `useNowTicker` (in `App`) re-reads it every 30 s, and only what
 * shows an age subscribes (`useNow`), so a tick re-renders those labels
 * rather than every file card down to them. A store, not a context: thread
 * cards render inside CodeMirror widgets, each its own React root.
 */

import { create } from "zustand";

interface NowState {
  now: number;
}

export const useNowStore = create<NowState>(() => ({ now: Date.now() }));
