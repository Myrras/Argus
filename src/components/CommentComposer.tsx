/**
 * The comment composer: a markdown textarea with a Write / Preview toggle
 * (the preview is the same `Markdown` the threads use), its actions and
 * Cancel. Used for a new line comment (Add to review / Comment now), a
 * draft's edit (Save) and a reply (Reply).
 *
 * Keys (`lib/keymap`, the `composer` scope): Submit comment (⌘↩ / Ctrl+↩)
 * runs the primary action, Escape cancels, Insert suggestion (⌘G / Ctrl+G)
 * inserts a suggestion block. Nothing runs without one of those or a click.
 *
 * Suggestions (`suggestion`, from `useComposerSuggestion`; only composers
 * on code lines get it): the toolbar's "Insert suggestion" puts a
 * ` ```suggestion ` block at the caret, prefilled with the comment's lines
 * at the head, in the platform's syntax (`lib/suggestions`); when it can't,
 * it stays off with the reason as its tooltip. Inside a suggestion's
 * content Enter keeps the line's indentation and Tab / Shift+Tab indent by
 * the file's step; elsewhere both behave as always. Preview shows a block
 * as the `SuggestedChange` diff a posted one gets. A block that no longer
 * covers the comment's lines is flagged with a fix. Inserting writes text
 * only: nothing posts.
 *
 * Presentational: the text, busy state and error come in as props, so the
 * caller (a store) keeps the text when a post fails or the widget is
 * rebuilt.
 */

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

import type { PostError } from "../ipc/types";
import { takeFocus } from "../lib/focusRequests";
import { formatBinding, matches, withHint } from "../lib/keymap";
import { isMac } from "../lib/platform";
import {
  enterInSuggestion,
  insertSuggestion,
  rangeText,
  retargetSuggestions,
  suggestionMismatch,
  tabInSuggestion,
  type TextEdit,
} from "../lib/suggestions";
import type { ComposerSuggestion } from "../lib/useComposerSuggestion";
import { currentKeymap, useKeymap } from "../lib/useKeymap";
import { toastInfo } from "../state/toastStore";
import { SuggestionIcon } from "./icons";
import { Markdown } from "./Markdown";
import styles from "./CommentComposer.module.css";

/** A binding (`Mod+KeyG`) in `aria-keyshortcuts` form (`Meta+G`). */
function ariaShortcut(binding: string | undefined): string | undefined {
  if (!binding) return undefined;
  return binding
    .split("+")
    .map((p) =>
      p === "Mod"
        ? isMac
          ? "Meta"
          : "Control"
        : p.replace(/^(Key|Digit)/, ""),
    )
    .join("+");
}

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
  suggestion,
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
  /** Suggestion tools: only for a composer on code lines. */
  suggestion?: ComposerSuggestion;
}) {
  const [preview, setPreview] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const keymap = useKeymap();
  const empty = body.trim() === "";
  // The caret to restore after an edit made through `onChange`.
  const pendingSelection = useRef<[number, number] | null>(null);
  // The caret while the preview hides the field.
  const lastCaret = useRef<number | null>(null);

  useLayoutEffect(() => {
    const sel = pendingSelection.current;
    const el = field.current;
    if (!sel || !el) return;
    pendingSelection.current = null;
    el.setSelectionRange(sel[0], sel[1]);
  }, [body, preview]);

  /**
   * Apply `edit` as typing would, so the field's own undo takes it back
   * (`insertText`, where the webview has it); else through `onChange`.
   */
  const applyEdit = (edit: TextEdit) => {
    const el = field.current;
    if (el && !preview) {
      el.focus({ preventScroll: true });
      el.setSelectionRange(edit.start, edit.end);
      let typed = false;
      try {
        typed =
          typeof document.execCommand === "function" &&
          document.execCommand("insertText", false, edit.text);
      } catch {
        typed = false;
      }
      if (
        typed &&
        el.value.slice(edit.start, edit.start + edit.text.length) === edit.text
      ) {
        el.setSelectionRange(edit.select[0], edit.select[1]);
        return;
      }
    }
    pendingSelection.current = edit.select;
    onChange(body.slice(0, edit.start) + edit.text + body.slice(edit.end));
  };

  const insertBlock = () => {
    if (!suggestion || busy) return;
    const { range, lines, blocked } = suggestion;
    if (blocked || !range || !lines) {
      if (blocked) toastInfo("Can't insert a suggestion", blocked);
      return;
    }
    const at = preview
      ? (lastCaret.current ?? body.length)
      : (field.current?.selectionEnd ?? body.length);
    const edit = insertSuggestion(body, at, suggestion.provider, range, lines);
    if (preview) setPreview(false);
    applyEdit(edit);
    suggestion.onInserted?.(range);
  };

  const onFieldKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing || e.metaKey || e.ctrlKey || e.altKey) {
      return;
    }
    const el = e.currentTarget;
    const { selectionStart: from, selectionEnd: to } = el;
    let edit: TextEdit | null = null;
    if (e.key === "Enter" && !e.shiftKey) {
      edit = enterInSuggestion(body, from, to);
    } else if (e.key === "Tab") {
      edit = tabInSuggestion(
        body,
        from,
        to,
        suggestion?.indent ?? "    ",
        e.shiftKey,
      );
    }
    if (!edit) return;
    e.preventDefault();
    applyEdit(edit);
  };

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
      return;
    }
    if (map.insertSuggestion.some((b) => matches(e, b))) {
      // Also without suggestion tools: never the browser's "find next".
      e.preventDefault();
      e.stopPropagation();
      insertBlock();
    }
  };

  const range = suggestion?.blocked === null ? suggestion.range : null;
  const mismatch =
    suggestion && range
      ? suggestionMismatch(
          body,
          suggestion.provider,
          range,
          suggestion.insertedFor,
        )
      : null;
  const fix =
    !mismatch || !suggestion || !range
      ? null
      : suggestion.provider === "gitlab"
        ? {
            label: "Update suggestion range",
            title: `Make the suggestion cover ${rangeText(range)}`,
            run: () => {
              pendingSelection.current = null;
              onChange(retargetSuggestions(body, range));
            },
          }
        : suggestion.onUseRange
          ? {
              label: `Comment on ${rangeText(mismatch.covered)}`,
              title:
                "Put the comment back on the lines the suggestion was written for",
              run: () => suggestion.onUseRange?.(mismatch.covered),
            }
          : null;
  const suggestOff = !suggestion || suggestion.blocked !== null;

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
        {suggestion && (
          <button
            type="button"
            className={`iconButton ${styles.tool}`}
            aria-label="Insert suggestion"
            aria-disabled={suggestOff || busy}
            aria-keyshortcuts={ariaShortcut(keymap.insertSuggestion[0])}
            title={
              suggestion.blocked ??
              withHint("Insert suggestion", keymap, "insertSuggestion")
            }
            data-testid="insert-suggestion"
            onMouseDown={(e) => {
              // Keep the caret where it is in the field.
              e.preventDefault();
            }}
            onClick={() => {
              if (!suggestOff && !busy) insertBlock();
            }}
          >
            <SuggestionIcon />
          </button>
        )}
        <span className={styles.label}>{label}</span>
      </div>
      {preview ? (
        <div className={styles.preview} data-testid="composer-preview">
          {empty ? (
            <p className={styles.nothing}>Nothing to preview.</p>
          ) : (
            <Markdown renderFence={suggestion?.renderFence}>{body}</Markdown>
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
          onKeyDown={onFieldKeyDown}
          onBlur={(e) => {
            lastCaret.current = e.currentTarget.selectionEnd;
          }}
        />
      )}
      {mismatch && (
        <div
          className={styles.mismatch}
          role="status"
          data-testid="suggestion-mismatch"
        >
          <span>{mismatch.message}</span>
          {fix && (
            <button
              type="button"
              className="ghost"
              title={fix.title}
              disabled={busy}
              onClick={fix.run}
            >
              {fix.label}
            </button>
          )}
        </div>
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
