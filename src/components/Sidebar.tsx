/**
 * The left sidebar island (Iris's `Sidebar`): the Reviews section — the four
 * lists with their counts and the reviews of the current one — and, pinned
 * below it, the Providers section.
 */

import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsStore } from "../state/settingsStore";
import { ProvidersSection } from "./ProvidersSection";
import { ReviewList } from "./ReviewList";
import { ReviewsIcon } from "./icons";
import styles from "./Sidebar.module.css";

export function Sidebar({
  now,
  onOpenSettings,
}: {
  now: number;
  onOpenSettings: () => void;
}) {
  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const status = useSettingsStore((s) => s.status);
  const report = useReviewStore((s) => s.lastRefresh);

  return (
    <aside
      className={styles.sidebar}
      style={{ width: sidebarWidth, minWidth: sidebarWidth }}
      aria-label="Sidebar"
    >
      <section className={styles.reviews} aria-label="Reviews">
        <div className={styles.sectionHeader}>
          <ReviewsIcon className={styles.panelIcon} />
          <span className={styles.panelTitle}>Reviews</span>
        </div>
        <ReviewList now={now} />
      </section>
      <ProvidersSection
        status={status}
        errors={report?.errors ?? []}
        onOpenSettings={onOpenSettings}
      />
    </aside>
  );
}
