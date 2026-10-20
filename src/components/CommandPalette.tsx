/**
 * The command palette (⌘K, M6): one field over a fuzzy-filtered list of
 * every keymap action with its current shortcuts (`lib/keymap`, run through
 * `runShortcut` exactly as its key would), the review actions, "Go to
 * review…" (the reviews of every list) and "Go to file…" (the open review's
 * shown files). ↑/↓ pick, Enter runs, Escape closes; Backspace on an empty
 * field leaves a "Go to" mode. No sibling app has a palette; the listbox is
 * Iris's `GoToField` pattern (the field keeps the focus, the active option
 * is its `aria-activedescendant`).
 *
 * Writing to a platform never runs from here unasked: "Approve…" opens the
 * same `ApproveModal` as the header's button, and "Post all…" a
 * confirmation first (`PaletteConfirms`). "Resolve thread" / "Unresolve
 * thread" (the focused thread, when its platform can resolve it) is the
 * thread button's own action, reversible, so it runs on Enter.
 */

import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";

import type { ReviewId } from "../ipc/types";
import {
  paletteRows,
  type PaletteItem,
  type PaletteMode as Mode,
  type PaletteRow as Row,
  type PaletteSection as Section,
} from "../lib/palette";
import { activeActions, formatBinding } from "../lib/keymap";
import { isMac } from "../lib/platform";
import { PROVIDER_LABEL, STATUS_LABEL, TABS, reviewRef } from "../lib/reviews";
import { runShortcut } from "../lib/useAppShortcuts";
import { useKeymap } from "../lib/useKeymap";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import {
  useCommentStore,
  useDrafts,
  usePostableCount,
} from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useThreadStore } from "../state/threadStore";
import { toastInfo } from "../state/toastStore";
import { Modal } from "./Modal";
import { ApproveModal } from "./ReviewDetail";
import styles from "./CommandPalette.module.css";

const PLACEHOLDER: Record<Mode, string> = {
  all: "Type a command, a review or a file…",
  reviews: "Go to review…",
  files: "Go to file…",
};

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** Everything the palette can offer right now. */
function usePaletteItems(setMode: (m: Mode) => void): PaletteItem[] {
  const keymap = useKeymap();
  const reviews = useReviewStore((s) => s.reviews);
  const review = useReviewStore((s) =>
    s.selectedId
      ? (s.reviews.find((r) => r.id === s.selectedId) ?? null)
      : null,
  );
  const order = useCodeViewStore((s) =>
    review ? s.byReview[review.id]?.order : undefined,
  );
  const postable = usePostableCount(review?.id ?? "");
  const posting = useCommentStore((s) =>
    review ? !!s.posting[review.id] : false,
  );
  const ask = useOverlayStore((s) => s.ask);
  const focusedId = useFocusStore((s) =>
    review && s.reviewId === review.id && s.item?.kind === "thread"
      ? s.item.threadId
      : null,
  );
  const thread = useThreadStore((s) =>
    review && focusedId
      ? (s.byReview[review.id]?.threads.find((t) => t.threadId === focusedId) ??
        null)
      : null,
  );

  return useMemo(() => {
    const out: PaletteItem[] = [];
    const id = review?.id ?? null;
    out.push({
      id: "goto:reviews",
      section: "Commands",
      label: "Go to review…",
      detail: `${reviews.length} in the lists`,
      stay: true,
      run: () => setMode("reviews"),
    });
    if (review && id) {
      out.push({
        id: "goto:files",
        section: "Commands",
        label: "Go to file…",
        detail: reviewRef(review),
        stay: true,
        run: () => setMode("files"),
      });
      if (
        review.status === "needsReview" ||
        review.status === "needsReReview"
      ) {
        out.push({
          id: "review:mark",
          section: "Commands",
          label: "Mark round reviewed",
          detail: "Remembers the current head as reviewed",
          run: () => void useReviewStore.getState().markReviewed(id),
        });
      }
      if (review.state === "open" && !review.approvedAt) {
        out.push({
          id: "review:approve",
          section: "Commands",
          label: "Approve…",
          detail: `Asks first; on ${PROVIDER_LABEL[review.provider]}`,
          run: () => ask({ kind: "approve", reviewId: id }),
        });
      }
      if (thread?.resolvable) {
        out.push({
          id: "thread:resolve",
          section: "Commands",
          label: thread.resolved ? "Unresolve thread" : "Resolve thread",
          detail: `${thread.author}'s thread; on ${PROVIDER_LABEL[review.provider]}`,
          keys: keymap.resolveThread.map((b) => formatBinding(b)),
          run: () =>
            void useThreadStore
              .getState()
              .setResolved(thread, !thread.resolved),
        });
      }
      if (postable > 0 && !posting) {
        out.push({
          id: "review:postAll",
          section: "Commands",
          label: "Post all pending comments…",
          detail: `${postable} pending; asks first`,
          run: () => ask({ kind: "postAll", reviewId: id }),
        });
      }
    }
    for (const a of activeActions()) {
      // Composer keys only mean something inside a comment; the palette
      // itself is open already.
      if (a.scope === "composer" || a.id === "openPalette") continue;
      if (a.scope === "files" && !review) continue;
      out.push({
        id: `action:${a.id}`,
        section: "Actions",
        label: a.label,
        extra: a.group,
        detail: a.note,
        keys: keymap[a.id].map((b) => formatBinding(b)),
        run: () => {
          if (!runShortcut(a.id)) toastInfo(`${a.label}: nothing to act on`);
        },
      });
    }
    if (isMac) {
      // The native menu owns ⌘, on macOS, so the keymap leaves it out.
      out.push({
        id: "action:settings",
        section: "Actions",
        label: "Open Settings",
        keys: ["⌘,"],
        run: () => useSettingsRequestStore.getState().open("general"),
      });
    }
    for (const r of reviews) {
      out.push({
        id: `review:${r.id}`,
        section: "Reviews",
        label: `${reviewRef(r)} ${r.title}`,
        extra: r.author,
        detail: STATUS_LABEL[r.status],
        run: () => {
          const store = useReviewStore.getState();
          // Its own list, so the sidebar shows it selected.
          const tab = TABS.find((t) => t.id === r.status);
          if (tab && store.tab !== tab.id) store.setTab(tab.id);
          store.select(r.id);
        },
      });
    }
    if (id) {
      for (const card of order ?? []) {
        const conversation = card === CONVERSATION;
        out.push({
          id: `file:${card}`,
          section: "Files",
          label: conversation ? "Conversation" : card,
          extra: conversation ? "" : fileName(card),
          detail: conversation ? "Threads on the review as a whole" : undefined,
          run: () =>
            useFocusStore
              .getState()
              .focusItem(
                id,
                conversation
                  ? { kind: "conversation" }
                  : { kind: "file", path: card },
                { move: true },
              ),
        });
      }
    }
    return out;
  }, [keymap, reviews, review, order, postable, posting, ask, setMode, thread]);
}

function Highlighted({ text, indices }: { text: string; indices: number[] }) {
  if (indices.length === 0) return <>{text}</>;
  const marked = new Set(indices);
  const parts: { text: string; mark: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const mark = marked.has(i);
    const last = parts[parts.length - 1];
    if (last && last.mark === mark) last.text += text[i];
    else parts.push({ text: text[i], mark });
  }
  return (
    <>
      {parts.map((p, i) =>
        p.mark ? (
          <mark key={i} className={styles.mark}>
            {p.text}
          </mark>
        ) : (
          <span key={i}>{p.text}</span>
        ),
      )}
    </>
  );
}

function PaletteBody({
  onClose,
  input,
}: {
  onClose: () => void;
  /** The field: the Modal focuses it on open. */
  input: React.RefObject<HTMLInputElement | null>;
}) {
  const [mode, setModeState] = useState<Mode>("all");
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const ids = useId();
  const listId = `${ids}-list`;

  const setMode = useCallback((m: Mode) => {
    setModeState(m);
    setQuery("");
    setActive(0);
  }, []);
  const items = usePaletteItems(setMode);
  const rows = useMemo(
    () => paletteRows(items, mode, query),
    [items, mode, query],
  );
  const current = Math.min(active, Math.max(0, rows.length - 1));

  useEffect(() => {
    document
      .getElementById(`${listId}-${current}`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [current, listId]);

  const runRow = (row: Row | undefined) => {
    if (!row) return;
    if (row.item.stay) {
      row.item.run();
      input.current?.focus();
      return;
    }
    // Close first: the action runs on the page, not inside the dialog.
    onClose();
    row.item.run();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (rows.length === 0) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((current + step + rows.length) % rows.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      runRow(rows[current]);
    } else if (e.key === "Backspace" && query === "" && mode !== "all") {
      e.preventDefault();
      setMode("all");
    }
  };

  let lastSection: Section | null = null;
  return (
    <div className={styles.palette}>
      <div className={styles.field}>
        {mode !== "all" && (
          <span className={styles.mode}>
            {mode === "reviews" ? "Review" : "File"}
          </span>
        )}
        <input
          ref={input}
          type="text"
          role="combobox"
          className={styles.input}
          aria-label={PLACEHOLDER[mode]}
          aria-expanded
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={
            rows.length > 0 ? `${listId}-${current}` : undefined
          }
          placeholder={PLACEHOLDER[mode]}
          value={query}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onKeyDown}
        />
      </div>
      <ul
        id={listId}
        className={styles.list}
        role="listbox"
        aria-label="Commands"
        // Keep the focus in the field.
        onMouseDown={(e) => e.preventDefault()}
      >
        {rows.length === 0 && (
          <li className={styles.empty} role="presentation">
            Nothing matches.
          </li>
        )}
        {rows.map((row, i) => {
          const heading =
            row.item.section !== lastSection ? row.item.section : null;
          lastSection = row.item.section;
          const labelHits = row.match.indices.filter(
            (n) => n < row.item.label.length,
          );
          return [
            heading && mode === "all" && (
              <li
                key={`h:${heading}`}
                className={styles.heading}
                role="presentation"
              >
                {heading}
              </li>
            ),
            <li
              key={row.item.id}
              id={`${listId}-${i}`}
              className={styles.option}
              role="option"
              aria-selected={i === current}
              onMouseMove={() => i !== current && setActive(i)}
              onClick={() => runRow(row)}
            >
              <span className={styles.label}>
                <span className={styles.text}>
                  <Highlighted text={row.item.label} indices={labelHits} />
                </span>
                {row.item.detail && (
                  <small className={styles.detail}>{row.item.detail}</small>
                )}
              </span>
              {row.item.keys && row.item.keys.length > 0 && (
                <span className={styles.keys}>
                  {row.item.keys.map((k) => (
                    <kbd key={k} className={styles.kbd}>
                      {k}
                    </kbd>
                  ))}
                </span>
              )}
            </li>,
          ];
        })}
      </ul>
      <p className={styles.footer} aria-hidden>
        ↑↓ pick · ↩ run · Esc close
        {mode !== "all" && " · ⌫ back"}
      </p>
    </div>
  );
}

/** Confirm "Post all" from the palette: the header's button, asked first. */
function PostAllModal({
  reviewId,
  onClose,
}: {
  reviewId: ReviewId;
  onClose: () => void;
}) {
  const review = useReviewStore((s) =>
    s.reviews.find((r) => r.id === reviewId),
  );
  // What Post all sends: an emptied edit's draft is left out.
  const n = usePostableCount(reviewId);
  const postAll = useCommentStore((s) => s.postAll);
  return (
    <Modal
      open
      title={`Post ${n} pending ${n === 1 ? "comment" : "comments"}`}
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={n === 0}
            onClick={() => {
              onClose();
              void postAll(reviewId);
            }}
          >
            Post all
          </button>
        </>
      }
    >
      <p className={styles.confirmText}>
        Posts every pending comment of{" "}
        <strong>{review ? reviewRef(review) : "this review"}</strong>
        {review ? ` on ${PROVIDER_LABEL[review.provider]}` : ""} now, one by
        one. A comment the platform refuses stays pending with the reason.
      </p>
    </Modal>
  );
}

/** The confirmations the palette asks for (`overlayStore.confirm`). */
function PaletteConfirms() {
  const confirm = useOverlayStore((s) => s.confirm);
  const closeConfirm = useOverlayStore((s) => s.closeConfirm);
  const review = useReviewStore((s) =>
    confirm ? s.reviews.find((r) => r.id === confirm.reviewId) : undefined,
  );
  const drafts = useDrafts(confirm?.reviewId ?? "");
  if (!confirm || !review) return null;
  return confirm.kind === "approve" ? (
    <ApproveModal
      review={review}
      pending={drafts.length}
      open
      onClose={closeConfirm}
    />
  ) : (
    <PostAllModal reviewId={review.id} onClose={closeConfirm} />
  );
}

export function CommandPalette() {
  const open = useOverlayStore((s) => s.open === "palette");
  const close = useOverlayStore((s) => s.close);
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <Modal
        open={open}
        title="Command palette"
        onClose={close}
        width="min(600px, 94vw)"
        initialFocus={input}
      >
        {open && <PaletteBody onClose={close} input={input} />}
      </Modal>
      <PaletteConfirms />
    </>
  );
}
