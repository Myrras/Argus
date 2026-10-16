/**
 * The detail island's Threads tab: the user's threads first (outdated ones
 * included, never hidden), the others behind a disclosure.
 */

import { useEffect, useState } from "react";

import type { ReviewRecord } from "../ipc/types";
import { splitThreads } from "../lib/reviews";
import { useThreadStore } from "../state/threadStore";
import { CaretIcon } from "./icons";
import { ThreadCard } from "./ThreadCard";
import styles from "./ReviewDetail.module.css";

export function ThreadsTab({
  review: r,
  now,
}: {
  review: ReviewRecord;
  now: number;
}) {
  const entry = useThreadStore((s) => s.byReview[r.id]);
  const loadThreads = useThreadStore((s) => s.load);
  const [othersOpen, setOthersOpen] = useState(false);

  useEffect(() => {
    void loadThreads(r.id);
  }, [r.id, loadThreads]);

  const { mine, others } = splitThreads(entry?.threads ?? []);

  return (
    <div className={styles.body}>
      {entry?.error && (
        <p className={styles.error} role="alert">
          {entry.error.message}
        </p>
      )}
      {!entry || (entry.loading && entry.threads.length === 0) ? (
        <p className={styles.muted}>Loading threads…</p>
      ) : (
        <>
          <h2 className={styles.sectionTitle}>
            Your threads{" "}
            <span className={styles.sectionCount}>{mine.length}</span>
          </h2>
          {mine.length === 0 ? (
            <p className={styles.muted}>You have no threads on this review.</p>
          ) : (
            <div className={styles.stack}>
              {mine.map((t) => (
                <ThreadCard
                  key={t.threadId}
                  thread={t}
                  provider={r.provider}
                  now={now}
                />
              ))}
            </div>
          )}

          {others.length > 0 && (
            <>
              <button
                type="button"
                className={`ghost ${styles.disclosure}`}
                aria-expanded={othersOpen}
                aria-controls={`others-${r.id}`}
                onClick={() => setOthersOpen((o) => !o)}
              >
                <CaretIcon
                  size={12}
                  className={styles.chevron}
                  data-open={othersOpen}
                />
                Other threads
                <span className={styles.sectionCount}>{others.length}</span>
              </button>
              {othersOpen && (
                <div id={`others-${r.id}`} className={styles.stack}>
                  {others.map((t) => (
                    <ThreadCard
                      key={t.threadId}
                      thread={t}
                      provider={r.provider}
                      now={now}
                    />
                  ))}
                </div>
              )}
            </>
          )}
          {mine.length === 0 && others.length === 0 && (
            <p className={styles.muted}>No threads on this review yet.</p>
          )}
        </>
      )}
    </div>
  );
}
