/**
 * The detail pane for the selected review: header, round state, the
 * "Mark round reviewed" action and the threads (the user's first).
 */

import { useEffect, useState } from "react";

import type { ReviewRecord } from "../ipc/types";
import { openExternal } from "../lib/links";
import {
  PROVIDER_LABEL,
  REASON_LABEL,
  STATUS_LABEL,
  reviewRef,
  splitThreads,
  untouchedLabel,
} from "../lib/reviews";
import { ago, fullTime } from "../lib/time";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { Badge, PlatformBadge } from "./Badge";
import { ChevronIcon, ExternalIcon } from "./icons";
import { ThreadCard } from "./ThreadCard";
import styles from "./ReviewDetail.module.css";

export function ReviewDetail({ now }: { now: number }) {
  const review = useReviewStore((s) =>
    s.selectedId
      ? (s.reviews.find((r) => r.id === s.selectedId) ?? null)
      : null,
  );

  if (!review) {
    return (
      <section className={styles.placeholder} aria-label="Review detail">
        <p>Select a review to see its threads.</p>
        <p className={styles.keys}>
          <kbd>↑</kbd> <kbd>↓</kbd> move · <kbd>Enter</kbd> open · <kbd>r</kbd>{" "}
          refresh · <kbd>1</kbd>–<kbd>4</kbd> tabs
        </p>
      </section>
    );
  }
  // Keyed so the collapsed state resets per review.
  return <Detail key={review.id} review={review} now={now} />;
}

function Detail({ review: r, now }: { review: ReviewRecord; now: number }) {
  const marking = useReviewStore((s) => s.marking === r.id);
  const markReviewed = useReviewStore((s) => s.markReviewed);
  const entry = useThreadStore((s) => s.byReview[r.id]);
  const loadThreads = useThreadStore((s) => s.load);
  const [othersOpen, setOthersOpen] = useState(false);

  useEffect(() => {
    void loadThreads(r.id);
  }, [r.id, loadThreads]);

  const { mine, others } = splitThreads(entry?.threads ?? []);
  const canMark = r.status === "needsReview" || r.status === "needsReReview";
  const untouched = untouchedLabel(r.untouchedThreads);

  return (
    <section className={styles.pane} aria-label="Review detail">
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
            Open in {PROVIDER_LABEL[r.provider]} <ExternalIcon size={12} />
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

      <div className={styles.threads}>
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
              <p className={styles.muted}>
                You have no threads on this review.
              </p>
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
                  <ChevronIcon
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
    </section>
  );
}
