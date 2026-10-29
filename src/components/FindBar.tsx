/**
 * The find bar (Cmd+F, `state/findStore`): pinned under the review page's
 * toolbar, in the page's flow (the cards' sticky headers move down by its
 * height, `--page-sticky-top`), so it never covers the first lines.
 *
 * The field (the selection prefilled when it opened), "3 of 27" (with
 * "Searching 12 of 40 files…" while files load, an invalid regex's reason
 * in its place), ↑/↓, the Match case / Whole word / Regex toggles, the
 * scope (Everything, Code only, Comments only), the files not searched
 * with why, and Close. Enter / Shift+Enter (and Mod+G / Mod+Shift+G while
 * the bar is open, `useAppShortcuts`) go to the next / previous match,
 * wrapping with a short "Wrapped" hint; Escape closes the bar and gives the
 * focus back.
 */

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";

import { withHint } from "../lib/keymap";
import { useKeymap } from "../lib/useKeymap";
import { findStatus, type FindScope } from "../lib/findInReview";
import { useFindStore } from "../state/findStore";
import {
  CloseIcon,
  FindIcon,
  MatchCaseIcon,
  NextMatchIcon,
  PrevMatchIcon,
  RegexIcon,
  WholeWordIcon,
} from "./icons";
import styles from "./FindBar.module.css";

const SCOPES: { id: FindScope; label: string }[] = [
  { id: "everything", label: "Everything" },
  { id: "code", label: "Code only" },
  { id: "comments", label: "Comments only" },
];

/** How long "Wrapped" shows after next/previous went round. */
const WRAPPED_MS = 1200;

export function FindBar() {
  const open = useFindStore((s) => s.open);
  if (!open) return null;
  return <OpenFindBar />;
}

function OpenFindBar() {
  const query = useFindStore((s) => s.query);
  const options = useFindStore((s) => s.options);
  const scope = useFindStore((s) => s.scope);
  const re = useFindStore((s) => s.re);
  const error = useFindStore((s) => s.error);
  const matches = useFindStore((s) => s.matches);
  const current = useFindStore((s) => s.current);
  const capped = useFindStore((s) => s.capped);
  const searching = useFindStore((s) => s.searching);
  const progress = useFindStore((s) => s.progress);
  const skipped = useFindStore((s) => s.skipped);
  const wrapSeq = useFindStore((s) => s.wrapSeq);
  const focusSeq = useFindStore((s) => s.focusSeq);
  const { setQuery, setOption, setScope, next, prev, close } =
    useFindStore.getState();
  const keymap = useKeymap();
  const input = useRef<HTMLInputElement>(null);
  const listId = useId();
  const [showSkipped, setShowSkipped] = useState(false);

  // Mod+F (again): take the focus and select the query.
  useLayoutEffect(() => {
    input.current?.focus({ preventScroll: true });
    input.current?.select();
  }, [focusSeq]);

  // "Wrapped" for a moment after going round.
  const [wrapped, setWrapped] = useState(false);
  const seenWrap = useRef(wrapSeq);
  useEffect(() => {
    if (wrapSeq === seenWrap.current) return;
    seenWrap.current = wrapSeq;
    setWrapped(true);
    const t = setTimeout(() => setWrapped(false), WRAPPED_MS);
    return () => clearTimeout(t);
  }, [wrapSeq]);

  const status = findStatus({
    re,
    error,
    matches,
    current,
    capped,
    searching,
    progress,
  });
  const none = !!re && !searching && matches.length === 0;

  const onKeyDown = (e: React.KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  };
  const onInputKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
    e.preventDefault();
    if (e.shiftKey) prev();
    else next();
  };

  const toggle = (
    key: "caseSensitive" | "wholeWord" | "regex",
    label: string,
    Icon: typeof MatchCaseIcon,
  ) => (
    <button
      type="button"
      className={`iconButton ${styles.toggle}`}
      aria-pressed={options[key]}
      aria-label={label}
      title={label}
      onClick={() => setOption(key, !options[key])}
    >
      <Icon />
    </button>
  );

  return (
    <div
      className={styles.bar}
      data-find-bar
      role="search"
      aria-label="Find in review"
      onKeyDown={onKeyDown}
    >
      <label className={styles.field} data-invalid={!!error} data-none={none}>
        <FindIcon className={styles.icon} />
        <input
          ref={input}
          type="text"
          value={query}
          placeholder="Find in review"
          aria-label="Find in review"
          aria-invalid={!!error}
          aria-describedby={`${listId}-status`}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onInputKey}
        />
      </label>
      <span
        id={`${listId}-status`}
        className={styles.status}
        data-error={!!error}
        role={error ? "alert" : "status"}
        aria-live="polite"
      >
        {status.text}
        {wrapped && !error && <span className={styles.wrapped}>Wrapped</span>}
        {status.searching && (
          <span className={styles.searching}>{status.searching}</span>
        )}
      </span>
      {skipped.length > 0 && (
        <button
          type="button"
          className={`ghost ${styles.skipped}`}
          aria-expanded={showSkipped}
          aria-controls={`${listId}-skipped`}
          title={skipped.map((x) => `${x.path}: ${x.reason}`).join("\n")}
          onClick={() => setShowSkipped((v) => !v)}
        >
          {skipped.length} {skipped.length === 1 ? "file" : "files"} not
          searched
        </button>
      )}
      <div className={styles.buttons}>
        <button
          type="button"
          className="iconButton"
          aria-label="Previous match"
          title={withHint("Previous match", keymap, "findPrevious")}
          disabled={matches.length === 0}
          onClick={() => prev()}
        >
          <PrevMatchIcon />
        </button>
        <button
          type="button"
          className="iconButton"
          aria-label="Next match"
          title={withHint("Next match", keymap, "findNext")}
          disabled={matches.length === 0}
          onClick={() => next()}
        >
          <NextMatchIcon />
        </button>
        <span className={styles.divider} aria-hidden />
        {toggle("caseSensitive", "Match case", MatchCaseIcon)}
        {toggle("wholeWord", "Whole word", WholeWordIcon)}
        {toggle("regex", "Regular expression", RegexIcon)}
        <select
          className={styles.scope}
          aria-label="Search in"
          title="What to search"
          value={scope}
          onChange={(e) => setScope(e.target.value as FindScope)}
        >
          {SCOPES.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="iconButton"
          aria-label="Close find"
          title="Close (Esc)"
          onClick={() => close()}
        >
          <CloseIcon />
        </button>
      </div>
      {showSkipped && skipped.length > 0 && (
        <ul
          id={`${listId}-skipped`}
          className={styles.skippedList}
          aria-label="Files not searched"
        >
          {skipped.map((x) => (
            <li key={x.path}>
              <span className="mono">{x.path}</span>
              <span className={styles.reason}>{x.reason}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
