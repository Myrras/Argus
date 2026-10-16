import { useEffect } from "react";

import { useNowStore } from "../state/nowStore";

/** The current time in ms (`nowStore`), re-read on the window's clock tick. */
export function useNow(): number {
  return useNowStore((s) => s.now);
}

/** Tick the window's clock every `intervalMs` (once, in `App`). */
export function useNowTicker(intervalMs = 30_000): void {
  useEffect(() => {
    useNowStore.setState({ now: Date.now() });
    const id = setInterval(
      () => useNowStore.setState({ now: Date.now() }),
      intervalMs,
    );
    return () => clearInterval(id);
  }, [intervalMs]);
}
