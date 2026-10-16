/**
 * A thread as the code view shows it, under its line or in a file's
 * "Outdated threads" section: the user's own threads open and accent-edged,
 * everyone else's folded to one line (author, count, first words) until
 * clicked. Notes in order, as on the thread card.
 *
 * Rendered inside CodeMirror block widgets (its own React root), so it takes
 * everything as props and reports size changes through `onResize`.
 */

import { useState } from "react";

import type { ProviderId, StoredThread } from "../ipc/types";
import { threadLocation } from "../lib/reviews";
import { age, fullTime } from "../lib/time";
import { showInThreadsTab } from "../state/codeViewStore";
import { Badge } from "./Badge";
import { CaretIcon } from "./icons";
import styles from "./ThreadBlock.module.css";

function preview(t: StoredThread): string {
  const first = t.notes.find((n) => !n.system) ?? t.notes[0];
  const text = (first?.body ?? "").replace(/\s+/g, " ").trim();
  return text.length > 90 ? `${text.slice(0, 89)}…` : text;
}

export function ThreadBlock({
  thread: t,
  now,
  onResize,
}: {
  thread: StoredThread;
  now: number;
  onResize?: () => void;
}) {
  const [open, setOpen] = useState(t.mine);
  const comments = t.notes.filter((n) => !n.system).length;
  const toggle = () => {
    setOpen((o) => !o);
    // After React commits the new height.
    requestAnimationFrame(() => onResize?.());
  };

  return (
    <article
      className={styles.thread}
      data-mine={t.mine}
      data-open={open}
      aria-label={`Thread by ${t.author}`}
    >
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
              <div className={styles.noteBody}>{n.body}</div>
            </li>
          ))}
        </ol>
      )}
    </article>
  );
}

/**
 * A thread listed at the top of its file: outdated (badge, the platform's
 * snippet, a link to its then/now history) or current without a usable line.
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
        <button
          type="button"
          className={`ghost ${styles.history}`}
          onClick={() => showInThreadsTab(t.threadId)}
        >
          Then/now history
        </button>
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
      <ThreadBlock thread={t} now={now} />
    </div>
  );
}
