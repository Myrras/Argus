/**
 * Typed wrappers over every Tauri IPC command.
 *
 * All command rejections are `IpcError` shapes from the Rust side;
 * `toIpcError` normalizes anything else (bridge failures, plugin errors) into
 * the same shape so callers have exactly one error type to handle.
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  AiRunState,
  ChecklistId,
  ChecklistItem,
  CloneStatus,
  CommentableLines,
  Finding,
  FindingId,
  FindingPost,
  FindingPostResult,
  IpcError,
  LocalFile,
  NewComment,
  PostedNote,
  PostResult,
  ProviderStatus,
  RefreshReport,
  ReviewFiles,
  ReviewId,
  ReviewRecord,
  ReviewRefresh,
  ReviewVerdict,
  Settings,
  StoredThread,
  ThreadHistory,
  ThreadId,
  Verdict,
  VerdictLabel,
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

/** What the UI says for an `internal` failure (a crashed background task). */
export const INTERNAL_MESSAGE = "Something went wrong in Argus; see the log";

/**
 * Coerce any rejection into an `IpcError` (kind `"unknown"` as fallback).
 * An `internal` one carries no reason the user can act on: it reads as a
 * generic error pointing at the log.
 */
export function toIpcError(value: unknown): IpcError {
  if (isIpcError(value)) {
    return value.kind === "internal"
      ? { kind: "internal", message: INTERNAL_MESSAGE }
      : value;
  }
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

/**
 * Refresh one review now; emits `review:changed` when it changed. Rejects
 * with kind `rateLimited` (the message says until when) when its provider
 * is rate-limited, without calling it.
 */
export function refreshReview(reviewId: ReviewId): Promise<ReviewRefresh> {
  return invoke("refresh_review", { reviewId });
}

/** The last refresh (poll or manual), `null` before the first. */
export function lastRefresh(): Promise<RefreshReport | null> {
  return invoke("last_refresh");
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

/**
 * Resolve (`resolved: true`) or reopen a stored thread of `reviewId` on its
 * platform; the thread as stored afterwards. Only for a `resolvable` thread.
 */
export function resolveThread(
  reviewId: ReviewId,
  threadId: ThreadId,
  resolved: boolean,
): Promise<StoredThread> {
  return invoke("resolve_thread", { reviewId, threadId, resolved });
}

/**
 * Submit the user's review: `verdict` pinned to `headSha` (the head the
 * dialog showed), with an optional markdown `summary`. Sets the review
 * state on the platform; rejects with `headMoved` when it got new commits
 * since, `unsupported` when the platform cannot. Never merges.
 */
export function submitReview(
  reviewId: ReviewId,
  headSha: string,
  verdict: ReviewVerdict,
  summary: string | null,
): Promise<ReviewRecord> {
  return invoke("submit_review", { reviewId, headSha, verdict, summary });
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

// --- Thread history (M3) ---
// Read only towards the platforms. A call may rewrite the review's untouched
// count; the backend then emits `review:changed` for it.

/**
 * The then/now history of one thread at its review's current head (computed
 * on a cache miss, so it can take a moment).
 */
export function threadHistory(threadId: ThreadId): Promise<ThreadHistory> {
  return invoke("thread_history", { threadId });
}

/** The histories of every thread of a review. */
export function reviewHistories(reviewId: ReviewId): Promise<ThreadHistory[]> {
  return invoke("review_histories", { reviewId });
}

// --- AI pass (M5) ---
// Runs happen in the backend and report through `ai:state` (`onAiState`).
// `postFindings` writes to a platform: call it from a button or shortcut
// handler only, never from an effect or a timer.

/** The review's latest run (`notStarted` when none, or the AI pass is off). */
export function aiState(reviewId: ReviewId): Promise<AiRunState> {
  return invoke("ai_state", { reviewId });
}

/** The review's findings, every round: high severity first. */
export function listFindings(reviewId: ReviewId): Promise<Finding[]> {
  return invoke("list_findings", { reviewId });
}

/** The review's "look here yourself" items, newest round first. */
export function listChecklist(reviewId: ReviewId): Promise<ChecklistItem[]> {
  return invoke("list_checklist", { reviewId });
}

/** The current verdict per thread of the review. */
export function listVerdicts(reviewId: ReviewId): Promise<Verdict[]> {
  return invoke("list_verdicts", { reviewId });
}

/**
 * The review's findings set aside as already raised in the review (an
 * existing thread or a posted finding makes their point), with what each
 * repeats.
 */
export function listSkippedFindings(reviewId: ReviewId): Promise<Finding[]> {
  return invoke("list_skipped_findings", { reviewId });
}

/** Show a set-aside finding anyway: it becomes a normal finding. Local only. */
export function showFindingAnyway(findingId: FindingId): Promise<Finding> {
  return invoke("show_finding_anyway", { findingId });
}

/** The review's checklist items set aside as already raised in the review. */
export function listSkippedChecklist(
  reviewId: ReviewId,
): Promise<ChecklistItem[]> {
  return invoke("list_skipped_checklist", { reviewId });
}

/** Show a set-aside checklist item anyway: it becomes a normal item. Local only. */
export function showChecklistItemAnyway(
  checklistId: ChecklistId,
): Promise<ChecklistItem> {
  return invoke("show_checklist_item_anyway", { checklistId });
}

/** Queue a run now; rejects with `invalidInput` when the AI pass is off. */
export function runAi(reviewId: ReviewId): Promise<AiRunState> {
  return invoke("run_ai", { reviewId });
}

/** Drop the review's queued run or stop its running one. */
export function cancelAi(reviewId: ReviewId): Promise<AiRunState> {
  return invoke("cancel_ai", { reviewId });
}

/** Dismiss a finding, or bring it back. */
export function dismissFinding(
  findingId: FindingId,
  dismissed: boolean,
): Promise<Finding> {
  return invoke("dismiss_finding", { findingId, dismissed });
}

export function setChecklistChecked(
  checklistId: ChecklistId,
  checked: boolean,
): Promise<ChecklistItem> {
  return invoke("set_checklist_checked", { checklistId, checked });
}

/** The user's own label on a thread; `null` clears it. Local only. */
export function setVerdictOverride(
  threadId: ThreadId,
  label: VerdictLabel | null,
): Promise<Verdict> {
  return invoke("set_verdict_override", { threadId, label });
}

/**
 * Post findings as line comments, one by one; one result per finding in the
 * same order. Rejects only when nothing could be tried.
 */
export function postFindings(
  reviewId: ReviewId,
  posts: FindingPost[],
): Promise<FindingPostResult[]> {
  return invoke("post_findings", { reviewId, posts });
}

// --- Open in editor ---

/**
 * The review's `paths` in its local clone; `null` when no clone is set for
 * its repository. Paths the clone cannot hold are left out.
 */
export function localFiles(
  reviewId: ReviewId,
  paths: string[],
): Promise<LocalFile[] | null> {
  return invoke("local_files", { reviewId, paths });
}

/**
 * Open the review's file from its clone in the editor set for it
 * (Settings → Editors), or with `useDefault` in the OS default app.
 */
export function openInEditor(
  reviewId: ReviewId,
  path: string,
  useDefault = false,
): Promise<void> {
  return invoke("open_in_editor", { reviewId, path, useDefault });
}

/** What is set and found for the review's repository (read-only). */
export function cloneStatus(reviewId: ReviewId): Promise<CloneStatus> {
  return invoke("clone_status", { reviewId });
}

/** Show the review's clone folder in Finder / Explorer. */
export function revealClone(reviewId: ReviewId): Promise<void> {
  return invoke("reveal_clone", { reviewId });
}

/** Show the review's file from its clone in Finder / Explorer. */
export function revealLocalFile(
  reviewId: ReviewId,
  path: string,
): Promise<void> {
  return invoke("reveal_local_file", { reviewId, path });
}
