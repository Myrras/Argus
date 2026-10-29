/**
 * A ` ```suggestion ` block as a small diff ("Suggested change"): the lines
 * it replaces as removed, its body as added, in the diff colours. The
 * replaced lines come from the file at `source` (`file_contents`, cached):
 * a thread's own commit (`originalSource`), or the review head for a
 * composer's preview of a suggestion not yet posted; GitHub's `diff_hunk`
 * until then or when that fails; without either, only the added lines
 * show, with a note. Read-only: Argus never applies a suggestion.
 */

import { useEffect, useState } from "react";

import type { Anchor, ProviderId, ReviewId } from "../ipc/types";
import {
  buildSuggestion,
  suggestionRange,
  type SuggestionAnchor,
  type SuggestionSpec,
} from "../lib/suggestions";
import { loadContents } from "../state/codeViewStore";
import styles from "./SuggestedChange.module.css";

export function SuggestedChange({
  spec,
  body,
  reviewId,
  anchor,
  source,
  provider,
  onResize,
}: {
  spec: SuggestionSpec;
  body: string;
  reviewId: ReviewId;
  /** The lines the comment is on (`null`: a thread on the whole review). */
  anchor: (SuggestionAnchor & Pick<Anchor, "diffHunk">) | null;
  /** Where the replaced lines are read; `null`: nowhere. */
  source: { sha: string; path: string } | null;
  provider: ProviderId;
  /** The height may change once the file is in. */
  onResize?: () => void;
}) {
  const [contents, setContents] = useState<string | null>(null);

  const hasRange = suggestionRange(spec, anchor, provider) !== null;
  const sha = hasRange ? (source?.sha ?? null) : null;
  const path = hasRange ? (source?.path ?? null) : null;
  useEffect(() => {
    if (!sha || !path) return;
    let live = true;
    loadContents(reviewId, sha, path).then(
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
  }, [reviewId, sha, path]);

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
