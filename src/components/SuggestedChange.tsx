/**
 * A ` ```suggestion ` block in a comment as a small diff ("Suggested
 * change"): the lines it replaces as removed, its body as added, in the
 * diff colours. The replaced lines come from the file at the comment's
 * commit (`file_contents`, cached), GitHub's `diff_hunk` until then or when
 * that fails; without either, only the added lines show, with a note.
 * Read-only: Argus never applies a suggestion.
 */

import { useEffect, useState } from "react";

import type { ProviderId, StoredThread } from "../ipc/types";
import {
  buildSuggestion,
  originalSource,
  suggestionRange,
  type SuggestionSpec,
} from "../lib/suggestions";
import { loadContents } from "../state/codeViewStore";
import styles from "./SuggestedChange.module.css";

export function SuggestedChange({
  spec,
  body,
  thread: t,
  provider,
  onResize,
}: {
  spec: SuggestionSpec;
  body: string;
  thread: StoredThread;
  provider: ProviderId;
  /** The height may change once the file is in. */
  onResize?: () => void;
}) {
  const anchor = t.anchor;
  const [contents, setContents] = useState<string | null>(null);

  const hasRange = suggestionRange(spec, anchor, provider) !== null;
  const source = anchor && hasRange ? originalSource(anchor) : null;
  const sha = source?.sha ?? null;
  const path = source?.path ?? null;
  useEffect(() => {
    if (!sha || !path) return;
    let live = true;
    loadContents(t.reviewId, sha, path).then(
      (text) => {
        if (!live) return;
        setContents(text);
        requestAnimationFrame(() => onResize?.());
      },
      // Keep the hunk's lines (or the note).
      () => undefined,
    );
    return () => {
      live = false;
    };
    // onResize is a fresh closure per render; the fetch is per file.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [t.reviewId, sha, path]);

  const s = buildSuggestion(spec, body, anchor, provider, contents);
  const range = s.range;
  const where = !range
    ? null
    : range.start === range.end
      ? `line ${range.end}`
      : `lines ${range.start}–${range.end}`;

  return (
    <figure className={styles.suggestion} aria-label="Suggested change">
      <figcaption className={styles.header}>
        <span className={styles.title}>Suggested change</span>
        {where && <span className={styles.where}>{where}</span>}
        {s.removed && s.added.length === 0 && (
          <span className={styles.where}>deletes the lines</span>
        )}
      </figcaption>
      {s.removed === null && (
        <p className={styles.note}>Original lines unavailable</p>
      )}
      <div className={`${styles.lines} mono`}>
        {(s.removed ?? []).map((line, i) => (
          <span key={`d${i}`} className={styles.line} data-kind="del">
            <span className={styles.num}>{(range?.start ?? 0) + i}</span>
            <span className={styles.sign}>-</span>
            {line || " "}
          </span>
        ))}
        {s.added.map((line, i) => (
          <span key={`a${i}`} className={styles.line} data-kind="add">
            <span className={styles.num} />
            <span className={styles.sign}>+</span>
            {line || " "}
          </span>
        ))}
      </div>
    </figure>
  );
}
