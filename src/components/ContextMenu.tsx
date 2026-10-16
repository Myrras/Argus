/**
 * A small cursor-positioned context menu (backdrop + menu card), animated in
 * and out via `usePresence` + `data-state`. Copied from Selene's ContextMenu
 * without its flyout submenus (nothing in Argus needs them yet; Iris's copy).
 */

import { useEffect } from "react";
import { createPortal } from "react-dom";

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

  if (!mounted) return null;

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
        role="menu"
        style={{ left: x, top: y }}
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
