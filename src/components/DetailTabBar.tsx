/**
 * The tab strip above the detail island, in Iris's `TabBar` look (Selene's
 * island chips on the canvas): Threads (with the thread count) and Files.
 * ←/→ move between the tabs, as in an ARIA tablist.
 */

import type { KeyboardEvent } from "react";

import type { DetailTab } from "../state/layoutStore";
import { FilesIcon, ThreadsIcon } from "./icons";
import styles from "./DetailTabBar.module.css";

const TABS: { id: DetailTab; label: string; Icon: typeof ThreadsIcon }[] = [
  { id: "threads", label: "Threads", Icon: ThreadsIcon },
  { id: "files", label: "Files", Icon: FilesIcon },
];

export function DetailTabBar({
  tab,
  onChange,
  threadCount,
}: {
  tab: DetailTab;
  onChange: (tab: DetailTab) => void;
  /** Shown on the Threads chip once the threads are loaded. */
  threadCount: number | null;
}) {
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    // Two tabs: either arrow goes to the other one.
    onChange(tab === "threads" ? "files" : "threads");
  };

  return (
    <div className={styles.bar} role="tablist" aria-label="Review views">
      {TABS.map(({ id, label, Icon }) => {
        const active = id === tab;
        return (
          <button
            key={id}
            type="button"
            role="tab"
            id={`detail-tab-${id}`}
            aria-selected={active}
            aria-controls="detail-panel"
            tabIndex={active ? 0 : -1}
            className={`${styles.tab}${active ? ` ${styles.active}` : ""}`}
            onClick={() => onChange(id)}
            onKeyDown={onKeyDown}
          >
            <Icon className={styles.icon} />
            <span className={styles.title}>{label}</span>
            {id === "threads" && threadCount !== null && (
              <span className={styles.count}>{threadCount}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
