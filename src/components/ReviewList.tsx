/**
 * The review list in the sidebar's Reviews section: the four tabs with counts
 * and the reviews of the current tab. Keyboard handling (↑/↓, Enter, 1–4)
 * lives in `App`; this renders the store's cursor and selection. A row's
 * context menu (Iris's `ContextMenu`) opens it in the browser, copies its
 * link, refreshes it alone or opens Submit review. A row refreshing on its
 * own shows a small turning glyph by its age. A review just approved from Argus stays in
 * its tab as "Approved ✓" for a moment, then leaves.
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
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { toastError, toastSuccess } from "../state/toastStore";
import { AiRunIndicator } from "./AiStatus";
import { RefreshIcon } from "./icons";
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
  const justApproved = useReviewStore((s) => s.justApproved);
  const ask = useOverlayStore((s) => s.ask);
  const refreshingOne = useReviewStore((s) => s.refreshingOne);
  const refreshOne = useReviewStore((s) => s.refreshOne);
  const [menu, setMenu] = useState<{
    review: ReviewRecord;
    x: number;
    y: number;
  } | null>(null);

  const counts = tabCounts(reviews, justApproved);
  const visible = reviewsInTab(reviews, tab, justApproved);

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
                refreshing={!!refreshingOne[r.id]}
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
        items={
          menu
            ? menuItems(menu.review, {
                refreshing: !!refreshingOne[menu.review.id],
                refresh: () => void refreshOne(menu.review.id),
                submitReview: () =>
                  ask({ kind: "submit", reviewId: menu.review.id }),
              })
            : []
        }
        onClose={() => setMenu(null)}
      />
    </div>
  );
}

function menuItems(
  r: ReviewRecord,
  actions: {
    refreshing: boolean;
    refresh: () => void;
    submitReview: () => void;
  },
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
      label: "Refresh",
      disabled: actions.refreshing,
      onSelect: actions.refresh,
    },
    {
      label: "Submit review…",
      disabled: r.state !== "open" || r.status === "approved",
      onSelect: actions.submitReview,
    },
  ];
}

function ReviewItem({
  review: r,
  now,
  selected,
  cursor,
  refreshing,
  onSelect,
  onMenu,
}: {
  review: ReviewRecord;
  now: number;
  selected: boolean;
  cursor: boolean;
  refreshing: boolean;
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
        aria-busy={refreshing || undefined}
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
          <AiRunIndicator reviewId={r.id} />
          {refreshing && (
            <span
              className={styles.refreshing}
              role="img"
              aria-label="Refreshing"
              title="Refreshing this review"
            >
              <RefreshIcon size={11} />
            </span>
          )}
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
          {r.status === "approved" && (
            <span className={styles.approved} role="status">
              Approved ✓
            </span>
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
