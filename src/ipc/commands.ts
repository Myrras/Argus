/**
 * Typed wrappers over every Tauri IPC command.
 *
 * All command rejections are `IpcError` shapes from the Rust side;
 * `toIpcError` normalizes anything else (bridge failures, plugin errors) into
 * the same shape so callers have exactly one error type to handle.
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  CommentableLines,
  IpcError,
  NewComment,
  PostedNote,
  PostResult,
  ProviderStatus,
  RefreshReport,
  ReviewFiles,
  ReviewId,
  ReviewRecord,
  Settings,
  StoredThread,
  ThreadId,
} from "./types";

/** Narrow an unknown rejection to the structured `IpcError` shape. */
export function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as IpcError).message === "string" &&
    typeof (value as IpcError).kind === "string"
  );
}

/** Coerce any rejection into an `IpcError` (kind `"unknown"` as fallback). */
export function toIpcError(value: unknown): IpcError {
  if (isIpcError(value)) return value;
  if (value instanceof Error)
    return { message: value.message, kind: "unknown" };
  return { message: String(value), kind: "unknown" };
}

/** The reviews shown in the tabs, most recently updated first. */
export function listReviews(): Promise<ReviewRecord[]> {
  return invoke("list_reviews");
}

export function getReview(reviewId: ReviewId): Promise<ReviewRecord> {
  return invoke("get_review", { reviewId });
}

/** The stored threads of a review, oldest first. */
export function listThreads(reviewId: ReviewId): Promise<StoredThread[]> {
  return invoke("list_threads", { reviewId });
}

/**
 * The review's changed files with their viewed marks: its own diff, or with
 * `sinceLastReview` the diff from the last-reviewed commit to head.
 */
export function listFiles(
  reviewId: ReviewId,
  sinceLastReview: boolean,
): Promise<ReviewFiles> {
  return invoke("list_files", { reviewId, sinceLastReview });
}

/** The content of `path` at `sha` (a sha `listFiles` returned). */
export function fileContents(
  reviewId: ReviewId,
  sha: string,
  path: string,
): Promise<string> {
  return invoke("file_contents", { reviewId, sha, path });
}

/** Mark a file viewed or not; resolves to the review's viewed paths. */
export function setFileViewed(
  reviewId: ReviewId,
  path: string,
  viewed: boolean,
): Promise<string[]> {
  return invoke("set_file_viewed", { reviewId, path, viewed });
}

/** Refresh every provider now; emits the same events a poll does. */
export function refreshReviews(): Promise<RefreshReport> {
  return invoke("refresh_reviews");
}

/** The last refresh (poll or manual), `null` before the first. */
export function lastRefresh(): Promise<RefreshReport | null> {
  return invoke("last_refresh");
}

/** The user finished this round: the watermark moves to the current head. */
export function markReviewed(reviewId: ReviewId): Promise<ReviewRecord> {
  return invoke("mark_reviewed", { reviewId });
}

// --- Posting. The only writes to a platform: call these from a button or
// shortcut handler only, never from an effect or a timer.

/**
 * Post line comments one by one; one result per comment in the same order.
 * Rejects only when nothing could be tried.
 */
export function postComments(
  reviewId: ReviewId,
  comments: NewComment[],
): Promise<PostResult[]> {
  return invoke("post_comments", { reviewId, comments });
}

/** Reply to a stored thread of `reviewId`. */
export function replyToThread(
  reviewId: ReviewId,
  threadId: ThreadId,
  body: string,
): Promise<PostedNote> {
  return invoke("reply_to_thread", { reviewId, threadId, body });
}

/**
 * Approve a review at `headSha` (the head the user confirmed); rejects with
 * `headMoved` when it got new commits since. Never merges.
 */
export function approveReview(
  reviewId: ReviewId,
  headSha: string,
): Promise<ReviewRecord> {
  return invoke("approve_review", { reviewId, headSha });
}

/** The lines of `path` a new comment can go on (read only). */
export function commentableLines(
  reviewId: ReviewId,
  path: string,
): Promise<CommentableLines> {
  return invoke("commentable_lines", { reviewId, path });
}

export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

/** Validate, save and apply; resolves to what was stored. Never a token. */
export function updateSettings(settings: Settings): Promise<Settings> {
  return invoke("update_settings", { settings });
}

/** Store the GitLab token in the OS keychain; blank deletes it. */
export function setGitlabToken(token: string): Promise<void> {
  return invoke("set_gitlab_token", { token });
}

export function gitlabTokenPresent(): Promise<boolean> {
  return invoke("gitlab_token_present");
}

export function providerStatus(): Promise<ProviderStatus> {
  return invoke("provider_status");
}
