/**
 * Typed wrappers over the global Tauri events: `src-tauri/src/events.rs`
 * emits the refresh ones after every refresh (poll or manual) and the AI
 * pass' `ai:state`, the submit scripts' `scripts:done` and the "Write
 * with AI" drafts' `script-draft:state` and `script-draft:progress`, the
 * macOS menu
 * (`lib.rs`) the Settings one.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AiRunState,
  RefreshReport,
  ReviewChangedEvent,
  ScriptDraft,
  ScriptDraftProgressEvent,
  ScriptsDone,
} from "./types";

export const REVIEW_CHANGED = "review:changed";
export const REVIEWS_REFRESHED = "reviews:refreshed";
export const AI_STATE = "ai:state";
export const SCRIPTS_DONE = "scripts:done";
export const SCRIPT_DRAFT_STATE = "script-draft:state";
export const SCRIPT_DRAFT_PROGRESS = "script-draft:progress";

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

/** The native macOS menu's Settings… item (⌘,) was activated (`lib.rs`). */
export function onMenuOpenSettings(handler: () => void): Promise<UnlistenFn> {
  return listen("menu:open-settings", () => handler());
}

// --- AI pass (M5) ---

/**
 * An AI run's state changed (queued, running, done, failed, cancelled). A
 * run that ended is followed by `review:changed` for its review.
 */
export function onAiState(
  handler: (state: AiRunState) => void,
): Promise<UnlistenFn> {
  return listen<AiRunState>(AI_STATE, (e) => handler(e.payload));
}

// --- Submit scripts ---

/** The scripts one submit ran have all ended: one result per ticked script. */
export function onScriptsDone(
  handler: (done: ScriptsDone) => void,
): Promise<UnlistenFn> {
  return listen<ScriptsDone>(SCRIPTS_DONE, (e) => handler(e.payload));
}

/** A "Write with AI" draft changed (running, waiting, idle, failed, stopped, linked). */
export function onScriptDraftState(
  handler: (draft: ScriptDraft) => void,
): Promise<UnlistenFn> {
  return listen<ScriptDraft>(SCRIPT_DRAFT_STATE, (e) => handler(e.payload));
}

/** A running draft's progress: stage, thinking, answer size (≤ 10 a second). */
export function onScriptDraftProgress(
  handler: (event: ScriptDraftProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ScriptDraftProgressEvent>(SCRIPT_DRAFT_PROGRESS, (e) =>
    handler(e.payload),
  );
}
