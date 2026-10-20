/** A file's change as one letter (A/M/D/R), coloured by kind. */

import type { FileStatus } from "../ipc/types";
import styles from "./FileStatusMark.module.css";

const LETTER: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  removed: "D",
  renamed: "R",
};

const STATUS_TITLE: Record<FileStatus, string> = {
  added: "Added",
  modified: "Modified",
  removed: "Removed",
  renamed: "Renamed",
};

export function FileStatusMark({ status }: { status: FileStatus }) {
  return (
    <span
      className={styles.mark}
      data-status={status}
      title={STATUS_TITLE[status]}
      aria-label={STATUS_TITLE[status]}
    >
      {LETTER[status]}
    </span>
  );
}

/** `+12 −3`, either part left out when zero. */
export function LineStats({
  additions,
  deletions,
}: {
  additions: number;
  deletions: number;
}) {
  if (additions === 0 && deletions === 0) return null;
  return (
    <span
      className={styles.stats}
      aria-label={`${additions} added, ${deletions} removed`}
    >
      {additions > 0 && <span className={styles.add}>+{additions}</span>}
      {deletions > 0 && <span className={styles.del}>−{deletions}</span>}
    </span>
  );
}

/** The thread count of a file; the accent when some are the user's. */
export function ThreadCount({ total, mine }: { total: number; mine: number }) {
  if (total === 0) return null;
  const label =
    mine > 0
      ? `${total} ${total === 1 ? "thread" : "threads"}, ${mine} yours`
      : `${total} ${total === 1 ? "thread" : "threads"}`;
  return (
    <span
      className={styles.threads}
      data-mine={mine > 0}
      title={label}
      aria-label={label}
    >
      {total}
    </span>
  );
}

/**
 * "2 open": the unresolved threads (resolvable, not resolved) of the
 * Conversation; nothing when there are none.
 */
export function OpenCount({ count }: { count: number }) {
  if (count === 0) return null;
  const label = `${count} unresolved ${count === 1 ? "thread" : "threads"}`;
  return (
    <span className={styles.open} title={label}>
      {count} open
    </span>
  );
}
