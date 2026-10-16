/**
 * A thread as the review view shows it — under its line, in a file's
 * "Outdated threads" / "Threads on this file" section, or in the
 * Conversation: the user's own threads open and accent-edged, everyone
 * else's folded to one line (author, count, first words) until clicked.
 * Notes in order, rendered as markdown (`Markdown`), ` ```suggestion `
 * blocks as a diff (`SuggestedChange`). A thread with a code
 * reference has a "Then/now history" toggle that expands its then/now slot
 * (M3) in place. "Reply" opens a reply composer at the foot of the thread
 * (`ReplyComposer`; posted only on its Reply button or ⌘↩).
 *
 * Rendered inside CodeMirror block widgets (its own React root), so it takes
 * everything as props and reports size changes through `onResize`.
 */

import { useState } from "react";

import type { ProviderId, StoredThread } from "../ipc/types";
import { previewText } from "../lib/commentText";
import { isConversationThread } from "../lib/diffThreads";
import { parseSuggestionInfo } from "../lib/suggestions";
import { threadLocation } from "../lib/reviews";
import { age, fullTime } from "../lib/time";
import { useCommentStore } from "../state/commentStore";
import { Badge } from "./Badge";
import { ReplyComposer } from "./CommentBlocks";
import { CaretIcon } from "./icons";
import { Markdown, type RenderFence } from "./Markdown";
import { SuggestedChange } from "./SuggestedChange";
import { VerdictBadge } from "./VerdictBadge";
import styles from "./ThreadBlock.module.css";

function preview(t: StoredThread): string {
  const first = t.notes.find((n) => !n.system) ?? t.notes[0];
  return previewText(first?.body ?? "");
}

export function ThreadBlock({
  thread: t,
  provider,
  now,
  onResize,
}: {
  thread: StoredThread;
  provider: ProviderId;
  now: number;
  onResize?: () => void;
}) {
  const [open, setOpen] = useState(t.mine);
  const [history, setHistory] = useState(false);
  const replying = useCommentStore((s) => !!s.replies[t.threadId]);
  const openReply = useCommentStore((s) => s.openReply);
  const comments = t.notes.filter((n) => !n.system).length;
  const anchored = !isConversationThread(t);
  // After React commits the new height.
  const resized = () => requestAnimationFrame(() => onResize?.());
  const toggle = () => {
    setOpen((o) => !o);
    resized();
  };
  const renderFence: RenderFence = (info, code) => {
    const spec = parseSuggestionInfo(info);
    return spec ? (
      <SuggestedChange
        spec={spec}
        body={code}
        thread={t}
        provider={provider}
        onResize={onResize}
      />
    ) : null;
  };
  const reply = () => {
    openReply(t.threadId);
    setOpen(true);
    resized();
  };
  const toggleHistory = () => {
    setHistory((h) => !h);
    setOpen(true);
    resized();
  };

  return (
    <article
      id={`thread-${t.threadId}`}
      className={styles.thread}
      data-mine={t.mine}
      data-open={open}
      data-resolved={t.resolved}
      aria-label={`Thread by ${t.author}`}
    >
      <div className={styles.headRow}>
        <button
          type="button"
          className={styles.head}
          aria-expanded={open}
          onClick={toggle}
        >
          <CaretIcon size={12} className={styles.caret} data-open={open} />
          <span className={styles.author}>{t.author}</span>
          <span className={styles.count}>
            {comments} {comments === 1 ? "comment" : "comments"}
          </span>
          {t.resolved && <Badge tone="success">Resolved</Badge>}
          {!open && <span className={styles.preview}>{preview(t)}</span>}
        </button>
        <VerdictBadge
          reviewId={t.reviewId}
          threadId={t.threadId}
          provider={provider}
        />
        <button
          type="button"
          className={`ghost ${styles.history}`}
          aria-label={`Reply to ${t.author}'s thread`}
          disabled={replying && open}
          onClick={reply}
        >
          Reply
        </button>
        {anchored && (
          <button
            type="button"
            className={`ghost ${styles.history}`}
            aria-expanded={history}
            aria-controls={`then-now-${t.threadId}`}
            onClick={toggleHistory}
          >
            Then/now history
          </button>
        )}
      </div>
      {/* Then/now code view (M3): the code at the comment's commit and at the
          current head, expanded in place. */}
      {anchored && history && (
        <div
          id={`then-now-${t.threadId}`}
          className={styles.thenNow}
          data-slot="then-now"
        >
          Code then and now: coming soon
        </div>
      )}
      {open && (
        <ol className={styles.notes}>
          {t.notes.map((n) => (
            <li key={n.id} className={styles.note} data-system={n.system}>
              <div className={styles.noteMeta}>
                <span className={styles.noteAuthor}>{n.author}</span>
                <span className={styles.noteAge} title={fullTime(n.createdAt)}>
                  {age(n.createdAt, now)}
                </span>
              </div>
              <div className={styles.noteBody}>
                <Markdown renderFence={renderFence}>{n.body}</Markdown>
              </div>
            </li>
          ))}
        </ol>
      )}
      {open && replying && (
        <ReplyComposer
          reviewId={t.reviewId}
          threadId={t.threadId}
          onResize={onResize}
        />
      )}
    </article>
  );
}

/**
 * A thread listed at the top of its file or in the Conversation's "On other
 * files": where it sits, outdated (badge, the commit, the platform's
 * snippet) or current without a usable line, then the thread itself.
 */
export function ListedThread({
  thread: t,
  provider,
  now,
}: {
  thread: StoredThread;
  provider: ProviderId;
  now: number;
}) {
  const a = t.anchor;
  const sha = a?.headSha ? a.headSha.slice(0, 8) : null;
  return (
    <div className={styles.listed}>
      <div className={styles.listedHead}>
        <span className={`${styles.location} mono`}>
          {threadLocation(t, provider)}
        </span>
        {t.outdated && (
          <Badge
            tone="warning"
            title="The platform considers this thread outdated"
          >
            Outdated
          </Badge>
        )}
        {t.outdated && sha && (
          <span className={styles.commit} title={a?.headSha}>
            on {sha}
          </span>
        )}
      </div>
      {t.outdated && a?.diffHunk && (
        <pre className={`${styles.hunk} mono`}>
          {a.diffHunk.split("\n").map((line, i) => (
            <span
              key={i}
              className={styles.hunkLine}
              data-kind={
                line.startsWith("@@")
                  ? "meta"
                  : line.startsWith("+")
                    ? "add"
                    : line.startsWith("-")
                      ? "del"
                      : "ctx"
              }
            >
              {line || " "}
            </span>
          ))}
        </pre>
      )}
      <ThreadBlock thread={t} provider={provider} now={now} />
    </div>
  );
}
