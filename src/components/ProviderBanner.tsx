/**
 * Provider problems under the title bar: GitHub unavailable (`gh` missing or
 * logged out), GitLab not set up or without a token, and the last refresh's
 * per-provider errors. Informational only — the list keeps working with
 * whatever providers do work.
 */

import type { ProviderStatus, RefreshError } from "../ipc/types";
import { bannerItems } from "../lib/banner";
import styles from "./ProviderBanner.module.css";

export function ProviderBanner({
  status,
  errors,
  onOpenSettings,
}: {
  status: ProviderStatus | null;
  errors: readonly RefreshError[];
  onOpenSettings: () => void;
}) {
  const items = bannerItems(status, errors);
  if (items.length === 0) return null;
  return (
    <div className={styles.banner} role="status" aria-label="Provider status">
      {items.map((item) => (
        <div key={item.key} className={styles.item} data-tone={item.tone}>
          <span className={styles.dot} aria-hidden />
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
        </div>
      ))}
    </div>
  );
}
