/**
 * "Look here yourself": what the AI could not verify on its own, for the
 * current round with a checkbox each (stored by Argus, never on the
 * platform); earlier rounds folded under a toggle. An item tied to a
 * comment of the review says so ("related to bob's comment", or "repeats
 * …" once shown anyway), as a finding does.
 */

import { useState } from "react";

import type { ChecklistItem, ReviewId, StoredThread } from "../ipc/types";
import {
  RELATED_HINT,
  SHOWN_ANYWAY_HINT,
  relatedLabel,
  repeatTarget,
  splitChecklist,
} from "../lib/findings";
import { useAiStore } from "../state/aiStore";
import { useThreadStore } from "../state/threadStore";
import { PointLink } from "./FindingsList";
import { CaretIcon } from "./icons";
import styles from "./Checklist.module.css";

const NO_ITEMS: ChecklistItem[] = [];
const NO_THREADS: StoredThread[] = [];

export function Checklist({ reviewId }: { reviewId: ReviewId }) {
  const items = useAiStore((s) =>
    s.runs[reviewId]?.enabled
      ? (s.byReview[reviewId]?.checklist ?? NO_ITEMS)
      : NO_ITEMS,
  );
  const [showEarlier, setShowEarlier] = useState(false);
  const { round, current, earlier } = splitChecklist(items);
  const open = current.filter((i) => !i.checked).length;

  return (
    <section className={styles.section} aria-label="Look here yourself">
      <h3 className={styles.title}>
        Look here yourself{" "}
        {current.length > 0 && (
          <span className={styles.count}>
            {open} of {current.length} open
          </span>
        )}
      </h3>
      {current.length === 0 ? (
        <p className={styles.empty}>Nothing to check by hand.</p>
      ) : (
        <ItemList reviewId={reviewId} items={current} />
      )}
      {earlier.length > 0 && (
        <>
          <button
            type="button"
            className={`ghost ${styles.earlierToggle}`}
            aria-expanded={showEarlier}
            onClick={() => setShowEarlier((v) => !v)}
          >
            <CaretIcon
              size={12}
              className={styles.caret}
              data-open={showEarlier}
            />
            Earlier rounds ({earlier.length})
          </button>
          {showEarlier &&
            [...new Set(earlier.map((i) => i.round))].map((r) => (
              <div key={r} className={styles.earlier}>
                <h4 className={styles.roundTitle}>Round {r}</h4>
                <ItemList
                  reviewId={reviewId}
                  items={earlier.filter((i) => i.round === r)}
                />
              </div>
            ))}
        </>
      )}
      {round !== null && current.length > 0 && (
        <p className={styles.note}>Round {round}. Kept in Argus only.</p>
      )}
    </section>
  );
}

function ItemList({
  reviewId,
  items,
}: {
  reviewId: ReviewId;
  items: ChecklistItem[];
}) {
  const setChecked = useAiStore((s) => s.setChecked);
  const threads = useThreadStore(
    (s) => s.byReview[reviewId]?.threads ?? NO_THREADS,
  );
  return (
    <ul className={styles.list}>
      {items.map((i) => {
        const related = relatedLabel(i, threads);
        const pointed = i.repeatOf ?? i.relatedTo[0] ?? null;
        return (
          <li
            key={i.id}
            className={styles.item}
            data-checked={i.checked}
            aria-label={`Check: ${i.text}`}
          >
            <label className={styles.label}>
              <input
                type="checkbox"
                checked={i.checked}
                onChange={(e) => void setChecked(i, e.target.checked)}
              />
              <span>{i.text}</span>
            </label>
            {/* Outside the label: a click on the tag must not tick it. */}
            {(related || i.repeatOf) && pointed && (
              <span className={styles.tags}>
                <PointLink
                  reviewId={reviewId}
                  threadId={pointed.threadId}
                  findingId={pointed.findingId}
                  title={i.repeatOf ? SHOWN_ANYWAY_HINT : RELATED_HINT}
                >
                  {related ?? `repeats ${repeatTarget(i, threads)}`}
                </PointLink>
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}
