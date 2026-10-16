/**
 * The detail island for the selected review: its header (round state, "Mark
 * round reviewed") over one review page (`ReviewView`: the Conversation, then
 * every changed file with its threads inline). Without a selection the
 * island shows the `WelcomeScreen`.
 */

import type { ReviewRecord } from "../ipc/types";
import { openExternal } from "../lib/links";
import {
  PROVIDER_LABEL,
  REASON_LABEL,
  STATUS_LABEL,
  reviewRef,
  untouchedLabel,
} from "../lib/reviews";
import { ago, fullTime } from "../lib/time";
import { useReviewStore } from "../state/reviewStore";
import { Badge, PlatformBadge } from "./Badge";
import { ExternalLinkIcon } from "./icons";
import { ReviewView } from "./ReviewView";
import { WelcomeScreen } from "./WelcomeScreen";
import styles from "./ReviewDetail.module.css";

export function ReviewDetail({ now }: { now: number }) {
  const review = useReviewStore((s) =>
    s.selectedId
      ? (s.reviews.find((r) => r.id === s.selectedId) ?? null)
      : null,
  );
  const refreshing = useReviewStore((s) => s.refreshing);
  const refresh = useReviewStore((s) => s.refresh);

  if (!review) {
    return (
      <section className={styles.island} aria-label="Review detail">
        <WelcomeScreen
          onRefresh={() => void refresh()}
          refreshing={refreshing}
        />
      </section>
    );
  }
  // Keyed so per-review UI state (collapsed sections) resets.
  return <Detail key={review.id} review={review} now={now} />;
}

function Detail({ review: r, now }: { review: ReviewRecord; now: number }) {
  return (
    <section className={styles.island} aria-label="Review detail">
      <ReviewHeader review={r} now={now} />
      <ReviewView review={r} now={now} />
    </section>
  );
}

function ReviewHeader({
  review: r,
  now,
}: {
  review: ReviewRecord;
  now: number;
}) {
  const marking = useReviewStore((s) => s.marking === r.id);
  const markReviewed = useReviewStore((s) => s.markReviewed);
  const canMark = r.status === "needsReview" || r.status === "needsReReview";
  const untouched = untouchedLabel(r.untouchedThreads);

  return (
    <header className={styles.header}>
      <div className={styles.topRow}>
        <PlatformBadge provider={r.provider} />
        <span className={`${styles.ref} mono`}>{reviewRef(r)}</span>
        {r.draft && <Badge>Draft</Badge>}
        <button
          type="button"
          className={`ghost ${styles.open}`}
          onClick={() => openExternal(r.url)}
          title={r.url}
        >
          Open in {PROVIDER_LABEL[r.provider]} <ExternalLinkIcon size={12} />
        </button>
      </div>
      <h1 className={styles.title}>{r.title}</h1>
      <div className={styles.meta}>
        <span>
          by <strong>{r.author}</strong>
        </span>
        {r.updatedAt && (
          <span title={fullTime(r.updatedAt)}>
            updated {ago(r.updatedAt, now)}
          </span>
        )}
        <Badge tone={r.status === "needsReReview" ? "accent" : "neutral"}>
          {STATUS_LABEL[r.status]}
        </Badge>
        <span>Round {r.round}</span>
        {r.state !== "open" && (
          <Badge tone={r.state === "merged" ? "accent" : "neutral"}>
            {r.state === "merged" ? "Merged" : "Closed"}
          </Badge>
        )}
      </div>
      {(r.roundReasons.length > 0 || untouched) && (
        <div className={styles.meta}>
          {r.roundReasons.map((reason) => (
            <Badge key={reason} tone="accent">
              {REASON_LABEL[reason]}
            </Badge>
          ))}
          {untouched && <span className={styles.untouched}>{untouched}</span>}
        </div>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className="primary"
          disabled={!canMark || marking}
          title={
            canMark
              ? "Remember the current head as reviewed"
              : "Nothing new since your last review"
          }
          onClick={() => void markReviewed(r.id)}
        >
          {marking ? "Marking…" : "Mark round reviewed"}
        </button>
        {r.lastReviewedAt && (
          <span
            className={styles.lastReviewed}
            title={fullTime(r.lastReviewedAt)}
          >
            Last marked reviewed {ago(r.lastReviewedAt, now)}
          </span>
        )}
      </div>
    </header>
  );
}
