/**
 * One changed file in the diff column: a header (status, path or old → new,
 * +/−, thread count, Viewed) and the diff, loaded lazily — the file's two
 * sides are fetched only once the card scrolls near the viewport or is
 * selected (cached per review, sha and path). Binary files and diffs too
 * large to render at once stay a collapsed card, so the UI never freezes;
 * "Load anyway" renders a large one.
 *
 * Its threads: current ones under their lines in the editor; outdated ones
 * (and any without a usable line, or all of them when no editor shows)
 * listed at the top of the card.
 *
 * Commenting: once the diff shows, the file's commentable lines are fetched
 * (`commentable_lines`, cached): the hunks' lines, and every other line when
 * the platform takes lines outside the diff (`outside`). The "+" follows the
 * pointer over them; a click opens the composer under the line
 * (`commentStore`), a drag selects a range inside one region
 * (`lib/commentRanges.regionOf`). Pending comments show under
 * their lines (or at the top, when their line is not in this view). The old
 * side takes none in "since" mode or when `list_files` says its left side
 * is approximate (`baseApproximate`, with a note above the diff).
 *
 * AI findings (`aiStore`) sit under their line like threads
 * (`FindingBlock`), or under "AI findings on this file" when their line is
 * not in the diff shown; dismissed ones only with "Show dismissed".
 *
 * Open in editor: with a local clone, the header has the editor button
 * (`OpenInEditorButton`) and right-click opens the file menu.
 *
 * Whitespace changes hidden (`hideWhitespace`, `layoutStore`): the editor
 * diffs with whitespace ignored, the header's +/− come from the patch
 * (`lib/whitespace`), and a file that changed only in whitespace is a
 * folded "Only whitespace changes" card (from its patch, or its contents
 * once loaded) with "Show changes" for this file. With threads, pending
 * comments or findings on its lines it keeps the editor (everything folded
 * but around them) under a note saying so.
 */

import {
  lazy,
  memo,
  Suspense,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import type {
  CommentableLines,
  FileEntry,
  ProviderId,
  ReviewFiles,
  StoredThread,
} from "../ipc/types";
import {
  clampRange,
  commentableSet,
  diffSideOf,
  regionOf,
  sideOf,
} from "../lib/commentRanges";
import {
  fitPlacements,
  placeThreads,
  threadCounts,
  type DiffSide,
} from "../lib/diffThreads";
import {
  loadFileSides,
  useCodeViewStore,
  waitsForLoadAnyway,
} from "../state/codeViewStore";
import {
  clearCommentableCache,
  loadCommentable,
  useCommentStore,
  useDrafts,
  type CommentTarget,
} from "../state/commentStore";
import { findingPlacement } from "../lib/findings";
import {
  fileWhitespaceStats,
  onlyWhitespaceDiffers,
  shownLineStats,
} from "../lib/whitespace";
import { keepFoldedCardInPlace, readingInside } from "../lib/scrollAnchor";
import { useAiStore, useFindings } from "../state/aiStore";
import { useFocusStore, useRevealCard } from "../state/focusStore";
import { pinCard, useMountStore, usePinned } from "../state/mountStore";
import { cardOf } from "../lib/focusModel";
import {
  estimateBodyHeight,
  heightKey,
  measuredHeight,
  rememberHeight,
} from "../lib/cardHeight";
import { DraftBlock, InlineComposer } from "./CommentBlocks";
import { FindingBlock } from "./FindingBlock";
import type { CommentGutter, LineItem } from "./DiffEditor";
import type { DiffLayout } from "../state/layoutStore";
import { Badge } from "./Badge";
import { FileStatusMark, LineStats, ThreadCount } from "./FileStatusMark";
import { CaretIcon } from "./icons";
import { ListedThread, ThreadBlock } from "./ThreadBlock";
import { OpenInEditorButton } from "./OpenInEditorButton";
import styles from "./FileDiffCard.module.css";

// CodeMirror and its languages load with the first diff, not at startup.
const DiffEditor = lazy(() =>
  import("./DiffEditor").then((m) => ({ default: m.DiffEditor })),
);

type Contents =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready"; original: string; modified: string }
  | { state: "error"; message: string };

function lineCount(text: string): number {
  if (text === "") return 0;
  let n = 1;
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) n++;
  return n;
}

/** The card header's height (`FileDiffCard.module.css`). */
const HEADER_PX = 35;

/** Mount a diff within this far of the viewport (the page column's height). */
const MOUNT_MARGIN = "150% 0px";
/** Keep a mounted diff until it is this far away (hysteresis: no flapping). */
const KEEP_MARGIN = "300% 0px";

/**
 * Whether `el` is within `margin` of the page column's viewport, kept
 * current. Without IntersectionObserver (tests), always.
 */
function useNearViewport(
  el: React.RefObject<HTMLElement | null>,
  margin: string,
): boolean {
  const [near, setNear] = useState(
    () => typeof IntersectionObserver === "undefined",
  );
  useEffect(() => {
    const node = el.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver(
      (entries) => {
        const last = entries[entries.length - 1];
        if (last) setNear(last.isIntersecting);
      },
      // The page's own scroller: against the window, the margin would be
      // clipped by the scroller's overflow and count nothing below it.
      {
        root: node.closest<HTMLElement>('[data-slot="diff"]'),
        rootMargin: margin,
      },
    );
    io.observe(node);
    return () => io.disconnect();
  }, [el, margin]);
  return near;
}

/**
 * Memoized: every prop is stable while nothing about the file changes (the
 * callbacks take the path), so a review page re-rendering (the clock, a
 * selection elsewhere) skips the cards. Ages tick inside the thread cards
 * (`useNow`).
 *
 * The card itself (its header and folded state) is always there; its body
 * — the diff, its threads and comments — is mounted only while the card is
 * within 1.5 screens of the viewport (kept until 3), selected, pinned for
 * a reveal (`mountStore`), focused or holding the open composer. Elsewhere
 * a placeholder of the body's last drawn height (else an estimate from the
 * patch, `lib/cardHeight`) keeps the page's height and positions, so a
 * review of hundreds of files keeps a handful of editors alive. What the
 * user changed in a body lives outside it (this shell's fold, "Load
 * anyway" and "Show changes"; the stores' composer, drafts, open threads;
 * the editor's opened stretches in `lib/diffEditors`), so it comes back
 * when the body mounts again.
 */
export const FileDiffCard = memo(function FileDiffCard({
  review,
  file: f,
  threads,
  provider,
  hasClone,
  layout,
  hideWhitespace = false,
  selected,
  onSelect: select,
  onToggleViewed: toggleViewed,
  cardRef,
  onFileContextMenu,
}: {
  review: ReviewFiles;
  file: FileEntry;
  threads: readonly StoredThread[];
  provider: ProviderId;
  /** A local clone is set for the review's repository. */
  hasClone: boolean;
  layout: DiffLayout;
  /** Lines that changed only in whitespace count as unchanged. */
  hideWhitespace?: boolean;
  selected: boolean;
  onSelect: (path: string) => void;
  onToggleViewed: (file: FileEntry) => void;
  cardRef: (path: string, el: HTMLElement | null) => void;
  /** Right-click on the header (the Open in editor menu). */
  onFileContextMenu?: (path: string, e: React.MouseEvent) => void;
}) {
  const onSelect = () => select(f.path);
  const onToggleViewed = () => toggleViewed(f);
  const [collapsed, setCollapsed] = useState(f.viewed);
  const [forced, setForced] = useState(false);
  const root = useRef<HTMLElement | null>(null);
  const reviewId = review.reviewId;

  // A viewed file folds away; un-viewing opens it again. Folding the file
  // being read keeps its header where it was (an action never scrolls).
  const viewedRef = useRef(f.viewed);
  const keepHeader = useRef(false);
  useEffect(() => {
    if (viewedRef.current !== f.viewed) {
      keepHeader.current = f.viewed && readingInside(root.current);
      setCollapsed(f.viewed);
    }
    viewedRef.current = f.viewed;
  }, [f.viewed]);
  useLayoutEffect(() => {
    if (!collapsed || !keepHeader.current) return;
    keepHeader.current = false;
    keepFoldedCardInPlace(root.current);
  }, [collapsed]);

  // Revealing one of its AI findings opens a folded card (and mounts it).
  const revealSeq = useAiStore((s) =>
    s.reveal?.path === f.path ? s.reveal.seq : null,
  );
  const seenReveal = useRef(revealSeq);
  useEffect(() => {
    if (revealSeq !== null && revealSeq !== seenReveal.current) {
      setCollapsed(false);
      pinCard(reviewId, f.path);
    }
    seenReveal.current = revealSeq;
  }, [revealSeq, reviewId, f.path]);

  // So does a keyboard move to one of its threads or findings (M6).
  useRevealCard(f.path, () => {
    setCollapsed(false);
    pinCard(reviewId, f.path);
  });

  // "Show changes" on a whitespace-only file: its diff with whitespace,
  // until the setting changes again.
  const [showWhitespace, setShowWhitespace] = useState(false);
  useEffect(() => setShowWhitespace(false), [hideWhitespace]);

  // -- Mounted or not ---------------------------------------------------------
  const close = useNearViewport(root, MOUNT_MARGIN);
  const within = useNearViewport(root, KEEP_MARGIN);
  const pinned = usePinned(reviewId, f.path);
  const focused = useFocusStore(
    (s) => s.reviewId === reviewId && !!s.item && cardOf(s.item) === f.path,
  );
  const composing = useCommentStore(
    (s) => s.composer?.reviewId === reviewId && s.composer.path === f.path,
  );
  const [mounted, setMounted] = useState(false);
  const live = close || selected || pinned || focused || composing;
  // Mount near; unmount only once well away (and nothing holds it).
  useEffect(() => {
    if (live) setMounted(true);
    else if (!within) setMounted(false);
  }, [live, within]);
  // A pin has done its job once the card is near: nearness keeps it.
  useEffect(() => {
    if (pinned && close) useMountStore.getState().unpin(reviewId, f.path);
  }, [pinned, close, reviewId, f.path]);
  const showBody = mounted || live;

  const large = waitsForLoadAnyway(f);
  const split = layout === "split";
  const hKey = heightKey(reviewId, f.path, layout, hideWhitespace);
  const placeholderHeight =
    measuredHeight(hKey) ??
    estimateBodyHeight(f, {
      split,
      threads: threads.length,
      large: large && !forced,
    });

  const count = threadCounts(threads);
  const stats = shownLineStats(f, hideWhitespace);

  return (
    <section
      ref={(el) => {
        root.current = el;
        cardRef(f.path, el);
      }}
      className={styles.card}
      data-selected={selected}
      data-viewed={f.viewed}
      data-file-card="file"
      data-path={f.path}
      data-mounted={showBody && !collapsed ? "true" : "false"}
      aria-label={`Diff of ${f.path}`}
      style={
        // Off screen and without a diff: the browser need not lay it out
        // or paint it at all (its size stands in); a mounted card is near
        // the viewport, and its sticky header and popovers need it whole.
        showBody && !collapsed
          ? undefined
          : {
              contentVisibility: "auto",
              containIntrinsicSize: `auto ${HEADER_PX + (collapsed ? 0 : placeholderHeight)}px`,
            }
      }
    >
      <header
        className={styles.header}
        onContextMenu={
          onFileContextMenu ? (e) => onFileContextMenu(f.path, e) : undefined
        }
      >
        <button
          type="button"
          className={`ghost ${styles.collapse}`}
          aria-expanded={!collapsed}
          aria-label={collapsed ? `Expand ${f.path}` : `Collapse ${f.path}`}
          onClick={() => {
            setCollapsed((c) => !c);
            onSelect();
          }}
        >
          <CaretIcon
            size={12}
            className={styles.caret}
            data-open={!collapsed}
          />
        </button>
        <FileStatusMark status={f.status} />
        <button
          type="button"
          className={`ghost ${styles.path} mono`}
          data-focus-target
          onClick={onSelect}
          title={f.path}
        >
          {f.oldPath ? (
            <>
              <span className={styles.oldPath}>{f.oldPath}</span>
              <span className={styles.arrow}> → </span>
              {f.path}
            </>
          ) : (
            f.path
          )}
        </button>
        <ThreadCount {...count} />
        <LineStats additions={stats.additions} deletions={stats.deletions} />
        <OpenInEditorButton reviewId={review.reviewId} path={f.path} />
        <label className={styles.viewed}>
          <input type="checkbox" checked={f.viewed} onChange={onToggleViewed} />
          Viewed
        </label>
      </header>

      {!collapsed &&
        (showBody ? (
          <FileDiffBody
            review={review}
            file={f}
            threads={threads}
            provider={provider}
            hasClone={hasClone}
            layout={layout}
            hideWhitespace={hideWhitespace}
            forced={forced}
            onForce={() => setForced(true)}
            showWhitespace={showWhitespace}
            setShowWhitespace={setShowWhitespace}
            heightKey={hKey}
            minHeight={placeholderHeight}
          />
        ) : (
          <div
            className={styles.unmounted}
            data-slot="unmounted-diff"
            style={{ height: placeholderHeight }}
            aria-hidden
          />
        ))}
    </section>
  );
});

/**
 * A card's body: the diff with its threads, comments and findings. Mounted
 * by `FileDiffCard` near the viewport only.
 */
function FileDiffBody({
  review,
  file: f,
  threads,
  provider,
  hasClone,
  layout,
  hideWhitespace,
  forced,
  onForce,
  showWhitespace,
  setShowWhitespace,
  heightKey: hKey,
  minHeight,
}: {
  review: ReviewFiles;
  file: FileEntry;
  threads: readonly StoredThread[];
  provider: ProviderId;
  hasClone: boolean;
  layout: DiffLayout;
  hideWhitespace: boolean;
  forced: boolean;
  onForce: () => void;
  showWhitespace: boolean;
  setShowWhitespace: (v: boolean | ((v: boolean) => boolean)) => void;
  heightKey: string;
  /** The placeholder's height, held while the contents load. */
  minHeight: number;
}) {
  const [contents, setContents] = useState<Contents>({ state: "idle" });
  const [attempt, setAttempt] = useState(0);
  const body = useRef<HTMLDivElement | null>(null);

  // Its drawn height, for the placeholder when it unmounts.
  useEffect(() => {
    const el = body.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      if (contents.state === "ready" || contents.state === "error") {
        rememberHeight(hKey, el.offsetHeight);
      }
    });
    ro.observe(el);
    return () => {
      if (contents.state === "ready") rememberHeight(hKey, el.offsetHeight);
      ro.disconnect();
    };
  }, [hKey, contents.state]);

  const large = waitsForLoadAnyway(f);
  const wanted = !f.binary && (!large || forced);

  // One load per file identity (and per Retry); the cache dedupes across
  // cards and remounts.
  const loadKey = [
    review.reviewId,
    review.baseSha,
    review.headSha,
    f.status,
    f.oldPath ?? "",
    f.path,
    attempt,
  ].join("|");
  const started = useRef<string | null>(null);
  useEffect(() => {
    if (!wanted || started.current === loadKey) return;
    started.current = loadKey;
    setContents({ state: "loading" });
    loadFileSides(review, f).then(
      ([original, modified]) => {
        if (started.current === loadKey) {
          setContents({ state: "ready", original, modified });
        }
      },
      (e: unknown) => {
        if (started.current !== loadKey) return;
        const message =
          typeof e === "object" && e && "message" in e
            ? String((e as { message: unknown }).message)
            : String(e);
        setContents({ state: "error", message });
      },
    );
  }, [wanted, loadKey, f, review]);

  const placed = useMemo(
    () => placeThreads(threads, { since: review.sinceLastReview }),
    [threads, review.sinceLastReview],
  );
  // No editor will show (binary, large and not loaded, failed to load): its
  // current threads are listed with the others instead of shown nowhere.
  const noEditor = f.binary || (large && !forced) || contents.state === "error";
  const fitted = useMemo(() => {
    if (noEditor) {
      return { fit: [], rest: placed.inline.map((p) => p.thread) };
    }
    if (contents.state !== "ready") {
      // Loading: they wait for the editor rather than flash in the list.
      return { fit: placed.inline, rest: [] as StoredThread[] };
    }
    return fitPlacements(placed.inline, {
      a: lineCount(contents.original),
      b: lineCount(contents.modified),
    });
  }, [placed, contents, noEditor]);
  const listed = [...placed.unplaced, ...fitted.rest];

  // -- Commenting ---------------------------------------------------------
  const ready = contents.state === "ready";
  const [commentable, setCommentable] = useState<CommentableLines | null>(null);
  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    loadCommentable(
      review.reviewId,
      review.headSha,
      review.reviewBaseSha,
      f.path,
    ).then(
      (c) => {
        if (!cancelled) setCommentable(c);
      },
      () => {
        // No "+" then; the diff and its threads still show.
        if (!cancelled) setCommentable(null);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [ready, review.reviewId, review.headSha, review.reviewBaseSha, f.path]);
  // Lines read at another head or base than the review's diff would point
  // elsewhere. Compared with `reviewBaseSha`, not `baseSha`: comments go on
  // the review's own diff, also when "since" mode shows a compare.
  const usable =
    commentable &&
    commentable.headSha === review.headSha &&
    commentable.baseSha === review.reviewBaseSha
      ? commentable
      : null;

  const reviewId = review.reviewId;
  const composer = useCommentStore((s) =>
    s.composer?.reviewId === reviewId && s.composer.path === f.path
      ? s.composer
      : null,
  );
  const focus = useCommentStore((s) =>
    s.focus?.reviewId === reviewId && s.focus.path === f.path ? s.focus : null,
  );
  const allDrafts = useDrafts(reviewId);
  const drafts = useMemo(
    () => allDrafts.filter((d) => d.path === f.path),
    [allDrafts, f.path],
  );

  // In "since" mode the left side is the last-reviewed commit, not the
  // base old-side lines refer to; with an approximate base (GitHub had no
  // merge base) the old side shown may not line up with the patch.
  const oldSideOff = review.sinceLastReview || review.baseApproximate;
  const docLines = ready
    ? { a: lineCount(contents.original), b: lineCount(contents.modified) }
    : { a: 0, b: 0 };
  // Every line of a side once the platform takes lines outside the diff
  // (`usable.outside`), else the hunks' lines.
  const lines = useMemo(
    () => ({
      a: oldSideOff
        ? new Set<number>()
        : commentableSet(usable, "old", docLines.a),
      b: commentableSet(usable, "new", docLines.b),
    }),
    [usable, oldSideOff, docLines.a, docLines.b],
  );

  const targetAt = (
    side: DiffSide,
    anchor: number,
    to: number,
  ): CommentTarget | null => {
    if (!lines[side].has(anchor)) return null;
    const s = sideOf(side);
    const range = clampRange(usable, s, anchor, to, docLines[side]);
    if (!range) return null;
    const region = regionOf(usable, s, anchor, docLines[side]);
    return {
      reviewId,
      path: f.path,
      side: s,
      anchor,
      ...range,
      headSha: review.headSha,
      baseSha: review.sinceLastReview ? null : review.baseSha,
      outside: region?.outside && usable ? usable.outside : null,
    };
  };

  const selectionSide = composer ? diffSideOf(composer.side) : null;
  const selFrom = composer ? (composer.startLine ?? composer.line) : 0;
  const selTo = composer?.line ?? 0;
  const focusSide = focus ? diffSideOf(focus.side) : null;
  const focusLine = focus?.line ?? 0;
  const comment: CommentGutter | null = useMemo(
    () => ({
      lines,
      selection: selectionSide
        ? { side: selectionSide, from: selFrom, to: selTo }
        : null,
      focus: focusSide ? { side: focusSide, line: focusLine } : null,
      regionOf: (side, line) => {
        if (!lines[side].has(line)) return null;
        const r = regionOf(usable, sideOf(side), line, docLines[side]);
        return r ? { key: r.key, outside: r.outside } : null;
      },
      outsideHint:
        usable?.outside === "fallback"
          ? "outside the diff: posts as a file comment quoting the lines"
          : usable?.outside === "native"
            ? "outside the diff"
            : null,
      onComment: (side, anchor, to, extend) => {
        const store = useCommentStore.getState();
        const open = store.composer;
        if (
          extend &&
          open &&
          open.reviewId === reviewId &&
          open.path === f.path &&
          open.side === sideOf(side) &&
          !open.draftId
        ) {
          store.extendComposer(usable, to, docLines[side]);
          return;
        }
        const target = targetAt(side, anchor, to);
        // Not `onSelect`: selecting scrolls the card to the top.
        if (target) store.openComposer(target);
      },
      onFocusLine: (side, line) => {
        useCommentStore.getState().setFocus({
          reviewId,
          path: f.path,
          side: sideOf(side),
          line,
          target: targetAt(side, line, line),
        });
      },
    }),
    // `targetAt` reads only what is listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      lines,
      usable,
      docLines.a,
      docLines.b,
      selectionSide,
      selFrom,
      selTo,
      focusSide,
      focusLine,
      reviewId,
      f.path,
    ],
  );

  const hasComposer = !!composer;
  const { items, unplacedDrafts } = useMemo(() => {
    const out: LineItem[] = [];
    const rest: string[] = [];
    for (const d of drafts) {
      const side = diffSideOf(d.side);
      if ((side === "a" && oldSideOff) || d.line > docLines[side]) {
        rest.push(d.id);
        continue;
      }
      out.push({
        key: `draft:${d.id}`,
        side,
        line: d.line,
        render: () => <DraftBlock reviewId={reviewId} id={d.id} />,
      });
    }
    if (composer) {
      out.push({
        key: `composer:${composer.draftId ?? "new"}`,
        side: diffSideOf(composer.side),
        line: composer.line,
        render: (onResize) => <InlineComposer onResize={onResize} />,
      });
    }
    return { items: out, unplacedDrafts: rest };
    // The composer's text is read inside; only where it sits matters here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    drafts,
    reviewId,
    oldSideOff,
    docLines.a,
    docLines.b,
    composer?.side,
    composer?.line,
    composer?.draftId,
    hasComposer,
  ]);

  // -- AI findings ------------------------------------------------------------
  const allFindings = useFindings(reviewId);
  const showDismissed = useAiStore((s) => s.showDismissed);
  const shownDiff =
    !f.binary && (!large || forced) && contents.state !== "error";
  const { findingItems, unplacedFindings } = useMemo(() => {
    const out: LineItem[] = [];
    const rest: string[] = [];
    for (const x of allFindings) {
      if (x.path !== f.path || (x.dismissed && !showDismissed)) continue;
      const at = findingPlacement(x, docLines, review.sinceLastReview);
      // While the diff loads, a finding waits for it rather than flashing
      // in the list at the top.
      if (!at) {
        if (ready || !shownDiff) rest.push(x.id);
        continue;
      }
      out.push({
        key: `finding:${x.id}`,
        side: at.side,
        line: at.line,
        render: (onResize) => (
          <FindingBlock
            reviewId={reviewId}
            findingId={x.id}
            onResize={onResize}
          />
        ),
      });
    }
    return { findingItems: out, unplacedFindings: rest };
    // `docLines` is rebuilt each render; its numbers are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    allFindings,
    showDismissed,
    f.path,
    reviewId,
    review.sinceLastReview,
    docLines.a,
    docLines.b,
    ready,
    shownDiff,
  ]);
  const lineItems = useMemo(
    () => (findingItems.length ? [...items, ...findingItems] : items),
    [items, findingItems],
  );

  // -- Whitespace hidden ------------------------------------------------------
  const hideHere = hideWhitespace && !showWhitespace;
  const whitespaceOnly = useMemo(() => {
    if (!hideWhitespace || f.binary) return false;
    if (contents.state === "ready") {
      return onlyWhitespaceDiffers(contents.original, contents.modified);
    }
    return fileWhitespaceStats(f)?.whitespaceOnly === true;
  }, [hideWhitespace, f, contents]);
  // Nothing on its lines to show: the card folds to a line.
  const whitespaceCard =
    whitespaceOnly &&
    hideHere &&
    fitted.fit.length === 0 &&
    lineItems.length === 0;

  const renderThread = (t: StoredThread, onResize: () => void) => (
    <ThreadBlock thread={t} provider={provider} onResize={onResize} />
  );

  return (
    <div
      ref={body}
      className={styles.body}
      style={
        contents.state === "loading" || contents.state === "idle"
          ? { minHeight }
          : undefined
      }
    >
      {placed.outdated.length > 0 && (
        <div className={styles.listed}>
          <h3 className={styles.listedTitle}>
            Outdated threads{" "}
            <span className={styles.listedCount}>{placed.outdated.length}</span>
          </h3>
          {placed.outdated.map((t) => (
            <ListedThread key={t.threadId} thread={t} provider={provider} />
          ))}
        </div>
      )}
      {unplacedDrafts.length > 0 && (
        <div className={styles.listed}>
          <h3 className={styles.listedTitle}>
            Pending comments not in this view{" "}
            <span className={styles.listedCount}>{unplacedDrafts.length}</span>
          </h3>
          {unplacedDrafts.map((id) => (
            <DraftBlock key={id} reviewId={reviewId} id={id} />
          ))}
        </div>
      )}
      {unplacedFindings.length > 0 && (
        <div className={styles.listed}>
          <h3 className={styles.listedTitle}>
            AI findings on this file{" "}
            <span className={styles.listedCount}>
              {unplacedFindings.length}
            </span>
          </h3>
          {unplacedFindings.map((id) => (
            <FindingBlock key={id} reviewId={reviewId} findingId={id} listed />
          ))}
        </div>
      )}
      {listed.length > 0 && (
        <div className={styles.listed}>
          <h3 className={styles.listedTitle}>
            Threads on this file{" "}
            <span className={styles.listedCount}>{listed.length}</span>
          </h3>
          {listed.map((t) => (
            <ListedThread key={t.threadId} thread={t} provider={provider} />
          ))}
        </div>
      )}

      {f.binary ? (
        <p className={styles.placeholder}>
          <Badge>Binary file</Badge> Not shown as text.
        </p>
      ) : large && !forced ? (
        <div className={styles.placeholder}>
          <Badge tone="warning">Large diff</Badge>
          <span>
            {f.tooLarge
              ? "The platform did not send this diff: it is too large."
              : `${f.additions + f.deletions} changed lines.`}
          </span>
          <button type="button" onClick={onForce}>
            Load anyway
          </button>
        </div>
      ) : whitespaceCard ? (
        <div className={styles.placeholder} data-slot="whitespace-only">
          <Badge>Only whitespace changes</Badge>
          <span>Hidden while whitespace changes are hidden.</span>
          <button type="button" onClick={() => setShowWhitespace(true)}>
            Show changes
          </button>
        </div>
      ) : contents.state === "ready" ? (
        <Suspense
          fallback={<p className={styles.placeholder}>Loading the diff…</p>}
        >
          {review.baseApproximate && !review.sinceLastReview && (
            <p className={styles.sideNote} role="note">
              Removed lines can&apos;t take comments for this file: GitHub could
              not compute its merge base.
              {hasClone &&
                " If your local clone is behind, fetch it to enable them."}
              {hasClone && (
                <>
                  {" "}
                  <button
                    type="button"
                    aria-label="Check again for the merge base"
                    onClick={() => {
                      clearCommentableCache(reviewId);
                      void useCodeViewStore.getState().load(reviewId);
                    }}
                  >
                    Check again
                  </button>
                </>
              )}
            </p>
          )}
          {whitespaceOnly && (
            <p className={styles.sideNote} role="note">
              {hideHere
                ? "Only whitespace changes in this file: folded around its comments."
                : "Only whitespace changes in this file, shown."}{" "}
              <button
                type="button"
                onClick={() => setShowWhitespace((v) => !v)}
              >
                {hideHere ? "Show changes" : "Hide them"}
              </button>
            </p>
          )}
          <DiffEditor
            path={f.path}
            original={contents.original}
            modified={contents.modified}
            layout={layout}
            placements={fitted.fit}
            renderThread={renderThread}
            items={lineItems}
            comment={comment}
            hideWhitespace={hideHere}
          />
        </Suspense>
      ) : contents.state === "error" ? (
        <div className={styles.error} role="alert">
          <p>Could not load the file: {contents.message}</p>
          <button type="button" onClick={() => setAttempt((n) => n + 1)}>
            Retry
          </button>
          {f.patch && <PatchView patch={f.patch} />}
        </div>
      ) : (
        <p className={styles.placeholder}>Loading…</p>
      )}
    </div>
  );
}

/** The platform's patch as plain lines: the fallback when contents fail. */
function PatchView({ patch }: { patch: string }) {
  return (
    <pre className={`${styles.patch} mono`}>
      {patch.split("\n").map((line, i) => (
        <span
          key={i}
          className={styles.patchLine}
          data-kind={
            line.startsWith("@@")
              ? "meta"
              : line.startsWith("+")
                ? "add"
                : line.startsWith("-")
                  ? "del"
                  : "ctx"
          }
        >
          {line || " "}
        </span>
      ))}
    </pre>
  );
}
