/**
 * Collapsible title of a sidebar section (Connections, …): caret, icon, title
 * and — while collapsed — how many rows it hides. Copied from Podium without
 * the agent "needs input" dot.
 */

import type { ComponentType } from "react";

import { CaretIcon } from "./icons";
import styles from "./Sidebar.module.css";

interface SectionTitleProps {
  title: string;
  Icon: ComponentType<{ className?: string }>;
  count: number;
  collapsed: boolean;
  onToggle: () => void;
}

export function SectionTitle({
  title,
  Icon,
  count,
  collapsed,
  onToggle,
}: SectionTitleProps) {
  // No aria-label: the visible text ("Connections (2)") is the accessible
  // name, so a screen reader hears the count too; `aria-expanded` carries the
  // state.
  return (
    <button
      type="button"
      className={styles.sectionToggle}
      aria-expanded={!collapsed}
      title={`${collapsed ? "Expand" : "Collapse"} ${title}`}
      onClick={onToggle}
    >
      <span
        className={styles.caretWrap}
        data-open={!collapsed ? "true" : undefined}
      >
        <CaretIcon size={12} />
      </span>
      <Icon className={styles.panelIcon} />
      <span className={styles.panelTitle}>
        {title}
        {collapsed && count > 0 && (
          <span className={styles.sectionCount}> ({count})</span>
        )}
      </span>
    </button>
  );
}
