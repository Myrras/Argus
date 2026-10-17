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
 * (`commentable_lines`, cached) and get the gutter's "+"; a click opens the
 * composer under the line (`commentStore`). Pending comments show under
 * their lines (or at the top, when their line is not in this view). The old
 * side takes none in "since" mode or when `list_files` says its left side
 * is approximate (`baseApproximate`, with a note above the diff).
 *
 * AI findings (`aiStore`) sit under their line like threads
 * (`FindingBlock`), or under "AI findings on this file" when their line is
 * not in the diff shown; dismissed ones only with "Show dismissed".
 */

import {
  lazy,
  memo,
  Suspense,
  useEffect,
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
  sideOf,
} from "../lib/commentRanges";
import {
  fitPlacements,
  placeThreads,
  threadCounts,
  type DiffSide,
} from "../lib/diffThreads";
import { loadContents } from "../state/codeViewStore";
import {
  loadCommentable,
  useCommentStore,
  useDrafts,
  type CommentTarget,
} from "../state/commentStore";
import { findingPlacement } from "../lib/findings";
import { useAiStore, useFindings } from "../state/aiStore";
import { useRevealCard } from "../state/focusStore";
import { DraftBlock, InlineComposer } from "./CommentBlocks";
import { FindingBlock } from "./FindingBlock";
import type { CommentGutter, LineItem } from "./DiffEditor";
import type { DiffLayout } from "../state/layoutStore";
import { Badge } from "./Badge";
import { FileStatusMark, LineStats, ThreadCount } from "./FileStatusMark";
import { CaretIcon } from "./icons";
import { ListedThread, ThreadBlock } from "./ThreadBlock";
import styles from "./FileDiffCard.module.css";

// CodeMirror and its languages load with the first diff, not at startup.
const DiffEditor = lazy(() =>
  import("./DiffEditor").then((m) => ({ default: m.DiffEditor })),
);

/** Above this many changed lines a diff waits for "Load anyway". */
export const LARGE_DIFF_LINES = 1500;

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

/**
 * Memoized: every prop is stable while nothing about the file changes (the
 * callbacks take the path), so a review page re-rendering (the clock, a
 * selection elsewhere) skips the cards. Ages tick inside the thread cards
 * (`useNow`).
 */
export const FileDiffCard = memo(function FileDiffCard({
  review,
  file: f,
  threads,
  provider,
  hasClone,
  layout,
  selected,
  onSelect: select,
  onToggleViewed: toggleViewed,
  cardRef,
}: {
  review: ReviewFiles;
  file: FileEntry;
  threads: readonly StoredThread[];
  provider: ProviderId;
  /** A local clone is set for the review's repository. */
  hasClone: boolean;
  layout: DiffLayout;
  selected: boolean;
  onSelect: (path: string) => void;
  onToggleViewed: (file: FileEntry) => void;
  cardRef: (path: string, el: HTMLElement | null) => void;
}) {
  const onSelect = () => select(f.path);
  const onToggleViewed = () => toggleViewed(f);
  const [collapsed, setCollapsed] = useState(f.viewed);
  const [forced, setForced] = useState(false);
  const [near, setNear] = useState(false);
  const [contents, setContents] = useState<Contents>({ state: "idle" });
  const [attempt, setAttempt] = useState(0);
  const root = useRef<HTMLElement | null>(null);

  // A viewed file folds away; un-viewing opens it again.
  const viewedRef = useRef(f.viewed);
  useEffect(() => {
    if (viewedRef.current !== f.viewed) setCollapsed(f.viewed);
    viewedRef.current = f.viewed;
  }, [f.viewed]);

  // Revealing one of its AI findings opens a folded card.
  const revealSeq = useAiStore((s) =>
    s.reveal?.path === f.path ? s.reveal.seq : null,
  );
  const seenReveal = useRef(revealSeq);
  useEffect(() => {
    if (revealSeq !== null && revealSeq !== seenReveal.current) {
      setCollapsed(false);
    }
    seenReveal.current = revealSeq;
  }, [revealSeq]);

  // So does a keyboard move to one of its threads or findings (M6).
  useRevealCard(f.path, () => setCollapsed(false));

  // Near the viewport? Without IntersectionObserver (tests), always.
  useEffect(() => {
    const el = root.current;
    if (!el || near) return;
    if (typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) setNear(true);
      },
      // The page's own scroller: against the window, the margin would be
      // clipped by the scroller's overflow and count nothing below it.
      {
        root: el.closest<HTMLElement>('[data-slot="diff"]'),
        rootMargin: "600px 0px",
      },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [near]);

  const large = f.tooLarge || f.additions + f.deletions > LARGE_DIFF_LINES;
  const wanted =
    !collapsed && !f.binary && (!large || forced) && (near || selected);

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
    const base =
      f.status === "added"
        ? Promise.resolve("")
        : loadContents(review.reviewId, review.baseSha, f.oldPath ?? f.path);
    const head =
      f.status === "removed"
        ? Promise.resolve("")
        : loadContents(review.reviewId, review.headSha, f.path);
    Promise.all([base, head]).then(
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
  const count = threadCounts(threads);

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
  const lines = useMemo(
    () => ({
      a: oldSideOff ? new Set<number>() : commentableSet(usable, "old"),
      b: commentableSet(usable, "new"),
    }),
    [usable, oldSideOff],
  );

  const targetAt = (
    side: DiffSide,
    anchor: number,
    to: number,
  ): CommentTarget | null => {
    if (!lines[side].has(anchor)) return null;
    const s = sideOf(side);
    const range = clampRange(usable, s, anchor, to);
    if (!range) return null;
    return {
      reviewId,
      path: f.path,
      side: s,
      anchor,
      ...range,
      headSha: review.headSha,
      baseSha: review.sinceLastReview ? null : review.baseSha,
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
          store.extendComposer(usable, to);
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
      selectionSide,
      selFrom,
      selTo,
      focusSide,
      focusLine,
      reviewId,
      f.path,
    ],
  );

  const docLines = ready
    ? { a: lineCount(contents.original), b: lineCount(contents.modified) }
    : { a: 0, b: 0 };
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

  const renderThread = (t: StoredThread, onResize: () => void) => (
    <ThreadBlock thread={t} provider={provider} onResize={onResize} />
  );

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
      aria-label={`Diff of ${f.path}`}
    >
      <header className={styles.header}>
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
        <LineStats additions={f.additions} deletions={f.deletions} />
        <label className={styles.viewed}>
          <input type="checkbox" checked={f.viewed} onChange={onToggleViewed} />
          Viewed
        </label>
      </header>

      {!collapsed && (
        <div className={styles.body}>
          {placed.outdated.length > 0 && (
            <div className={styles.listed}>
              <h3 className={styles.listedTitle}>
                Outdated threads{" "}
                <span className={styles.listedCount}>
                  {placed.outdated.length}
                </span>
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
                <span className={styles.listedCount}>
                  {unplacedDrafts.length}
                </span>
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
                <FindingBlock
                  key={id}
                  reviewId={reviewId}
                  findingId={id}
                  listed
                />
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
              <button type="button" onClick={() => setForced(true)}>
                Load anyway
              </button>
            </div>
          ) : contents.state === "ready" ? (
            <Suspense
              fallback={<p className={styles.placeholder}>Loading the diff…</p>}
            >
              {review.baseApproximate && !review.sinceLastReview && (
                <p className={styles.sideNote} role="note">
                  Removed lines can&apos;t take comments for this file: GitHub
                  could not compute its merge base.
                  {hasClone &&
                    " If your local clone is behind, fetch it to enable them."}
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
      )}
    </section>
  );
});

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
