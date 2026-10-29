/**
 * A thread as the review view shows it — under its line, in a file's
 * "Outdated threads" / "Threads on this file" section, or in the
 * Conversation: the user's own threads open and accent-edged, everyone
 * else's folded to one line (author, count, first words) until clicked.
 * Notes in order, rendered as markdown (`Markdown`), ` ```suggestion `
 * blocks as a diff (`SuggestedChange`). A thread with a code
 * reference has a "Then/now history" toggle for its then/now slot (M3,
 * `ThenNow`), open by default on the user's own threads; the state lives in
 * `historyStore`, as does the thread last clicked (the `toggleThenNow` key).
 * Once its history is in, the notes become the thread's `Timeline` (notes,
 * commits and resolve events; the notes always the thread's own, so a
 * reply shows with the threads' re-read, not the slower history's:
 * `threadTimeline`); until then, or when it fails, the plain notes. "Reply" opens a reply composer at the foot of the thread
 * (`ReplyComposer`; posted only on its Reply button or ⌘↩). "Resolve" /
 * "Unresolve" (only on a thread its platform can resolve) writes that to
 * the platform at once (`threadStore.setResolved`; reversible, so no
 * confirmation); the badge follows the re-read. A resolved thread reads
 * as settled: a check on its badge, a muted head and edge, and it folds
 * (`historyStore.threadOpen`: folded by default, and once a resolve lands;
 * reopened, it unfolds). A click, Reply, then/now or `h` still opens it.
 * The keyboard (M6, `focusStore`): the focused thread has a ring and the
 * DOM focus; Enter (or `t`) opens or folds it, `r` starts a reply, `s`
 * resolves or reopens it. The find bar's matches are marked in its author
 * and notes (`FindText`, `Markdown`'s `findUnit`).
 *
 * Rendered inside CodeMirror block widgets (its own React root), so it takes
 * everything as props and reports size changes through `onResize`.
 */

import { useEffect, useRef } from "react";

import type { ProviderId, StoredThread } from "../ipc/types";
import { previewText } from "../lib/commentText";
import { isConversationThread, outsideDiffLabel } from "../lib/diffThreads";
import { originalSource, parseSuggestionInfo } from "../lib/suggestions";
import { threadTimeline } from "../lib/threadHistory";
import { threadLocation } from "../lib/reviews";
import { age, fullTime } from "../lib/time";
import { useCommentStore } from "../state/commentStore";
import {
  thenNowOpen,
  threadOpen,
  useHistoryStore,
} from "../state/historyStore";
import { useNow } from "../lib/useNow";
import { useThreadHistory } from "../lib/useThreadHistory";
import { threadDescription } from "../lib/focusModel";
import { useThreadFocus } from "../state/focusStore";
import { useThreadStore } from "../state/threadStore";
import { noteUnit, threadUnit } from "../lib/findInReview";
import { useKeymap } from "../lib/useKeymap";
import { withHint } from "../lib/keymap";
import { Badge } from "./Badge";
import { ReplyComposer } from "./CommentBlocks";
import { CaretIcon, ResolvedIcon } from "./icons";
import { FindText, Markdown, type RenderFence } from "./Markdown";
import { SuggestedChange } from "./SuggestedChange";
import { ThenNow, Timeline } from "./ThreadHistory";
import { VerdictBadge } from "./VerdictBadge";
import styles from "./ThreadBlock.module.css";

function preview(t: StoredThread): string {
  const first = t.notes.find((n) => !n.system) ?? t.notes[0];
  return previewText(first?.body ?? "");
}

export function ThreadBlock({
  thread: t,
  provider,
  now: fixedNow,
  onResize,
}: {
  thread: StoredThread;
  provider: ProviderId;
  /** The time ages count from; the window's clock (`useNow`) by default. */
  now?: number;
  onResize?: () => void;
}) {
  const clock = useNow();
  const now = fixedNow ?? clock;
  // In the store, not local state: the card may be rebuilt with its widget.
  const open = useHistoryStore((s) => threadOpen(s.open, t));
  const storeOpen = useHistoryStore((s) => s.setOpen);
  const setOpen = (o: boolean) => storeOpen(t.threadId, o);
  const history = useHistoryStore(
    (s) => !isConversationThread(t) && thenNowOpen(s.expanded, t),
  );
  const setHistory = useHistoryStore((s) => s.setExpanded);
  const focus = useHistoryStore((s) => s.focus);
  const past = useThreadHistory(t);
  const replying = useCommentStore((s) => !!s.replies[t.threadId]);
  const resolving = useThreadStore((s) => !!s.resolving[t.threadId]);
  const setResolved = useThreadStore((s) => s.setResolved);
  const keymap = useKeymap();
  const openReply = useCommentStore((s) => s.openReply);
  const comments = t.notes.filter((n) => !n.system).length;
  const anchored = !isConversationThread(t);
  // After React commits the new height.
  const resized = () => requestAnimationFrame(() => onResize?.());
  const toggle = () => {
    setOpen(!open);
    resized();
  };
  const renderFence: RenderFence = (info, code) => {
    const spec = parseSuggestionInfo(info);
    return spec ? (
      <SuggestedChange
        spec={spec}
        body={code}
        reviewId={t.reviewId}
        anchor={t.anchor}
        source={t.anchor ? originalSource(t.anchor) : null}
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
    setHistory(t.threadId, !history);
    setOpen(true);
    resized();
  };
  const { focused, onKeyDown } = useThreadFocus(t.threadId, { toggle, reply });
  // Opened from the key too; the height moves when the history lands.
  const resizeRef = useRef(onResize);
  resizeRef.current = onResize;
  // Only when it turns on (the `h` key on a folded thread): a card
  // rebuilt with its widget must not unfold a thread that was folded.
  const historyWas = useRef(history);
  useEffect(() => {
    if (history && !historyWas.current) storeOpen(t.threadId, true);
    historyWas.current = history;
  }, [history, storeOpen, t.threadId]);
  useEffect(() => {
    requestAnimationFrame(() => resizeRef.current?.());
  }, [history, past.history, past.loading, past.error]);

  const outsideLabel = outsideDiffLabel(t);
  return (
    <article
      id={`thread-${t.threadId}`}
      className={styles.thread}
      data-mine={t.mine}
      data-open={open}
      data-resolved={t.resolved}
      data-focused={focused}
      tabIndex={-1}
      aria-label={`Thread by ${t.author}`}
      aria-describedby={`thread-desc-${t.threadId}`}
      onKeyDown={onKeyDown}
      onPointerDown={() =>
        focus({ threadId: t.threadId, reviewId: t.reviewId, mine: t.mine })
      }
    >
      <span id={`thread-desc-${t.threadId}`} className="visually-hidden">
        {threadDescription(t)}
      </span>
      <div className={styles.headRow}>
        <button
          type="button"
          className={styles.head}
          aria-expanded={open}
          onClick={toggle}
        >
          <CaretIcon size={12} className={styles.caret} data-open={open} />
          <span className={styles.author}>
            <FindText text={t.author} unit={threadUnit(t.threadId)} />
          </span>
          <span className={styles.count}>
            {comments} {comments === 1 ? "comment" : "comments"}
          </span>
          {outsideLabel && (
            <span className={styles.outside}>{outsideLabel}</span>
          )}
          {t.resolved && (
            <Badge tone="success">
              <ResolvedIcon size={11} />
              Resolved
            </Badge>
          )}
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
          onClick={reply}
        >
          Reply
        </button>
        {t.resolvable && (
          <button
            type="button"
            className={`ghost ${styles.history}`}
            aria-label={`${t.resolved ? "Unresolve" : "Resolve"} ${t.author}'s thread`}
            title={withHint(
              t.resolved
                ? "Reopen the thread on the platform"
                : "Resolve the thread on the platform",
              keymap,
              "resolveThread",
            )}
            aria-busy={resolving}
            disabled={resolving}
            onClick={() => void setResolved(t, !t.resolved)}
          >
            {resolving
              ? t.resolved
                ? "Reopening…"
                : "Resolving…"
              : t.resolved
                ? "Unresolve"
                : "Resolve"}
          </button>
        )}
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
      {anchored && open && history && (
        <div
          id={`then-now-${t.threadId}`}
          className={styles.thenNow}
          data-slot="then-now"
        >
          <ThenNow thread={t} state={past} onResize={onResize} />
        </div>
      )}
      {open && past.history && (
        <Timeline
          entries={threadTimeline(t, past.history)}
          now={now}
          renderFence={renderFence}
          threadId={t.threadId}
        />
      )}
      {open && !past.history && (
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
                <Markdown
                  renderFence={renderFence}
                  findUnit={noteUnit(t.threadId, n.id)}
                >
                  {n.body}
                </Markdown>
              </div>
            </li>
          ))}
        </ol>
      )}
      {open && replying && (
        <ReplyComposer
          reviewId={t.reviewId}
          threadId={t.threadId}
          thread={t}
          provider={provider}
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
  /** The time ages count from; the window's clock by default. */
  now?: number;
}) {
  const a = t.anchor;
  const sha = a?.headSha ? a.headSha.slice(0, 8) : null;
  // The then/now shows the code the thread was made on; the platform's
  // excerpt only until it does.
  const thenShown = useHistoryStore(
    (s) =>
      thenNowOpen(s.expanded, t) &&
      !!s.byThread[t.threadId]?.history?.thenSnapshot,
  );
  return (
    <div className={styles.listed}>
      <div className={styles.listedHead}>
        <span className={`${styles.location} mono`}>
          {threadLocation(t, provider)}
        </span>
        {outsideDiffLabel(t) && (
          <span className={styles.outside}>{outsideDiffLabel(t)}</span>
        )}
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
      {t.outdated && a?.diffHunk && !thenShown && (
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
