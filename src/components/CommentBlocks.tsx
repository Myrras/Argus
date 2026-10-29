/**
 * The comment blocks of the review view, each reading `commentStore` itself
 * (they render inside CodeMirror block widgets, each its own React root, so
 * they cannot take live props):
 *
 * - `InlineComposer`: the open composer under its line — a new comment
 *   ("Add to review" queues a draft, "Comment now" posts just this one) or
 *   a draft's edit ("Save").
 * - `DraftBlock`: a pending comment under its line, with Edit / Delete, and
 *   for one that failed the platform's reason and Retry.
 * - `ReplyComposer`: a thread's reply composer.
 *
 * The composers on code lines get suggestion tools (`useComposerSuggestion`):
 * a new comment on new-side lines, a reply in a thread on new-side lines
 * (its suggestion applies to the thread's lines on both platforms). A
 * draft previews its suggestion blocks the same way.
 */

import { useEffect, useRef } from "react";

import type {
  FileEntry,
  ProviderId,
  ReviewId,
  StoredThread,
  ThreadId,
} from "../ipc/types";
import { outsideNote, rangeLabel } from "../lib/commentRanges";
import { FOCUS, takeFocus } from "../lib/focusRequests";
import { pauseScrollAnchor } from "../lib/scrollAnchor";
import {
  commentSuggestionPlan,
  replySuggestionPlan,
  type SuggestionPlan,
} from "../lib/suggestions";
import { useComposerSuggestion } from "../lib/useComposerSuggestion";
import { useCodeViewStore } from "../state/codeViewStore";
import { useCommentStore, type Draft } from "../state/commentStore";
import { useReviewStore } from "../state/reviewStore";
import { Badge } from "./Badge";
import { CommentComposer } from "./CommentComposer";
import { Markdown } from "./Markdown";
import styles from "./CommentBlocks.module.css";

/** The review's platform (null when the review is not in the list). */
function useProvider(reviewId: ReviewId | null): ProviderId | null {
  return useReviewStore((s) =>
    reviewId
      ? (s.reviews.find((r) => r.id === reviewId)?.provider ?? null)
      : null,
  );
}

/** A file of the review's list (null when not listed or not loaded). */
function useFileEntry(
  reviewId: ReviewId | null,
  path: string | null,
): FileEntry | null {
  return useCodeViewStore((s) =>
    reviewId && path
      ? (s.byReview[reviewId]?.files?.files.find((f) => f.path === path) ??
        null)
      : null,
  );
}

const NO_PLAN: SuggestionPlan = { ok: false, reason: "" };

/** Where a new comment's (or a draft's) suggestion goes. */
function usePlanFor(
  reviewId: ReviewId | null,
  provider: ProviderId | null,
  t: Pick<
    Draft,
    "path" | "side" | "line" | "startLine" | "headSha" | "outside"
  > | null,
): SuggestionPlan {
  const file = useFileEntry(reviewId, t?.path ?? null);
  if (!t || !provider) return NO_PLAN;
  return commentSuggestionPlan({ ...t, provider, file });
}

export function InlineComposer({ onResize }: { onResize?: () => void }) {
  const c = useCommentStore((s) => s.composer);
  const setBody = useCommentStore((s) => s.setComposerBody);
  const close = useCommentStore((s) => s.closeComposer);
  const save = useCommentStore((s) => s.saveComposer);
  const commentNow = useCommentStore((s) => s.commentNow);
  const setSuggestedFor = useCommentStore((s) => s.setSuggestedFor);
  const setRange = useCommentStore((s) => s.setComposerRange);
  // A draft's Save waits while its review posts: the draft may be on its
  // way with its old text.
  const posting = useCommentStore((s) => (c ? !!s.posting[c.reviewId] : false));
  const reviewId = c?.reviewId ?? null;
  const provider = useProvider(reviewId);
  const plan = usePlanFor(reviewId, provider, c);
  const suggestion = useComposerSuggestion({
    reviewId: reviewId ?? "",
    provider,
    plan,
    insertedFor: c?.suggestedFor ?? null,
    onInserted: setSuggestedFor,
    // A draft's edit keeps its lines.
    onUseRange: c?.draftId ? undefined : setRange,
    onResize,
  });
  if (!c) return null;
  const where = rangeLabel(c.side, c);
  const note = outsideNote(c.outside);
  return (
    <div className={styles.block}>
      {note && (
        <p className={styles.outsideNote} data-testid="outside-note">
          {note}
        </p>
      )}
      <CommentComposer
        label={
          c.draftId ? `Edit pending comment on ${where}` : `Comment on ${where}`
        }
        body={c.body}
        onChange={setBody}
        busy={c.busy}
        error={c.error}
        onCancel={close}
        onResize={onResize}
        focusKey={FOCUS.composer}
        suggestion={suggestion}
        primary={
          c.draftId
            ? {
                label: "Save",
                onClick: save,
                disabled: posting,
                title: posting ? "Wait for the post to finish" : undefined,
              }
            : {
                label: "Add to review",
                title:
                  "Keep it as a pending comment; nothing is posted until Post all",
                onClick: save,
              }
        }
        secondary={
          c.draftId
            ? undefined
            : {
                label: "Comment now",
                busyLabel: "Posting…",
                title: "Post just this comment now",
                onClick: () => void commentNow(),
              }
        }
      />
    </div>
  );
}

export function DraftBlock({
  reviewId,
  id,
}: {
  reviewId: ReviewId;
  id: string;
}) {
  const d = useCommentStore((s) =>
    s.drafts[reviewId]?.find((x) => x.id === id),
  );
  const editing = useCommentStore((s) => s.composer?.draftId === id);
  const posting = useCommentStore((s) => !!s.posting[reviewId]);
  const edit = useCommentStore((s) => s.editDraft);
  const remove = useCommentStore((s) => s.removeDraft);
  const retry = useCommentStore((s) => s.retryDraft);
  const provider = useProvider(reviewId);
  const plan = usePlanFor(reviewId, provider, d ?? null);
  const suggestion = useComposerSuggestion({ reviewId, provider, plan });
  if (!d || editing) return null;
  return (
    <article
      className={`${styles.block} ${styles.draft}`}
      data-failed={!!d.error}
      aria-label={`Pending comment on ${d.path} ${rangeLabel(d.side, d)}`}
    >
      <div className={styles.head}>
        <Badge tone={d.error ? "warning" : "accent"}>
          {d.error ? "Not posted" : "Pending"}
        </Badge>
        <span className={styles.where}>
          {rangeLabel(d.side, d)}
          {d.outside ? " (outside the diff)" : ""}
        </span>
        <span className={styles.spacer} />
        {d.error && (
          <button
            type="button"
            className="ghost"
            disabled={posting}
            onClick={() => void retry(reviewId, id)}
          >
            {posting ? "Posting…" : "Retry"}
          </button>
        )}
        <button
          type="button"
          className="ghost"
          disabled={posting}
          onClick={() => edit(reviewId, id)}
        >
          Edit
        </button>
        <button
          type="button"
          className="ghost"
          disabled={posting}
          aria-label="Delete pending comment"
          onClick={() => remove(reviewId, id)}
        >
          Delete
        </button>
      </div>
      {d.error && (
        <p className={styles.error} role="alert">
          {d.error.message}
        </p>
      )}
      <div className={styles.body}>
        <Markdown renderFence={suggestion?.renderFence}>{d.body}</Markdown>
      </div>
    </article>
  );
}

export function ReplyComposer({
  reviewId,
  threadId,
  thread,
  provider,
  onResize,
}: {
  reviewId: ReviewId;
  threadId: ThreadId;
  /** The thread replied to: its lines take the reply's suggestion. */
  thread?: StoredThread;
  provider?: ProviderId;
  onResize?: () => void;
}) {
  const r = useCommentStore((s) => s.replies[threadId]);
  const anchorPath = thread?.anchor
    ? (thread.anchor.pathNew ?? thread.anchor.pathOld)
    : null;
  const file = useFileEntry(reviewId, anchorPath);
  const listHead = useCodeViewStore(
    (s) => s.byReview[reviewId]?.files?.headSha ?? null,
  );
  const recordHead = useReviewStore(
    (s) => s.reviews.find((x) => x.id === reviewId)?.headSha ?? null,
  );
  const plan =
    thread && provider
      ? replySuggestionPlan({
          provider,
          thread,
          headSha: listHead ?? recordHead,
          file,
        })
      : NO_PLAN;
  const suggestion = useComposerSuggestion({
    reviewId,
    provider: thread ? (provider ?? null) : null,
    plan,
    onResize,
  });
  const setBody = useCommentStore((s) => s.setReplyBody);
  const close = useCommentStore((s) => s.closeReply);
  const send = useCommentStore((s) => s.sendReply);
  const box = useRef<HTMLDivElement>(null);
  const seq = r?.revealSeq ?? 0;
  const seen = useRef<number | null>(null);
  // Scroll to the composer once when it opens (only on a pending request,
  // so a widget remount stays put) and again on each Reply click while it
  // is open. The scroll container is the page's diff column; the browser
  // walks up out of the CodeMirror widget by itself.
  useEffect(() => {
    const first = seen.current === null;
    const changed = !first && seen.current !== seq;
    seen.current = seq;
    if (!r || (first ? !takeFocus(FOCUS.reply(threadId)) : !changed)) return;
    const reduce =
      document.documentElement.dataset.reduceMotion === "true" ||
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    pauseScrollAnchor();
    box.current?.scrollIntoView?.({
      block: "nearest",
      behavior: reduce ? "auto" : "smooth",
    });
    box.current?.querySelector("textarea")?.focus({ preventScroll: true });
    // Keyed on the request counter and the composer's presence only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seq, !!r]);
  if (!r) return null;
  return (
    <div className={styles.reply} ref={box}>
      <CommentComposer
        label="Reply"
        placeholder="Reply (markdown)"
        body={r.body}
        onChange={(b) => setBody(threadId, b)}
        busy={r.busy}
        error={r.error}
        onCancel={() => close(threadId)}
        onResize={onResize}
        focusKey={FOCUS.reply(threadId)}
        suggestion={suggestion}
        primary={{
          label: "Reply",
          busyLabel: "Replying…",
          onClick: () => void send(reviewId, threadId),
        }}
      />
    </div>
  );
}
