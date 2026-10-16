/**
 * The review list in the sidebar's Reviews section: the four tabs with counts
 * and the reviews of the current tab. Keyboard handling (↑/↓, Enter, 1–4)
 * lives in `App`; this renders the store's cursor and selection. A row's
 * context menu (Iris's `ContextMenu`) opens it in the browser, copies its
 * link or marks its round reviewed.
 */

import { useEffect, useRef, useState } from "react";

import type { ReviewRecord } from "../ipc/types";
import { openExternal } from "../lib/links";
import {
  EMPTY_TAB,
  PROVIDER_LABEL,
  REASON_LABEL,
  TABS,
  reviewRef,
  reviewsInTab,
  tabCounts,
  untouchedLabel,
} from "../lib/reviews";
import { age, fullTime } from "../lib/time";
import { useReviewStore } from "../state/reviewStore";
import { toastError, toastSuccess } from "../state/toastStore";
import { Badge, PlatformBadge } from "./Badge";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import styles from "./ReviewList.module.css";

export function ReviewList({ now }: { now: number }) {
  const reviews = useReviewStore((s) => s.reviews);
  const loaded = useReviewStore((s) => s.loaded);
  const tab = useReviewStore((s) => s.tab);
  const selectedId = useReviewStore((s) => s.selectedId);
  const cursorId = useReviewStore((s) => s.cursorId);
  const setTab = useReviewStore((s) => s.setTab);
  const select = useReviewStore((s) => s.select);
  const markReviewed = useReviewStore((s) => s.markReviewed);
  const [menu, setMenu] = useState<{
    review: ReviewRecord;
    x: number;
    y: number;
  } | null>(null);

  const counts = tabCounts(reviews);
  const visible = reviewsInTab(reviews, tab);

  return (
    <div className={styles.pane}>
      <div className={styles.tabs} role="tablist" aria-label="Review lists">
        {TABS.map((t, i) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls="review-list"
            className={styles.tab}
            title={`${t.label} (${i + 1})`}
            onClick={() => setTab(t.id)}
          >
            <span className={styles.tabLabel}>{t.label}</span>
            <span className={styles.count} data-zero={counts[t.id] === 0}>
              {counts[t.id]}
            </span>
          </button>
        ))}
      </div>

      <div
        id="review-list"
        role="tabpanel"
        aria-labelledby={`tab-${tab}`}
        className={styles.scroll}
      >
        {!loaded ? (
          <p className={styles.empty}>Loading reviews…</p>
        ) : visible.length === 0 ? (
          <p className={styles.empty}>{EMPTY_TAB[tab]}</p>
        ) : (
          <ul className={styles.list}>
            {visible.map((r) => (
              <ReviewItem
                key={r.id}
                review={r}
                now={now}
                selected={r.id === selectedId}
                cursor={r.id === cursorId}
                onSelect={() => select(r.id)}
                onMenu={(x, y) => setMenu({ review: r, x, y })}
              />
            ))}
          </ul>
        )}
      </div>
      <ContextMenu
        open={menu !== null}
        x={menu?.x ?? 0}
        y={menu?.y ?? 0}
        items={menu ? menuItems(menu.review, markReviewed) : []}
        onClose={() => setMenu(null)}
      />
    </div>
  );
}

function menuItems(
  r: ReviewRecord,
  markReviewed: (id: ReviewRecord["id"]) => Promise<void>,
): ContextMenuItem[] {
  return [
    {
      label: `Open in ${PROVIDER_LABEL[r.provider]}`,
      onSelect: () => openExternal(r.url),
    },
    {
      label: "Copy link",
      onSelect: () =>
        void navigator.clipboard
          .writeText(r.url)
          .then(() => toastSuccess("Link copied"))
          .catch(() => toastError("Could not copy the link")),
    },
    {
      label: "Mark round reviewed",
      disabled: r.status !== "needsReview" && r.status !== "needsReReview",
      onSelect: () => void markReviewed(r.id),
    },
  ];
}

function ReviewItem({
  review: r,
  now,
  selected,
  cursor,
  onSelect,
  onMenu,
}: {
  review: ReviewRecord;
  now: number;
  selected: boolean;
  cursor: boolean;
  onSelect: () => void;
  onMenu: (x: number, y: number) => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (cursor) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [cursor]);
  const untouched = untouchedLabel(r.untouchedThreads);
  const showRound = r.status === "needsReReview";
  return (
    <li>
      <button
        ref={ref}
        type="button"
        className={styles.item}
        data-selected={selected}
        data-cursor={cursor}
        aria-current={selected ? "true" : undefined}
        onClick={onSelect}
        onContextMenu={(e) => {
          e.preventDefault();
          onMenu(e.clientX, e.clientY);
        }}
      >
        <span className={styles.row}>
          <PlatformBadge provider={r.provider} />
          <span className={`${styles.ref} mono`}>{reviewRef(r)}</span>
          {r.draft && <Badge>Draft</Badge>}
          <span
            className={styles.age}
            title={fullTime(r.updatedAt ?? r.fetchedAt)}
          >
            {age(r.updatedAt ?? r.fetchedAt, now)}
          </span>
        </span>
        <span className={styles.title}>{r.title}</span>
        <span className={styles.row}>
          <span className={styles.author}>{r.author}</span>
          {r.state !== "open" && (
            <Badge tone={r.state === "merged" ? "accent" : "neutral"}>
              {r.state === "merged" ? "Merged" : "Closed"}
            </Badge>
          )}
        </span>
        {showRound && (
          <span className={styles.round}>
            {r.roundReasons.map((reason) => (
              <Badge key={reason} tone="accent">
                {REASON_LABEL[reason]}
              </Badge>
            ))}
            {untouched && <span className={styles.untouched}>{untouched}</span>}
          </span>
        )}
      </button>
    </li>
  );
}
