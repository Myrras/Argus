/**
 * Application shell: title bar (refresh, settings, theme), the provider
 * banner, and the list/detail split. Owns the global keyboard shortcuts and
 * the event subscriptions; everything else lives in the stores.
 */

import { useCallback, useEffect, useState } from "react";

import { GearIcon, RefreshIcon, ThemeIcon } from "./components/icons";
import { LogoMark } from "./components/LogoMark";
import { ProviderBanner } from "./components/ProviderBanner";
import { ReviewDetail } from "./components/ReviewDetail";
import { ReviewList } from "./components/ReviewList";
import { SettingsDialog } from "./components/SettingsDialog";
import { TABS } from "./lib/reviews";
import { ago, fullTime } from "./lib/time";
import { useNow } from "./lib/useNow";
import { subscribeReviewEvents, useReviewStore } from "./state/reviewStore";
import {
  subscribeSettingsEvents,
  useSettingsStore,
} from "./state/settingsStore";
import { subscribeThreadEvents } from "./state/threadStore";
import { useThemeStore } from "./state/themeStore";
import styles from "./App.module.css";

/** Typing in a field, or a modifier chord: not ours. */
function ignoreKey(e: KeyboardEvent): boolean {
  if (e.altKey || e.ctrlKey || e.metaKey) return true;
  const t = e.target;
  if (!(t instanceof HTMLElement)) return false;
  return (
    t.isContentEditable ||
    ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName) ||
    t.closest('[role="dialog"]') !== null
  );
}

export default function App() {
  const themeMode = useThemeStore((s) => s.mode);
  const toggleTheme = useThemeStore((s) => s.toggle);
  const refreshing = useReviewStore((s) => s.refreshing);
  const report = useReviewStore((s) => s.lastRefresh);
  const listError = useReviewStore((s) => s.lastError);
  const status = useSettingsStore((s) => s.status);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const now = useNow();

  const openSettings = useCallback(() => setSettingsOpen(true), []);
  const closeSettings = useCallback(() => setSettingsOpen(false), []);

  // Subscribe first, then load, so nothing between the two is missed.
  useEffect(() => {
    let cancelled = false;
    const offs: (() => void)[] = [];
    void (async () => {
      const subs = await Promise.all([
        subscribeReviewEvents(),
        subscribeThreadEvents(),
        subscribeSettingsEvents(),
      ]).catch(() => [] as (() => void)[]);
      if (cancelled) {
        subs.forEach((off) => off());
        return;
      }
      offs.push(...subs);
      await Promise.all([
        useReviewStore.getState().load(),
        useSettingsStore.getState().load(),
      ]);
    })();
    return () => {
      cancelled = true;
      offs.forEach((off) => off());
    };
  }, []);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
        return;
      }
      if (ignoreKey(e)) return;
      const store = useReviewStore.getState();
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        store.moveCursor(e.key === "ArrowDown" ? 1 : -1);
      } else if (e.key === "Enter") {
        // A focused button handles its own Enter.
        if (e.target instanceof HTMLButtonElement) return;
        e.preventDefault();
        store.openCursor();
      } else if (e.key === "r") {
        e.preventDefault();
        void store.refresh();
      } else if (/^[1-4]$/.test(e.key)) {
        store.setTab(TABS[Number(e.key) - 1].id);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const refresh = useReviewStore((s) => s.refresh);

  return (
    <div className={styles.app}>
      <header className={styles.titlebar}>
        <div className={styles.brand}>
          <LogoMark size={18} className={styles.logo} aria-hidden />
          <span className={styles.product}>Argus</span>
        </div>
        <div className={styles.controls}>
          <span
            className={styles.refreshed}
            title={fullTime(report?.refreshedAt ?? null)}
          >
            {refreshing
              ? "Refreshing…"
              : `Refreshed ${ago(report?.refreshedAt ?? null, now)}`}
          </span>
          <button
            type="button"
            className="ghost"
            aria-label="Refresh (r)"
            title="Refresh (r)"
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            <RefreshIcon className={styles.spin} data-spinning={refreshing} />
          </button>
          <button
            type="button"
            className="ghost"
            aria-label="Settings"
            title="Settings (⌘,)"
            onClick={openSettings}
          >
            <GearIcon />
          </button>
          <button
            type="button"
            className="ghost"
            onClick={toggleTheme}
            aria-label="Toggle theme"
            title={`${themeMode === "dark" ? "Light" : "Dark"} theme`}
          >
            <ThemeIcon />
          </button>
        </div>
      </header>

      <ProviderBanner
        status={status}
        errors={report?.errors ?? []}
        onOpenSettings={openSettings}
      />
      {listError && (
        <div className={styles.listError} role="alert">
          {listError.message}
        </div>
      )}

      <main className={styles.body}>
        <ReviewList now={now} />
        <ReviewDetail now={now} />
      </main>

      <SettingsDialog open={settingsOpen} onClose={closeSettings} />
    </div>
  );
}
