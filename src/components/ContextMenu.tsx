/**
 * A small cursor-positioned context menu (backdrop + menu card), animated in
 * and out via `usePresence` + `data-state`. Copied from Selene's ContextMenu
 * without its flyout submenus (nothing in Argus needs them yet; Iris's copy).
 *
 * Keyboard: the first item takes the focus on open, ↑/↓ (wrapping) and
 * Home/End move over the enabled items, Enter picks, Esc or Tab closes, and
 * the focus goes back to what had it.
 */

import {
  useEffect,
  useRef,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { createPortal } from "react-dom";

import { activeElement } from "../lib/focusables";
import { MOTION, usePresence } from "../lib/motion";
import styles from "./ContextMenu.module.css";

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  /** Its keyboard shortcut, shown at the right edge (`⌘R`). */
  shortcut?: string;
  disabled?: boolean;
  /** Paint the item in the danger tone (delete). */
  danger?: boolean;
}

export function ContextMenu({
  open,
  x,
  y,
  items,
  onClose,
}: {
  open: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  // Keep mounted across the (fast) exit animation.
  const { mounted, state } = usePresence(open, MOTION.fast);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const menuRef = useRef<HTMLDivElement>(null);
  // Into the menu on open; back on close, unless the focus moved on (a
  // dialog an item opened).
  useEffect(() => {
    if (!open) return;
    const back = activeElement();
    return () => {
      // The menu itself is still mounted here (its exit animation).
      const at = document.activeElement;
      const inMenu =
        !at || at === document.body || at.closest('[role="menu"]') !== null;
      if (inMenu && back?.isConnected) back.focus({ preventScroll: true });
    };
  }, [open]);
  useEffect(() => {
    if (open && mounted) enabled(menuRef.current)[0]?.focus();
  }, [open, mounted]);

  if (!mounted) return null;

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const list = enabled(menuRef.current);
    if (list.length === 0) return;
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => {
      e.preventDefault();
      list[(i + list.length) % list.length]?.focus();
    };
    switch (e.key) {
      case "ArrowDown":
        return go(at + 1);
      case "ArrowUp":
        return go(at < 0 ? -1 : at - 1);
      case "Home":
        return go(0);
      case "End":
        return go(list.length - 1);
      case "Tab":
        e.preventDefault();
        onClose();
        return;
    }
  };

  // Portal to <body> so an island's `overflow: hidden` / transform cannot clip
  // or re-anchor the fixed-position menu.
  return createPortal(
    <div
      className={styles.backdrop}
      role="presentation"
      onMouseDown={onClose}
      onContextMenu={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <div
        className={styles.menu}
        data-state={state}
        ref={menuRef}
        role="menu"
        style={{ left: x, top: y }}
        onKeyDown={onKeyDown}
        // Don't let a click inside the menu hit the backdrop's dismiss.
        onMouseDown={(e) => e.stopPropagation()}
      >
        {items.map((item) => (
          <button
            key={item.label}
            type="button"
            role="menuitem"
            className={styles.item}
            data-danger={item.danger ? "true" : undefined}
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            {item.label}
            {item.shortcut && (
              <span className={styles.shortcut} aria-hidden="true">
                {item.shortcut}
              </span>
            )}
          </button>
        ))}
      </div>
    </div>,
    document.body,
  );
}

/** The menu's enabled items, in order. */
function enabled(menu: HTMLElement | null): HTMLButtonElement[] {
  return menu
    ? [
        ...menu.querySelectorAll<HTMLButtonElement>(
          '[role="menuitem"]:not([disabled])',
        ),
      ]
    : [];
}
