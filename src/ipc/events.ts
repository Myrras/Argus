/**
 * Typed wrappers over the global Tauri events `src-tauri/src/events.rs`
 * emits after every refresh (poll or manual).
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { RefreshReport, ReviewChangedEvent } from "./types";

export const REVIEW_CHANGED = "review:changed";
export const REVIEWS_REFRESHED = "reviews:refreshed";

/** One review's stored state changed (emitted per review, before the report). */
export function onReviewChanged(
  handler: (event: ReviewChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<ReviewChangedEvent>(REVIEW_CHANGED, (e) => handler(e.payload));
}

/** A refresh finished: its time, changed ids and per-provider errors. */
export function onReviewsRefreshed(
  handler: (report: RefreshReport) => void,
): Promise<UnlistenFn> {
  return listen<RefreshReport>(REVIEWS_REFRESHED, (e) => handler(e.payload));
}
