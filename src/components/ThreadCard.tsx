/**
 * One thread: where it sits, its state (outdated is a badge, never a reason
 * to hide anything), a slot for the then/now code view, and its notes in
 * order.
 */

import type { ProviderId, StoredThread } from "../ipc/types";
import { threadLocation } from "../lib/reviews";
import { age, fullTime } from "../lib/time";
import { Badge } from "./Badge";
import styles from "./ThreadCard.module.css";

export function ThreadCard({
  thread: t,
  provider,
  now,
}: {
  thread: StoredThread;
  provider: ProviderId;
  now: number;
}) {
  const location = threadLocation(t, provider);
  return (
    <article
      className={styles.card}
      data-mine={t.mine}
      data-resolved={t.resolved}
      aria-label={`Thread on ${location}`}
    >
      <header className={styles.header}>
        <span className={`${styles.location} mono`} title={location}>
          {location}
        </span>
        {t.outdated && (
          <Badge
            tone="warning"
            title="The platform considers this thread outdated"
          >
            Outdated
          </Badge>
        )}
        {t.resolved ? (
          <Badge tone="success">Resolved</Badge>
        ) : (
          <Badge>Open</Badge>
        )}
      </header>

      {/* Then/now code view (M3): the code at the comment's commit and at the
          current head. The slot keeps the card's layout stable until then. */}
      {t.anchor && (
        <div className={styles.thenNow} data-slot="then-now">
          Code then and now: coming soon
        </div>
      )}

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
    </article>
  );
}
