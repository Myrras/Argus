/**
 * The sidebar's Providers section (pinned under the review list): one line
 * per provider with its state, then what needs attention — GitHub
 * unavailable, GitLab not set up or without a token, and the last refresh's
 * per-provider errors (`lib/banner`). Informational only: the list keeps
 * working with whatever providers do work. Collapsible like Iris's sidebar
 * sections; collapsed, the title counts the problems.
 */

import type { ProviderStatus, RefreshError } from "../ipc/types";
import { bannerItems } from "../lib/banner";
import { providerLines } from "../lib/providers";
import { useLayoutStore } from "../state/layoutStore";
import { ProvidersIcon } from "./icons";
import { SectionTitle } from "./SectionTitle";
import sidebar from "./Sidebar.module.css";
import styles from "./ProvidersSection.module.css";

export function ProvidersSection({
  status,
  errors,
  onOpenSettings,
}: {
  status: ProviderStatus | null;
  errors: readonly RefreshError[];
  onOpenSettings: () => void;
}) {
  const collapsed = useLayoutStore((s) => s.providersCollapsed);
  const setCollapsed = useLayoutStore((s) => s.setProvidersCollapsed);
  const items = bannerItems(status, errors);

  return (
    <section className={sidebar.subsection} aria-label="Providers">
      <div className={sidebar.sectionHeader}>
        <SectionTitle
          title="Providers"
          Icon={ProvidersIcon}
          count={items.length}
          collapsed={collapsed}
          onToggle={() => setCollapsed(!collapsed)}
        />
      </div>
      <div
        className={`${sidebar.sectionBody}${collapsed ? ` ${sidebar.sectionBodyCollapsed}` : ""}`}
      >
        <div className={sidebar.sectionBodyInner}>
          {!status ? (
            <p className={styles.muted}>Checking providers…</p>
          ) : (
            <ul className={styles.lines}>
              {providerLines(status).map((line) => (
                <li key={line.key} className={styles.line}>
                  <span
                    className={styles.dot}
                    data-tone={line.tone}
                    aria-hidden
                  />
                  <span className={styles.name}>{line.label}</span>
                  {line.host && (
                    <span className={styles.host} title={line.host}>
                      {line.host}
                    </span>
                  )}
                  <span className={styles.state} data-tone={line.tone}>
                    {line.state}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {items.length > 0 && (
            <ul className={styles.items} role="status">
              {items.map((item) => (
                <li
                  key={item.key}
                  className={styles.item}
                  data-tone={item.tone}
                >
                  <span className={styles.text}>{item.text}</span>
                  {item.settings && (
                    <button
                      type="button"
                      className={`ghost ${styles.action}`}
                      onClick={onOpenSettings}
                    >
                      Settings
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  );
}
