/**
 * The detail island's body: one scrolling review page, a platform's
 * conversation and "Files changed" in one.
 *
 * - Navigation (`data-slot="file-tree"`, collapsible, `layoutStore`): a
 *   filter box, the Conversation entry, then the files as folders or a flat
 *   list, per file its status, +/−, threads and a Viewed checkbox.
 * - The page (`data-slot="diff"`): a toolbar ("N of M files viewed" with a
 *   progress line along its edge, "Since my last review" in later rounds,
 *   unified / side by side as icons, "Hide whitespace changes" — a toggle
 *   that reads "Whitespace hidden" while on, `layoutStore.hideWhitespace`,
 *   key `w` — and the shortcut help) over the
 *   Conversation section (`ConversationCard`: the threads without a code
 *   reference) and every shown file's `FileDiffCard` (its diff with its
 *   threads), in the tree's order. j/k (or the rebound keys) walk the
 *   Conversation and the files, v toggles Viewed (`useAppShortcuts`);
 *   n/p the threads and findings (`focusStore`, `usePageFocus`). Cmd+F
 *   opens the find bar under the toolbar (`FindBar`, `findStore`).
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { FileEntry, ReviewRecord, StoredThread } from "../ipc/types";
import {
  conversationThreads,
  myOpenCount,
  threadsOnFile,
} from "../lib/diffThreads";
import { filterFiles, orderFiles } from "../lib/fileTree";
import { withHint } from "../lib/keymap";
import { useKeymap } from "../lib/useKeymap";
import { useLocalFileMenu } from "../lib/useLocalFileMenu";
import { usePageFocus } from "../lib/usePageFocus";
import { anchorColumn, pauseScrollAnchor } from "../lib/scrollAnchor";
import { canDiffSinceLastReview, reviewRef } from "../lib/reviews";
import {
  CONVERSATION,
  contentsBusy,
  fileSides,
  hasContents,
  useCodeViewStore,
  waitsForLoadAnyway,
} from "../state/codeViewStore";
import { warmFileCache, type FileRequest } from "../ipc/commands";
import { pageIdle, pageLater, startPrefetch, warmed } from "../lib/prefetch";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useHistoryStore } from "../state/historyStore";
import {
  useLayoutStore,
  type DiffLayout,
  type FileListMode,
} from "../state/layoutStore";
import { useOverlayStore } from "../state/overlayStore";
import { useThreadStore } from "../state/threadStore";
import { useFindStore } from "../state/findStore";
import { ConversationCard } from "./ConversationCard";
import { FileDiffCard } from "./FileDiffCard";
import { FileTree } from "./FileTree";
import { FindBar } from "./FindBar";
import {
  DiffSplitIcon,
  DiffUnifiedIcon,
  FileTreeIcon,
  FlatListIcon,
  FolderListIcon,
  KeyboardIcon,
  PanelCloseIcon,
  PanelOpenIcon,
  SinceIcon,
  WhitespaceIcon,
} from "./icons";
import { OpenInEditorButton } from "./OpenInEditorButton";
import { useLocalFiles } from "../state/localFileStore";
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
  const hideWhitespace = useLayoutStore((s) => s.hideWhitespace);
  const toggleWhitespace = useLayoutStore((s) => s.toggleHideWhitespace);
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
  const hasClone = useBackendSettingsStore(
    (s) => `${r.host}/${r.repo}` in (s.settings?.clonePaths ?? {}),
  );
  const keymap = useKeymap();
  const showOverlay = useOverlayStore((s) => s.show);

  const [query, setQuery] = useState("");
  const findOpen = useFindStore((s) => s.open && s.reviewId === r.id);
  // Keyboard moves (n/p, j/k, the palette) put the DOM focus on the item.
  usePageFocus(r.id);
  const canSince = canDiffSinceLastReview(r);

  useEffect(() => {
    void loadThreads(r.id);
  }, [r.id, loadThreads]);
  // The diff editor's code (CodeMirror) loads while the file list does,
  // not after it: the first diff shows sooner.
  useEffect(() => {
    void import("./DiffEditor");
  }, []);
  // The threads' then/now histories (M3), one call per review and head.
  const loadHistories = useHistoryStore((s) => s.loadReview);
  useEffect(() => {
    void loadHistories(r.id, r.headSha);
  }, [r.id, r.headSha, loadHistories]);

  // Load on open; a review whose watermark caught up drops "since" mode.
  const since = (entry?.since ?? false) && canSince;
  const loaded = !!entry?.files;
  useEffect(() => {
    if (!loaded || entry?.since !== since) void load(r.id, since);
    // Only on open and when the mode must change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [r.id, since]);

  const files = entry?.files ?? null;

  // The rest of the files into the core's disk cache, in idle time (none
  // needed with a clone: it answers at once). Once per diff: a viewed mark
  // makes a new file list, not a new diff.
  const filesNow = useRef(files);
  filesNow.current = files;
  const diffKey = files
    ? `${files.reviewId}|${files.baseSha}|${files.headSha}|${files.files.length}`
    : null;
  useEffect(() => {
    const list = filesNow.current;
    if (!diffKey || !list || hasClone) return;
    const wanted = list.files
      .filter((f) => !f.binary && !waitsForLoadAnyway(f))
      .map((f) => fileSides(list, f));
    const key = (req: FileRequest) => `${list.reviewId}|${req.sha}|${req.path}`;
    return startPrefetch(
      wanted,
      {
        busy: contentsBusy,
        warm: (requests) =>
          warmFileCache(list.reviewId, requests).then((n) => {
            for (const req of requests) warmed.add(key(req));
            return n;
          }),
        idle: pageIdle,
        later: pageLater,
      },
      // Held by the page, or warmed before (a review opened again).
      (req) =>
        warmed.has(key(req)) || hasContents(list.reviewId, req.sha, req.path),
    );
  }, [diffKey, hasClone]);

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
  const talkAll = useMemo(
    () => [...talk.conversation, ...talk.elsewhere],
    [talk],
  );

  const byFile = useMemo(() => {
    const m = new Map<string, StoredThread[]>();
    for (const f of files?.files ?? [])
      m.set(f.path, threadsOnFile(threads, f));
    return m;
  }, [files, threads]);
  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const [path, list] of byFile) m.set(path, myOpenCount(list));
    return m;
  }, [byFile]);

  // Scroll to the selected stop (tree click, j/k).
  const selected = entry?.selected ?? null;
  const quietly = entry?.quietly ?? null;
  const cards = useRef(new Map<string, HTMLElement>());
  const lastScrolled = useRef<string | null>(null);
  useEffect(() => {
    if (!selected || selected === lastScrolled.current) return;
    lastScrolled.current = selected;
    // The find bar selected it and scrolls to its match itself.
    if (quietly === selected) return;
    const card = cards.current.get(selected);
    if (card) pauseScrollAnchor();
    card?.scrollIntoView?.({ block: "start" });
    // `quietly` comes with `selected`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);
  // What the user reads stays put while content above it changes height
  // (an item action, a re-read): only navigation scrolls the page.
  const column = useRef<HTMLDivElement>(null);
  const stack = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!column.current || !stack.current) return;
    return anchorColumn(column.current, stack.current);
  }, []);
  // Stable, so the memoized tree rows skip re-renders of the page.
  const pick = useCallback(
    (path: string) => {
      lastScrolled.current = null;
      select(r.id, path);
    },
    [r.id, select],
  );
  const total = files?.files.length ?? 0;
  // Stable, so the memoized file cards skip re-renders of the page.
  const toggleViewed = useCallback(
    (f: FileEntry) => void setViewed(r.id, f.path, !f.viewed),
    [r.id, setViewed],
  );
  const selectFile = useCallback(
    (path: string) => select(r.id, path),
    [r.id, select],
  );
  const setCard = useCallback((key: string, el: HTMLElement | null) => {
    if (el) cards.current.set(key, el);
    else cards.current.delete(key);
  }, []);
  const cardRef = (key: string) => (el: HTMLElement | null) => setCard(key, el);
  const conversationEntry = {
    open: myOpenCount(talkAll),
    total: talkAll.length,
    selected: selected === CONVERSATION,
    onSelect: () => pick(CONVERSATION),
  };

  const viewed = files?.files.filter((f) => f.viewed).length ?? 0;

  // Open in editor: the files in the local clone, when one is set.
  const localPaths = useMemo(
    () => files?.files.map((f) => f.path) ?? null,
    [files],
  );
  useLocalFiles(r.id, localPaths, hasClone);
  const { onContextMenu: onLocalMenu, menu: localMenu } = useLocalFileMenu(
    r.id,
  );
  const fileAction = useCallback(
    (f: FileEntry) => <OpenInEditorButton reviewId={r.id} path={f.path} />,
    [r.id],
  );
  const onTreeContextMenu = useCallback(
    (f: FileEntry, e: React.MouseEvent) => onLocalMenu(f.path, e),
    [onLocalMenu],
  );

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
              <ListModeToggle mode={listMode} onChange={setListMode} />
            </>
          )}
          <button
            type="button"
            className="iconButton"
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
                hideWhitespace={hideWhitespace}
                conversation={conversationEntry}
                fileAction={fileAction}
                onFileContextMenu={onTreeContextMenu}
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
        ref={column}
        className={styles.diff}
        data-slot="diff"
        data-find-open={findOpen}
        aria-label={`Changes in ${reviewRef(r)}`}
      >
        <div className={styles.toolbar}>
          <span
            className={styles.progress}
            aria-live="polite"
            title={files ? `${viewed} of ${total} files viewed` : undefined}
          >
            {files ? `${viewed} of ${total} files viewed` : ""}
          </span>
          {canSince && (
            <button
              type="button"
              className={styles.since}
              aria-pressed={since}
              aria-label="Since my last review"
              onClick={() => void load(r.id, !since)}
              title={
                since
                  ? "Show the whole change again"
                  : "Only what changed since you last marked a round reviewed"
              }
            >
              <SinceIcon />
              <span className={styles.sinceLabel}>Since my last review</span>
            </button>
          )}
          <LayoutToggle layout={layout} onChange={setLayout} />
          <button
            type="button"
            className={styles.whitespace}
            aria-pressed={hideWhitespace}
            aria-label="Hide whitespace changes"
            title={withHint(
              hideWhitespace
                ? "Whitespace changes are hidden: show them again"
                : "Hide whitespace changes: lines that changed only in whitespace count as unchanged",
              keymap,
              "toggleWhitespace",
            )}
            onClick={toggleWhitespace}
          >
            <WhitespaceIcon />
            {hideWhitespace && (
              <span className={styles.whitespaceLabel}>Whitespace hidden</span>
            )}
          </button>
          <button
            type="button"
            className="iconButton"
            aria-label="Keyboard shortcuts"
            title={withHint("Keyboard shortcuts", keymap, "showShortcuts")}
            onClick={() => showOverlay("help")}
          >
            <KeyboardIcon />
          </button>
          {/* How much is viewed, along the toolbar's bottom edge. */}
          {files && total > 0 && (
            <span
              className={styles.meter}
              data-done={viewed === total ? "true" : "false"}
              style={{ transform: `scaleX(${viewed / total})` }}
              aria-hidden
            />
          )}
        </div>
        <FindBar />
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
        <div ref={stack} className={styles.stack}>
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
                hasClone={hasClone}
                layout={layout}
                hideWhitespace={hideWhitespace}
                selected={selected === f.path}
                onSelect={selectFile}
                onToggleViewed={toggleViewed}
                cardRef={setCard}
                onFileContextMenu={onLocalMenu}
              />
            ))
          )}
        </div>
      </div>
      {localMenu}
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
          [
            "unified",
            "Unified",
            "One column, old and new lines interleaved",
            DiffUnifiedIcon,
          ],
          [
            "split",
            "Side by side",
            "Old and new in two columns",
            DiffSplitIcon,
          ],
        ] as const
      ).map(([id, label, hint, Icon]) => (
        <button
          key={id}
          type="button"
          className="iconButton"
          aria-pressed={layout === id}
          aria-label={label}
          title={`${label}: ${hint}`}
          onClick={() => onChange(id)}
        >
          <Icon />
        </button>
      ))}
    </div>
  );
}

/** The file list as folders or as one flat list of paths. */
function ListModeToggle({
  mode,
  onChange,
}: {
  mode: FileListMode;
  onChange: (mode: FileListMode) => void;
}) {
  return (
    <div className={styles.segmented} role="group" aria-label="File list">
      {(
        [
          ["tree", "Show folders", FolderListIcon],
          ["flat", "Show a flat list", FlatListIcon],
        ] as const
      ).map(([id, label, Icon]) => (
        <button
          key={id}
          type="button"
          className="iconButton"
          aria-pressed={mode === id}
          aria-label={label}
          title={label}
          onClick={() => onChange(id)}
        >
          <Icon />
        </button>
      ))}
    </div>
  );
}
