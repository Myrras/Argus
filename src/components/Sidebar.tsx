/**
 * The left sidebar island (Iris's `Sidebar`): the Reviews section — the four
 * lists with their counts and the reviews of the current one — and, pinned
 * below it, the Providers section.
 */

import { currentErrors } from "../lib/banner";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { ProvidersSection } from "./ProvidersSection";
import { ReviewList } from "./ReviewList";
import { ReviewsIcon } from "./icons";
import styles from "./Sidebar.module.css";

export function Sidebar({ now }: { now: number }) {
  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const accounts = useBackendSettingsStore((s) => s.accounts);
  const openSettings = useSettingsRequestStore((s) => s.open);
  const report = useReviewStore((s) => s.lastRefresh);
  const setupChanged = useBackendSettingsStore((s) => s.setupChanged);

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
        accounts={accounts}
        errors={currentErrors(report, setupChanged)}
        onOpenSettings={(account) => openSettings("accounts", account)}
      />
    </aside>
  );
}
