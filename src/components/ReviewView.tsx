/**
 * The detail island's body: one scrolling review page, a platform's
 * conversation and "Files changed" in one.
 *
 * - Navigation (`data-slot="file-tree"`, collapsible, `layoutStore`): a
 *   filter box, the Conversation entry, then the files as folders or a flat
 *   list, per file its status, +/−, threads and a Viewed checkbox.
 * - The page (`data-slot="diff"`): a toolbar ("N of M files viewed",
 *   "Since my last review" in later rounds, unified / side by side) over the
 *   Conversation section (`ConversationCard`: the threads without a code
 *   reference) and every shown file's `FileDiffCard` (its diff with its
 *   threads), in the tree's order. j/k (or the rebound keys) walk the
 *   Conversation and the files, v toggles Viewed (`useAppShortcuts`).
 */

import { useEffect, useMemo, useRef, useState } from "react";

import type { FileEntry, ReviewRecord, StoredThread } from "../ipc/types";
import {
  conversationThreads,
  threadCounts,
  threadsOnFile,
} from "../lib/diffThreads";
import { filterFiles, orderFiles } from "../lib/fileTree";
import { formatBinding } from "../lib/keymap";
import { useKeymap } from "../lib/useKeymap";
import { canDiffSinceLastReview, reviewRef } from "../lib/reviews";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import { useLayoutStore, type DiffLayout } from "../state/layoutStore";
import { useThreadStore } from "../state/threadStore";
import { ConversationCard } from "./ConversationCard";
import { FileDiffCard } from "./FileDiffCard";
import { FileTree } from "./FileTree";
import { FileTreeIcon, PanelCloseIcon, PanelOpenIcon } from "./icons";
import styles from "./ReviewView.module.css";

const EMPTY_THREADS: StoredThread[] = [];

export function ReviewView({
  review: r,
  now = Date.now(),
}: {
  review: ReviewRecord;
  now?: number;
}) {
  const open = useLayoutStore((s) => s.fileTreeOpen);
  const toggle = useLayoutStore((s) => s.toggleFileTree);
  const layout = useLayoutStore((s) => s.diffLayout);
  const setLayout = useLayoutStore((s) => s.setDiffLayout);
  const listMode = useLayoutStore((s) => s.fileListMode);
  const setListMode = useLayoutStore((s) => s.setFileListMode);

  const entry = useCodeViewStore((s) => s.byReview[r.id]);
  const load = useCodeViewStore((s) => s.load);
  const setViewed = useCodeViewStore((s) => s.setViewed);
  const select = useCodeViewStore((s) => s.select);
  const setOrder = useCodeViewStore((s) => s.setOrder);
  const threadEntry = useThreadStore((s) => s.byReview[r.id]);
  const threads = threadEntry?.threads ?? EMPTY_THREADS;
  const loadThreads = useThreadStore((s) => s.load);
  const keymap = useKeymap();

  const [query, setQuery] = useState("");
  const canSince = canDiffSinceLastReview(r);

  useEffect(() => {
    void loadThreads(r.id);
  }, [r.id, loadThreads]);

  // Load on open; a review whose watermark caught up drops "since" mode.
  const since = (entry?.since ?? false) && canSince;
  const loaded = !!entry?.files;
  useEffect(() => {
    if (!loaded || entry?.since !== since) void load(r.id, since);
    // Only on open and when the mode must change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [r.id, since]);

  const files = entry?.files ?? null;
  const shown = useMemo(
    () => orderFiles(filterFiles(files?.files ?? [], query), listMode),
    [files, query, listMode],
  );
  useEffect(() => {
    setOrder(r.id, [CONVERSATION, ...shown.map((f) => f.path)]);
  }, [r.id, shown, setOrder]);

  const talk = useMemo(
    () => conversationThreads(threads, files?.files ?? null, !!entry?.error),
    [threads, files, entry?.error],
  );
  const talkCount = useMemo(
    () => threadCounts([...talk.conversation, ...talk.elsewhere]),
    [talk],
  );

  const byFile = useMemo(() => {
    const m = new Map<string, StoredThread[]>();
    for (const f of files?.files ?? [])
      m.set(f.path, threadsOnFile(threads, f));
    return m;
  }, [files, threads]);
  const counts = useMemo(() => {
    const m = new Map<string, { total: number; mine: number }>();
    for (const [path, list] of byFile) m.set(path, threadCounts(list));
    return m;
  }, [byFile]);

  // Scroll to the selected stop (tree click, j/k).
  const selected = entry?.selected ?? null;
  const cards = useRef(new Map<string, HTMLElement>());
  const lastScrolled = useRef<string | null>(null);
  useEffect(() => {
    if (!selected || selected === lastScrolled.current) return;
    lastScrolled.current = selected;
    cards.current.get(selected)?.scrollIntoView?.({ block: "start" });
  }, [selected]);
  const pick = (path: string) => {
    lastScrolled.current = null;
    select(r.id, path);
  };
  const total = files?.files.length ?? 0;
  const toggleViewed = (f: FileEntry) =>
    void setViewed(r.id, f.path, !f.viewed);
  const cardRef = (key: string) => (el: HTMLElement | null) => {
    if (el) cards.current.set(key, el);
    else cards.current.delete(key);
  };
  const conversationEntry = {
    count: talkCount,
    selected: selected === CONVERSATION,
    onSelect: () => pick(CONVERSATION),
  };

  const viewed = files?.files.filter((f) => f.viewed).length ?? 0;

  return (
    <div className={styles.view}>
      <aside
        className={styles.tree}
        data-open={open ? "true" : "false"}
        data-slot="file-tree"
        aria-label="Review navigation"
      >
        <div className={styles.treeHeader}>
          {open && (
            <>
              <FileTreeIcon className={styles.treeIcon} />
              <span className={styles.treeTitle}>
                Files {files && <span className={styles.count}>{total}</span>}
              </span>
              <button
                type="button"
                className={`ghost ${styles.modeToggle}`}
                onClick={() =>
                  setListMode(listMode === "tree" ? "flat" : "tree")
                }
                title={
                  listMode === "tree" ? "Show a flat list" : "Show folders"
                }
              >
                {listMode === "tree" ? "Flat" : "Tree"}
              </button>
            </>
          )}
          <button
            type="button"
            className={`ghost ${styles.treeToggle}`}
            aria-expanded={open}
            aria-label={
              open ? "Collapse the file tree" : "Expand the file tree"
            }
            title={open ? "Collapse the file tree" : "Expand the file tree"}
            onClick={toggle}
          >
            {open ? <PanelCloseIcon /> : <PanelOpenIcon />}
          </button>
        </div>
        {open && (
          <>
            <div className={styles.filter}>
              <input
                type="search"
                placeholder="Filter files"
                aria-label="Filter files"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setQuery("");
                }}
              />
            </div>
            <div className={styles.treeBody}>
              <FileTree
                files={shown}
                mode={listMode}
                selected={selected}
                counts={counts}
                onSelect={pick}
                onToggleViewed={toggleViewed}
                conversation={conversationEntry}
                empty={
                  !files
                    ? null
                    : total === 0
                      ? "No files changed."
                      : "No files match."
                }
              />
              {!files && entry?.loading && (
                <p className={styles.placeholder}>Loading files…</p>
              )}
            </div>
          </>
        )}
      </aside>
      <div
        className={styles.diff}
        data-slot="diff"
        aria-label={`Changes in ${reviewRef(r)}`}
      >
        <div className={styles.toolbar}>
          <span className={styles.progress} aria-live="polite">
            {files ? `${viewed} of ${total} files viewed` : ""}
          </span>
          {canSince && (
            <button
              type="button"
              className={styles.since}
              aria-pressed={since}
              onClick={() => void load(r.id, !since)}
              title={
                since
                  ? "Show the whole change again"
                  : "Only what changed since you last marked a round reviewed"
              }
            >
              Since my last review
            </button>
          )}
          <LayoutToggle layout={layout} onChange={setLayout} />
          <span className={styles.keys}>
            {formatBinding(keymap.nextFile[0] ?? "")}/
            {formatBinding(keymap.prevFile[0] ?? "")} next/prev ·{" "}
            {formatBinding(keymap.toggleViewed[0] ?? "")} viewed
          </span>
        </div>
        {entry?.error && (
          <div className={styles.error} role="alert">
            {entry.error.message}{" "}
            <button type="button" onClick={() => void load(r.id)}>
              Retry
            </button>
          </div>
        )}
        {files?.truncated && (
          <p className={styles.notice}>
            The platform lists only part of this change; open it in the browser
            for the rest.
          </p>
        )}
        <div className={styles.stack}>
          <ConversationCard
            conversation={talk.conversation}
            elsewhere={talk.elsewhere}
            provider={r.provider}
            loading={
              !threadEntry ||
              (threadEntry.loading && threadEntry.threads.length === 0)
            }
            error={threadEntry?.error ?? null}
            selected={selected === CONVERSATION}
            now={now}
            onSelect={() => select(r.id, CONVERSATION)}
            cardRef={cardRef(CONVERSATION)}
          />
          {!files ? (
            <p className={styles.placeholder}>
              {entry?.loading || !entry ? "Loading the changed files…" : ""}
            </p>
          ) : shown.length === 0 ? (
            <p className={styles.placeholder}>
              {total === 0 ? "No files changed." : "No files match the filter."}
            </p>
          ) : (
            shown.map((f) => (
              <FileDiffCard
                key={`${files.sinceLastReview}:${f.path}`}
                review={files}
                file={f}
                threads={byFile.get(f.path) ?? EMPTY_THREADS}
                provider={r.provider}
                layout={layout}
                selected={selected === f.path}
                now={now}
                onSelect={() => select(r.id, f.path)}
                onToggleViewed={() => toggleViewed(f)}
                cardRef={cardRef(f.path)}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function LayoutToggle({
  layout,
  onChange,
}: {
  layout: DiffLayout;
  onChange: (layout: DiffLayout) => void;
}) {
  return (
    <div className={styles.segmented} role="group" aria-label="Diff layout">
      {(
        [
          ["unified", "Unified"],
          ["split", "Side by side"],
        ] as const
      ).map(([id, label]) => (
        <button
          key={id}
          type="button"
          aria-pressed={layout === id}
          onClick={() => onChange(id)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
