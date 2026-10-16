/**
 * "Look here yourself": what the AI could not verify on its own, for the
 * current round with a checkbox each (stored by Argus, never on the
 * platform); earlier rounds folded under a toggle.
 */

import { useState } from "react";

import type { ChecklistItem, ReviewId } from "../ipc/types";
import { splitChecklist } from "../lib/findings";
import { useAiStore } from "../state/aiStore";
import { CaretIcon } from "./icons";
import styles from "./Checklist.module.css";

const NO_ITEMS: ChecklistItem[] = [];

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
        <ItemList items={current} />
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
                <ItemList items={earlier.filter((i) => i.round === r)} />
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

function ItemList({ items }: { items: ChecklistItem[] }) {
  const setChecked = useAiStore((s) => s.setChecked);
  return (
    <ul className={styles.list}>
      {items.map((i) => (
        <li key={i.id} className={styles.item} data-checked={i.checked}>
          <label className={styles.label}>
            <input
              type="checkbox"
              checked={i.checked}
              onChange={(e) => void setChecked(i, e.target.checked)}
            />
            <span>{i.text}</span>
          </label>
        </li>
      ))}
    </ul>
  );
}
