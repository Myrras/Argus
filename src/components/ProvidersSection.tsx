/**
 * The sidebar's Providers section (pinned under the review list): one line
 * per account Argus polls (every enabled host of the user's gh and glab)
 * with its state, then what needs attention — a host not logged in or not
 * reachable, a missing CLI, no accounts at all, and the last refresh's
 * errors (`lib/banner`). Informational only: the list keeps working with
 * whatever hosts do work. "Settings" opens Settings → Accounts on the host.
 * Collapsible like Iris's sidebar sections; collapsed, the title counts the
 * problems.
 */

import type { Account, RefreshError } from "../ipc/types";
import { bannerItems } from "../lib/banner";
import { providerLines } from "../lib/providers";
import { useLayoutStore } from "../state/layoutStore";
import { ProvidersIcon } from "./icons";
import { SectionTitle } from "./SectionTitle";
import sidebar from "./Sidebar.module.css";
import styles from "./ProvidersSection.module.css";

export function ProvidersSection({
  accounts,
  errors,
  onOpenSettings,
}: {
  accounts: readonly Account[] | null;
  errors: readonly RefreshError[];
  /** Opens Settings → Accounts, on that account (`provider:host`) if any. */
  onOpenSettings: (account?: string) => void;
}) {
  const collapsed = useLayoutStore((s) => s.providersCollapsed);
  const setCollapsed = useLayoutStore((s) => s.setProvidersCollapsed);
  const items = bannerItems(accounts, errors);
  const lines = accounts ? providerLines(accounts) : [];

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
          {!accounts ? (
            <p className={styles.muted}>Checking accounts…</p>
          ) : lines.length === 0 ? (
            <p className={styles.muted}>No accounts in use.</p>
          ) : (
            <ul className={styles.lines}>
              {lines.map((line) => (
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
                  title={item.detail}
                >
                  <span className={styles.text}>{item.text}</span>
                  {item.settings && (
                    <button
                      type="button"
                      className={`ghost ${styles.action}`}
                      onClick={() => onOpenSettings(item.account)}
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
