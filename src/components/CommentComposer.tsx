/**
 * The comment composer: a markdown textarea with a Write / Preview toggle
 * (the preview is the same `Markdown` the threads use), its actions and
 * Cancel. Used for a new line comment (Add to review / Comment now), a
 * draft's edit (Save) and a reply (Reply).
 *
 * Keys (`lib/keymap`, the `composer` scope): Submit comment (⌘↩ / Ctrl+↩)
 * runs the primary action, Escape cancels. Nothing runs without one of
 * those or a click.
 *
 * Presentational: the text, busy state and error come in as props, so the
 * caller (a store) keeps the text when a post fails or the widget is
 * rebuilt.
 */

import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import type { PostError } from "../ipc/types";
import { takeFocus } from "../lib/focusRequests";
import { formatBinding, matches } from "../lib/keymap";
import { currentKeymap, useKeymap } from "../lib/useKeymap";
import { Markdown } from "./Markdown";
import styles from "./CommentComposer.module.css";

export interface ComposerAction {
  label: string;
  /** Shown while this action runs. */
  busyLabel?: string;
  onClick: () => void;
  title?: string;
  /** Not available right now (the button greys out, ⌘↩ does nothing). */
  disabled?: boolean;
}

export function CommentComposer({
  label,
  body,
  onChange,
  primary,
  secondary,
  onCancel,
  busy = false,
  error = null,
  placeholder = "Leave a comment (markdown)",
  onResize,
  focusKey,
}: {
  /** What is being written, for the field's name ("Comment on line 12"). */
  label: string;
  body: string;
  onChange: (body: string) => void;
  /** The ⌘↩ action. */
  primary: ComposerAction;
  secondary?: ComposerAction;
  onCancel: () => void;
  busy?: boolean;
  error?: PostError | null;
  placeholder?: string;
  /** The block's height may have changed (CodeMirror re-measures). */
  onResize?: () => void;
  /**
   * Take the focus on mount only when this request is pending
   * (`lib/focusRequests`): a widget remount must not steal it. Without
   * one, it focuses on every mount.
   */
  focusKey?: string;
}) {
  const [preview, setPreview] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const keymap = useKeymap();
  const empty = body.trim() === "";

  // Focus on open, and back on the field when leaving the preview.
  const mounted = useRef(false);
  useEffect(() => {
    const first = !mounted.current;
    mounted.current = true;
    if (preview) return;
    if (first && focusKey !== undefined && !takeFocus(focusKey)) return;
    field.current?.focus({ preventScroll: true });
    // Only on open and on leaving the preview.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [preview]);

  // A new height (preview, error, text growing) after React commits it.
  useEffect(() => {
    const id = requestAnimationFrame(() => onResize?.());
    return () => cancelAnimationFrame(id);
  }, [preview, error, body, busy, onResize]);

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    const map = currentKeymap();
    if (map.cancelComment.some((b) => matches(e, b))) {
      e.preventDefault();
      e.stopPropagation();
      if (!busy) onCancel();
      return;
    }
    if (map.submitComment.some((b) => matches(e, b))) {
      e.preventDefault();
      e.stopPropagation();
      if (!busy && !empty && !primary.disabled) primary.onClick();
    }
  };

  const hint = keymap.submitComment[0]
    ? formatBinding(keymap.submitComment[0])
    : "";

  return (
    <div
      className={styles.composer}
      role="group"
      aria-label={label}
      data-busy={busy}
      onKeyDown={onKeyDown}
    >
      <div className={styles.tabs} role="tablist" aria-label="Composer mode">
        <button
          type="button"
          role="tab"
          aria-selected={!preview}
          className={styles.tab}
          onClick={() => setPreview(false)}
        >
          Write
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={preview}
          className={styles.tab}
          onClick={() => setPreview(true)}
        >
          Preview
        </button>
        <span className={styles.label}>{label}</span>
      </div>
      {preview ? (
        <div className={styles.preview} data-testid="composer-preview">
          {empty ? (
            <p className={styles.nothing}>Nothing to preview.</p>
          ) : (
            <Markdown>{body}</Markdown>
          )}
        </div>
      ) : (
        <textarea
          ref={field}
          className={styles.field}
          aria-label={label}
          placeholder={placeholder}
          value={body}
          disabled={busy}
          rows={Math.min(14, Math.max(3, body.split("\n").length + 1))}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
      {error && (
        <p className={styles.error} role="alert">
          {error.message}
        </p>
      )}
      <div className={styles.actions}>
        <span className={styles.hint}>
          {hint && `${hint} ${primary.label.toLowerCase()} · Esc cancel`}
        </span>
        <button type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </button>
        {secondary && (
          <button
            type="button"
            disabled={busy || empty}
            title={secondary.title}
            onClick={secondary.onClick}
          >
            {busy && secondary.busyLabel
              ? secondary.busyLabel
              : secondary.label}
          </button>
        )}
        <button
          type="button"
          className="primary"
          disabled={busy || empty || primary.disabled}
          title={primary.title}
          onClick={primary.onClick}
        >
          {busy && primary.busyLabel ? primary.busyLabel : primary.label}
        </button>
      </div>
    </div>
  );
}
