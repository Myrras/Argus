/**
 * A thread's history (M3) inside its card (`ThreadBlock`):
 *
 * - `ThenNow`: what happened to the commented code (the status line per
 *   `LineStatus`) over two columns, Then (the comment's commit, path:line)
 *   and Now (path:line at head), aligned row by row from the backend's diff
 *   rows, changed lines in the diff colours, the commented lines edged.
 *   Side by side when the card is wide, stacked when narrow. The code is
 *   highlighted by CodeMirror (`ThenNowEditor`, lazy; plain monospaced rows
 *   until it loads). An unlocated thread never shows a now line.
 * - `Timeline`: notes, the commits that touched the lines and resolve
 *   events on one line of time, oldest first, so "replied before touching
 *   it" reads at a glance. Notes render as markdown; long ones fold.
 *
 * Both take the thread's `useThreadHistory` state.
 */

import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import type {
  StoredThread,
  ThreadHistory as History,
  TimelineEntry,
} from "../ipc/types";
import {
  changeText,
  entryKey,
  isLongBody,
  nowLocation,
  shortSha,
  statusLine,
  thenLocation,
  thenNowColumns,
  type ColumnLine,
} from "../lib/threadHistory";
import { age, fullTime } from "../lib/time";
import { noteUnit } from "../lib/findInReview";
import { useFindCurrentIn } from "../state/findStore";
import type { ThreadHistoryState } from "../lib/useThreadHistory";
import {
  CommitIcon,
  FileGoneIcon,
  LinesRemovedIcon,
  RenamedIcon,
  ResolvedIcon,
  ThreadsIcon,
  UnresolvedIcon,
} from "./icons";
import { Markdown, type RenderFence } from "./Markdown";
import styles from "./ThreadHistory.module.css";

const ThenNowEditor = lazy(() =>
  import("./ThenNowEditor").then((m) => ({ default: m.ThenNowEditor })),
);

// --- Then / now ------------------------------------------------------------

function PlainColumn({ lines }: { lines: readonly ColumnLine[] }) {
  return (
    <div className={`${styles.plain} mono`}>
      {lines.map((l, i) => (
        <div
          key={i}
          className={styles.plainLine}
          data-kind={l.kind}
          data-anchored={l.anchored}
        >
          <span className={styles.num}>{l.line ?? ""}</span>
          <span className={styles.text}>{l.text || " "}</span>
        </div>
      ))}
    </div>
  );
}

function CodeColumn({
  side,
  caption,
  sha,
  lines,
  path,
  empty,
  onResize,
}: {
  side: "then" | "now";
  caption: string | null;
  sha: string | null;
  lines: readonly ColumnLine[] | null;
  path: string;
  /** Shown instead of code when there is none. */
  empty: string;
  onResize?: () => void;
}) {
  return (
    <div
      className={styles.column}
      data-side={side}
      aria-label={side === "then" ? "Code then" : "Code now"}
    >
      <div className={styles.columnHead}>
        <span className={styles.side}>{side === "then" ? "Then" : "Now"}</span>
        {sha && (
          <span className={`${styles.sha} mono`} title={sha}>
            {shortSha(sha)}
          </span>
        )}
        {caption && (
          <span className={`${styles.where} mono`} title={caption}>
            {caption}
          </span>
        )}
      </div>
      {lines ? (
        <div className={styles.code}>
          <Suspense fallback={<PlainColumn lines={lines} />}>
            <ThenNowEditor lines={lines} path={path} onReady={onResize} />
          </Suspense>
        </div>
      ) : (
        <p className={styles.empty}>{empty}</p>
      )}
    </div>
  );
}

function nowEmpty(h: History): string {
  switch (h.status.kind) {
    case "deleted":
      return "The commented lines are gone.";
    case "fileGone":
      return "The file is gone.";
    case "unlocated":
      return "Not located.";
    default:
      return "The code at head is unavailable.";
  }
}

/** The then/now block of a thread on code. */
export function ThenNow({
  thread: t,
  state,
  onResize,
}: {
  thread: StoredThread;
  state: ThreadHistoryState;
  onResize?: () => void;
}) {
  const { history: h, loading, error, retry } = state;
  const columns = useMemo(() => (h ? thenNowColumns(h) : null), [h]);
  if (!h) {
    return error ? (
      <p className={styles.notice} role="alert">
        Could not load the history: {error.message}{" "}
        <button type="button" className="ghost" onClick={retry}>
          Retry
        </button>
      </p>
    ) : (
      <p className={styles.notice}>
        {loading ? "Loading the code then and now…" : "No history yet."}
      </p>
    );
  }
  const status = statusLine(h, t.startedByMe);
  if (!status || !columns) return null;
  const thenPath = h.thenSnapshot?.path ?? "";
  const nowPath = h.nowSnapshot?.path ?? thenPath;
  return (
    <section className={styles.thenNow} aria-label="Code then and now">
      <p className={styles.status} data-tone={status.tone} role="status">
        {status.text}
      </p>
      {(columns.then || columns.now) && (
        <div className={styles.columns}>
          <CodeColumn
            side="then"
            caption={thenLocation(h)}
            sha={h.thenSnapshot?.sha ?? null}
            lines={columns.then}
            path={thenPath}
            empty="The code at the comment's commit is unavailable."
            onResize={onResize}
          />
          <CodeColumn
            side="now"
            caption={nowLocation(h)}
            sha={null}
            lines={columns.now}
            path={nowPath}
            empty={nowEmpty(h)}
            onResize={onResize}
          />
        </div>
      )}
      {h.thenSnapshot?.source === "diffHunk" && (
        <p className={styles.footnote}>
          Then is the platform's diff excerpt: the file at that commit could not
          be read.
        </p>
      )}
    </section>
  );
}

// --- Timeline --------------------------------------------------------------

function entryIcon(e: TimelineEntry): ReactNode {
  switch (e.kind) {
    case "note":
      return <ThreadsIcon size={12} />;
    case "resolved":
      return <ResolvedIcon size={12} />;
    case "unresolved":
      return <UnresolvedIcon size={12} />;
    case "commit":
      switch (e.change.kind) {
        case "renamed":
          return <RenamedIcon size={12} />;
        case "deleted":
          return <LinesRemovedIcon size={12} />;
        case "fileGone":
          return <FileGoneIcon size={12} />;
        default:
          return <CommitIcon size={12} />;
      }
  }
}

function When({ at, now }: { at: string | null; now: number }) {
  if (!at) return null;
  return (
    <span className={styles.age} title={fullTime(at)}>
      {age(at, now)}
    </span>
  );
}

function NoteBody({
  body,
  renderFence,
  findUnit,
}: {
  body: string;
  renderFence?: RenderFence;
  findUnit?: string;
}) {
  const long = isLongBody(body);
  const [more, setMore] = useState(false);
  // The find bar went to a match in here: show all of it.
  const found = useFindCurrentIn(findUnit);
  useEffect(() => {
    if (found && long) setMore(true);
  }, [found, long]);
  return (
    <>
      <div className={styles.body} data-folded={long && !more}>
        <Markdown renderFence={renderFence} findUnit={findUnit}>
          {body}
        </Markdown>
      </div>
      {long && (
        <button
          type="button"
          className={`ghost ${styles.more}`}
          aria-expanded={more}
          onClick={() => setMore((m) => !m)}
        >
          {more ? "Show less" : "Show more"}
        </button>
      )}
    </>
  );
}

/** The thread's events, oldest first, on one line of time. */
export function Timeline({
  entries,
  now,
  renderFence,
  threadId,
}: {
  entries: readonly TimelineEntry[];
  now: number;
  renderFence?: RenderFence;
  /** The thread's id: its notes' find units (`findInReview.noteUnit`). */
  threadId?: string;
}) {
  return (
    <ol className={styles.timeline} aria-label="Timeline">
      {entries.map((e, i) => (
        <li
          key={entryKey(e, i)}
          className={styles.entry}
          data-kind={e.kind}
          data-system={e.kind === "note" ? e.system : undefined}
        >
          <span className={styles.dot}>{entryIcon(e)}</span>
          <div className={styles.event}>
            {e.kind === "note" ? (
              <>
                <div className={styles.meta}>
                  <span className={styles.author}>{e.author}</span>
                  <When at={e.at} now={now} />
                </div>
                <div className={styles.noteBody}>
                  <NoteBody
                    body={e.body}
                    renderFence={renderFence}
                    findUnit={
                      threadId ? noteUnit(threadId, e.noteId) : undefined
                    }
                  />
                </div>
              </>
            ) : e.kind === "commit" ? (
              <div className={styles.meta}>
                <span className={`${styles.commitSha} mono`} title={e.sha}>
                  {shortSha(e.sha)}
                </span>
                <span className={styles.commitTitle}>{e.title}</span>
                <span className={styles.commitInfo}>
                  {e.author} {changeText(e.change)}
                </span>
                <When at={e.at} now={now} />
              </div>
            ) : (
              <div className={styles.meta}>
                <span className={styles.stateText}>
                  {e.kind === "resolved" ? "Resolved" : "Unresolved"} by {e.by}
                </span>
                <When at={e.at} now={now} />
              </div>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
