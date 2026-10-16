/**
 * One changed file in the diff column: a header (status, path or old → new,
 * +/−, thread count, Viewed) and the diff, loaded lazily — the file's two
 * sides are fetched only once the card scrolls near the viewport or is
 * selected (cached per review, sha and path). Binary files and diffs too
 * large to render at once stay a collapsed card, so the UI never freezes;
 * "Load anyway" renders a large one.
 *
 * Its threads: current ones under their lines in the editor; outdated ones
 * (and any without a usable line) listed at the top of the card.
 */

import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";

import type {
  FileEntry,
  ProviderId,
  ReviewFiles,
  StoredThread,
} from "../ipc/types";
import { fitPlacements, placeThreads, threadCounts } from "../lib/diffThreads";
import { loadContents } from "../state/codeViewStore";
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

export function FileDiffCard({
  review,
  file: f,
  threads,
  provider,
  layout,
  selected,
  now,
  onSelect,
  onToggleViewed,
  cardRef,
}: {
  review: ReviewFiles;
  file: FileEntry;
  threads: readonly StoredThread[];
  provider: ProviderId;
  layout: DiffLayout;
  selected: boolean;
  now: number;
  onSelect: () => void;
  onToggleViewed: () => void;
  cardRef: (el: HTMLElement | null) => void;
}) {
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
      { rootMargin: "600px 0px" },
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
  const fitted = useMemo(() => {
    if (contents.state !== "ready") {
      return { fit: placed.inline, rest: [] as StoredThread[] };
    }
    return fitPlacements(placed.inline, {
      a: lineCount(contents.original),
      b: lineCount(contents.modified),
    });
  }, [placed, contents]);
  const listed = [...placed.unplaced, ...fitted.rest];
  const count = threadCounts(threads);

  const renderThread = (t: StoredThread, onResize: () => void) => (
    <ThreadBlock thread={t} provider={provider} now={now} onResize={onResize} />
  );

  return (
    <section
      ref={(el) => {
        root.current = el;
        cardRef(el);
      }}
      className={styles.card}
      data-selected={selected}
      data-viewed={f.viewed}
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
                <ListedThread
                  key={t.threadId}
                  thread={t}
                  provider={provider}
                  now={now}
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
                <ListedThread
                  key={t.threadId}
                  thread={t}
                  provider={provider}
                  now={now}
                />
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
              <DiffEditor
                path={f.path}
                original={contents.original}
                modified={contents.modified}
                layout={layout}
                placements={fitted.fit}
                renderThread={renderThread}
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
