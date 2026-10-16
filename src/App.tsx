/**
 * Argus — application shell (Iris's island layout, Podium's look).
 *
 * Layout: a slim title bar on the canvas (brand; when the list was refreshed,
 * the refresh button and the settings gear); below it the sidebar island on
 * the left (the review lists and the providers) and the work column on the
 * right (the detail tab strip and the detail island). This component also
 * owns the app-level wiring: the backend event subscriptions, the keyboard
 * shortcuts, settings access, the sidebar resizer, and suppressing the
 * webview's browser context menu.
 */

import { useCallback, useEffect, useState } from "react";

import { TABS } from "./lib/reviews";
import { isMac, isWindows } from "./lib/platform";
import { ago, fullTime } from "./lib/time";
import { useNow } from "./lib/useNow";
import { useLayoutStore } from "./state/layoutStore";
import { subscribeReviewEvents, useReviewStore } from "./state/reviewStore";
import {
  subscribeSettingsEvents,
  useSettingsStore,
} from "./state/settingsStore";
import { subscribeThreadEvents } from "./state/threadStore";
import { LogoMark } from "./components/LogoMark";
import { ReviewDetail } from "./components/ReviewDetail";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { WindowControls } from "./components/WindowControls";
import { RefreshIcon, SettingsIcon } from "./components/icons";
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
  const refreshing = useReviewStore((s) => s.refreshing);
  const report = useReviewStore((s) => s.lastRefresh);
  const refresh = useReviewStore((s) => s.refresh);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const now = useNow();

  const openSettings = useCallback(() => setSettingsOpen(true), []);
  const closeSettings = useCallback(() => setSettingsOpen(false), []);

  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const setSidebarWidth = useLayoutStore((s) => s.setSidebarWidth);

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

  const onSidebarResizerDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startW = sidebarWidth;
      function onMove(ev: MouseEvent) {
        setSidebarWidth(startW + (ev.clientX - startX));
      }
      function onUp() {
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
      }
      document.body.style.cursor = "col-resize";
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [sidebarWidth, setSidebarWidth],
  );

  // Suppress the webview's default right-click menu (the browser-y "Save as /
  // Print / Share" items, most glaring on Windows' WebView2). Kept inside
  // editable surfaces so native copy/paste still works there.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (
        target?.closest(
          'input, textarea, [contenteditable=""], [contenteditable="true"]',
        )
      ) {
        return;
      }
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

  return (
    <div className={styles.app}>
      <header
        className={`${styles.titlebar}${isWindows ? ` ${styles.titlebarWindows}` : ""}${isMac ? ` ${styles.titlebarMac}` : ""}`}
        // Windows has no native title bar (decorations: false) and macOS
        // overlays its traffic lights on our strip (titleBarStyle: Overlay),
        // so the whole strip is the drag handle on either. Linux keeps its
        // native title bar for now.
        data-tauri-drag-region={isWindows || isMac ? true : undefined}
      >
        {/* macOS draws its own traffic lights top-left (.titlebarMac leaves
            room for them), so the brand is centered instead of leading. */}
        <div
          className={`${styles.brand}${isMac ? ` ${styles.brandCentered}` : ""}`}
        >
          <LogoMark size={18} className={styles.logo} aria-hidden />
          <span className={styles.product}>Argus</span>
          <span className={styles.tagline}>review cockpit</span>
        </div>
        <div className={styles.titlebarRight}>
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
            className={styles.refreshBtn}
            aria-label="Refresh (r)"
            title="Refresh (r)"
            disabled={refreshing}
            data-spinning={refreshing ? "true" : undefined}
            onClick={() => void refresh()}
          >
            <RefreshIcon size={16} />
          </button>
          <button
            type="button"
            className={styles.settingsBtn}
            aria-label="Settings"
            title="Settings (⌘,)"
            onClick={openSettings}
          >
            <SettingsIcon size={16} />
          </button>
          {isWindows && <WindowControls />}
        </div>
      </header>

      <div className={styles.body}>
        <Sidebar now={now} onOpenSettings={openSettings} />
        <div
          className={styles.sidebarResizer}
          onMouseDown={onSidebarResizerDown}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
        />
        <main className={styles.work}>
          <ReviewDetail now={now} />
        </main>
      </div>

      <Toasts />
      <SettingsDialog open={settingsOpen} onClose={closeSettings} />
    </div>
  );
}
