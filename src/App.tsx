/**
 * Argus — application shell (Iris's island layout, Podium's look).
 *
 * Layout: a slim title bar on the canvas (brand; when the list was refreshed,
 * the refresh button and the settings gear); below it the sidebar island on
 * the left (the review lists and the providers) and the work column on the
 * right (the detail tab strip and the detail island). This component also
 * owns the app-level wiring: the backend event subscriptions, the keyboard
 * shortcuts (`lib/keymap`, `useAppShortcuts`), settings access (native menu
 * on macOS, title-bar gear elsewhere, `settingsRequestStore` from deeper in
 * the tree), the sidebar resizer, and suppressing the webview's browser
 * context menu.
 */

import { useCallback, useEffect, useState } from "react";

import { onMenuOpenSettings } from "./ipc/events";
import { withHint } from "./lib/keymap";
import { isMac, isWindows } from "./lib/platform";
import { ago, fullTime } from "./lib/time";
import { useAppShortcuts } from "./lib/useAppShortcuts";
import { useKeymap } from "./lib/useKeymap";
import { useNow } from "./lib/useNow";
import {
  subscribeBackendSettingsEvents,
  useBackendSettingsStore,
} from "./state/backendSettingsStore";
import { useLayoutStore } from "./state/layoutStore";
import { subscribeReviewEvents, useReviewStore } from "./state/reviewStore";
import { useSettingsRequestStore } from "./state/settingsRequestStore";
import { subscribeThreadEvents } from "./state/threadStore";
import { subscribeCodeViewEvents } from "./state/codeViewStore";
import { LogoMark } from "./components/LogoMark";
import { ReviewDetail } from "./components/ReviewDetail";
import { SettingsModal } from "./components/SettingsModal";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { WindowControls } from "./components/WindowControls";
import { RefreshIcon, SettingsIcon } from "./components/icons";
import styles from "./App.module.css";

export default function App() {
  const refreshing = useReviewStore((s) => s.refreshing);
  const report = useReviewStore((s) => s.lastRefresh);
  const refresh = useReviewStore((s) => s.refresh);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const now = useNow();
  const keymap = useKeymap();

  // Ctrl+, off macOS (rebindable) and the list keys; the browser shortcuts
  // are swallowed everywhere.
  const openSettings = useCallback(() => setSettingsOpen(true), []);
  useAppShortcuts({ onOpenSettings: openSettings });
  const settingsRequest = useSettingsRequestStore((s) => s.request);
  useEffect(() => {
    if (settingsRequest) setSettingsOpen(true);
  }, [settingsRequest]);

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
        subscribeCodeViewEvents(),
        subscribeBackendSettingsEvents(),
      ]).catch(() => [] as (() => void)[]);
      if (cancelled) {
        subs.forEach((off) => off());
        return;
      }
      offs.push(...subs);
      await Promise.all([
        useReviewStore.getState().load(),
        useBackendSettingsStore.getState().load(),
      ]);
    })();
    return () => {
      cancelled = true;
      offs.forEach((off) => off());
    };
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

  // macOS: the native menu's Settings… item (⌘,) emits `menu:open-settings`.
  // Elsewhere the shortcut comes from the keymap (`useAppShortcuts`), so
  // the two never double-fire.
  useEffect(() => {
    const menuSub = onMenuOpenSettings(() => setSettingsOpen(true));
    return () => {
      menuSub.then((unlisten) => unlisten()).catch(() => undefined);
    };
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
            aria-label="Refresh"
            title={withHint("Refresh", keymap, "refresh")}
            disabled={refreshing}
            data-spinning={refreshing ? "true" : undefined}
            onClick={() => void refresh()}
          >
            <RefreshIcon size={16} />
          </button>
          {/* macOS reaches Settings via the native menu bar (⌘,); the gear
              lives in the custom title bar on the other platforms. */}
          {!isMac && (
            <button
              type="button"
              className={styles.settingsBtn}
              aria-label="Settings"
              title={withHint("Settings", keymap, "openSettings")}
              onClick={openSettings}
            >
              <SettingsIcon size={16} />
            </button>
          )}
          {isWindows && <WindowControls />}
        </div>
      </header>

      <div className={styles.body}>
        <Sidebar now={now} />
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
      <SettingsModal
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
          useSettingsRequestStore.getState().clear();
        }}
      />
    </div>
  );
}
