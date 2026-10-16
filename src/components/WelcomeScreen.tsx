/**
 * Welcome content for the detail island while no review is selected (Iris's
 * `WelcomeScreen`): the mark, a greeting, the keys that drive the list and a
 * refresh button.
 */

import { LogoMark } from "./LogoMark";
import { RefreshIcon } from "./icons";
import styles from "./WelcomeScreen.module.css";

export function WelcomeScreen({
  onRefresh,
  refreshing = false,
}: {
  /** Refreshes every provider; without it the button is shown disabled. */
  onRefresh?: () => void;
  refreshing?: boolean;
}) {
  return (
    <div className={styles.welcome}>
      <LogoMark size={48} className={styles.logo} aria-hidden />
      <h1 className={styles.title}>Welcome to Argus</h1>
      <p className={styles.subtitle}>Select a review to see its threads.</p>
      <p className={styles.keys}>
        <kbd>↑</kbd> <kbd>↓</kbd> move · <kbd>Enter</kbd> open · <kbd>r</kbd>{" "}
        refresh · <kbd>1</kbd>–<kbd>4</kbd> lists
      </p>
      <button
        type="button"
        className={styles.openBtn}
        disabled={!onRefresh || refreshing}
        onClick={onRefresh}
      >
        <RefreshIcon size={15} />
        {refreshing ? "Refreshing…" : "Refresh now"}
      </button>
    </div>
  );
}
