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
 */

import type { ReviewId, ThreadId } from "../ipc/types";
import { rangeLabel } from "../lib/commentRanges";
import { FOCUS } from "../lib/focusRequests";
import { useCommentStore } from "../state/commentStore";
import { Badge } from "./Badge";
import { CommentComposer } from "./CommentComposer";
import { Markdown } from "./Markdown";
import styles from "./CommentBlocks.module.css";

export function InlineComposer({ onResize }: { onResize?: () => void }) {
  const c = useCommentStore((s) => s.composer);
  const setBody = useCommentStore((s) => s.setComposerBody);
  const close = useCommentStore((s) => s.closeComposer);
  const save = useCommentStore((s) => s.saveComposer);
  const commentNow = useCommentStore((s) => s.commentNow);
  if (!c) return null;
  const where = rangeLabel(c.side, c);
  return (
    <div className={styles.block}>
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
        primary={
          c.draftId
            ? { label: "Save", onClick: save }
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
        <span className={styles.where}>{rangeLabel(d.side, d)}</span>
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
        <Markdown>{d.body}</Markdown>
      </div>
    </article>
  );
}

export function ReplyComposer({
  reviewId,
  threadId,
  onResize,
}: {
  reviewId: ReviewId;
  threadId: ThreadId;
  onResize?: () => void;
}) {
  const r = useCommentStore((s) => s.replies[threadId]);
  const setBody = useCommentStore((s) => s.setReplyBody);
  const close = useCommentStore((s) => s.closeReply);
  const send = useCommentStore((s) => s.sendReply);
  if (!r) return null;
  return (
    <div className={styles.reply}>
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
        primary={{
          label: "Reply",
          busyLabel: "Replying…",
          onClick: () => void send(reviewId, threadId),
        }}
      />
    </div>
  );
}
