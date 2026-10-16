/**
 * Motion primitives shared across the UI.
 *
 * Most animation is pure CSS (transitions + keyframes driven by the `--dur-*` /
 * `--ease-*` tokens in `tokens.css`). This module exposes the small JS pieces
 * CSS can't do on its own — chiefly the reduced-motion query used to shorten or
 * skip JS-driven timing.
 */

/** Duration scale in milliseconds — mirrors the `--dur-*` tokens. */
export const MOTION = {
  micro: 90,
  fast: 150,
  base: 220,
  slow: 360,
} as const;

/**
 * Whether the user (or OS) asked for reduced motion. Guarded so it is safe in
 * non-browser/test environments where `matchMedia` is absent (jsdom): we treat
 * "unknown" as "motion allowed", since the global CSS guard is the real
 * enforcement point and this only shortens JS timers.
 */
export function prefersReducedMotion(): boolean {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return false;
  }
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
