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

/**
 * The open (unresolved) thread count of a file card; the accent when some
 * are the user's. Nothing once every thread is resolved.
 */
export function ThreadCount({ open, mine }: { open: number; mine: number }) {
  if (open === 0) return null;
  const label =
    mine > 0
      ? `${open} open ${open === 1 ? "thread" : "threads"}, ${mine} yours`
      : `${open} open ${open === 1 ? "thread" : "threads"}`;
  return (
    <span
      className={styles.threads}
      data-mine={mine > 0}
      title={label}
      aria-label={label}
    >
      {open}
    </span>
  );
}

/** A file's open threads of the user's, in the tree; nothing when none. */
export function MyOpenThreads({ count }: { count: number }) {
  if (count === 0) return null;
  const label = `${count} open ${count === 1 ? "thread" : "threads"} of yours`;
  return (
    <span
      className={styles.threads}
      data-mine="true"
      title={label}
      aria-label={label}
    >
      {count}
    </span>
  );
}

/**
 * "2 open": the unresolved threads (resolvable, not resolved) of the
 * Conversation; nothing when there are none. `label` overrides the tooltip.
 */
export function OpenCount({
  count,
  label: given,
}: {
  count: number;
  label?: string;
}) {
  if (count === 0) return null;
  const label =
    given ?? `${count} unresolved ${count === 1 ? "thread" : "threads"}`;
  return (
    <span className={styles.open} title={label}>
      {count} open
    </span>
  );
}
