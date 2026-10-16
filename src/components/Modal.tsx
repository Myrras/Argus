/**
 * A small, accessible modal dialog (overlay + centered card): the focus
 * moves in on open, Tab and Shift+Tab stay inside, and the focus goes back
 * to what had it when the dialog closes.
 */

import { useEffect, useRef } from "react";
import type {
  KeyboardEvent as ReactKeyboardEvent,
  ReactNode,
  RefObject,
} from "react";
import { createPortal } from "react-dom";

import { activeElement, focusables } from "../lib/focusables";
import { MOTION, usePresence } from "../lib/motion";
import { CloseIcon } from "./icons";
import styles from "./Modal.module.css";

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  /** Footer actions (buttons), rendered right-aligned. */
  footer?: ReactNode;
  /** Tone of the header accent bar. */
  tone?: "default" | "danger" | "warning";
  /** Card width — a px number or any CSS width (e.g. `"min(960px, 94vw)"`). */
  width?: number | string;
  /** What takes the focus on open (a field); the card by default. */
  initialFocus?: RefObject<HTMLElement | null>;
}

export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
  tone = "default",
  width = 460,
  initialFocus,
}: ModalProps) {
  const cardRef = useRef<HTMLDivElement>(null);
  // Keep the dialog mounted while it animates closed (see `usePresence`).
  const { mounted, state } = usePresence(open, MOTION.base);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Give the focus back to what had it before the dialog opened (if it is
  // still there; the dialog may have removed it).
  useEffect(() => {
    if (!open) return;
    const back = activeElement();
    return () => {
      if (back?.isConnected) back.focus({ preventScroll: true });
    };
  }, [open]);

  // Focus the card only when the dialog *opens* — keyed on `open` (and its
  // mount), not `onClose`, so an unstable `onClose` (a fresh closure each
  // render of the parent) can't re-fire this and steal focus mid-typing.
  useEffect(() => {
    // `mounted` lags `open` by a render on the first open (`usePresence`):
    // the card is only there once both are set.
    if (open && mounted) {
      (initialFocus?.current ?? cardRef.current)?.focus({
        preventScroll: true,
      });
    }
    // `initialFocus` is a ref: read when the dialog opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, mounted]);

  if (!mounted) return null;

  // Tab and Shift+Tab wrap inside the card: nothing behind a modal dialog
  // takes the focus.
  const trapTab = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Tab") return;
    const card = cardRef.current;
    const stops = focusables(card);
    const first = stops[0];
    const last = stops[stops.length - 1];
    const at = document.activeElement;
    if (!first || !last) {
      e.preventDefault();
      return;
    }
    if (e.shiftKey && (at === first || at === card)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && at === last) {
      e.preventDefault();
      first.focus();
    }
  };

  // Portal to <body>: a caller inside a transformed/overflow-clipped ancestor
  // (a file pane) would otherwise clip or re-anchor `position: fixed`.
  return createPortal(
    <div
      className={styles.overlay}
      data-state={state}
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={cardRef}
        className={styles.card}
        data-state={state}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        style={{ width }}
        onKeyDown={trapTab}
      >
        <header className={`${styles.header} ${styles[tone]}`}>
          <h2 className={styles.title}>{title}</h2>
          <button
            type="button"
            className="ghost"
            aria-label="Close dialog"
            onClick={onClose}
          >
            <CloseIcon />
          </button>
        </header>
        <div className={styles.body}>{children}</div>
        {footer && <footer className={styles.footer}>{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}
