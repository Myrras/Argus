/**
 * Find in the review (Cmd+F): the find bar's state and its search.
 *
 * The bar (`FindBar`, pinned under the review page's toolbar) holds a query
 * with Match case / Whole word / Regex and a scope (Everything, Code only,
 * Comments only). The search (`lib/findInReview`, pure) runs over what the
 * stores hold — threads, findings, the files — plus the files' contents,
 * which it loads itself through the same cache the cards use
 * (`codeViewStore.loadFileSides`), six at a time, from the reading position
 * on. Binary, large and patchless files are not searched (as their cards
 * show no diff); they are listed with the reason. The work is sliced: a
 * file at a time, the rest of the page searched in ~8 ms slices, results
 * flushed at most every 100 ms, so a review of 100+ files stays responsive.
 * A newer query (typing, debounced 150 ms) or the bar closing drops an
 * older run.
 *
 * Typing jumps to the first match at or after where the reader is
 * (`lib/findReveal.readingPlace`), once the files between there and that
 * match are searched (so it jumps once, not back and forth while files
 * load). Next / previous wrap, with `wrapSeq` for the bar's hint. Going to a
 * match focuses its item as a click would (`focusStore.focusFound`: its
 * card selected and opened, a thread unfolded; a code line becomes the
 * focused line the comment key uses) and reveals it (`revealMatch`); the
 * DOM focus stays in the bar.
 *
 * The highlighting follows the store: CodeMirror through `lib/findBridge`,
 * comment text through `useFindMarks`.
 */

import { useMemo } from "react";
import { create } from "zustand";

import type { FileEntry, ReviewId } from "../ipc/types";
import { publishFindHighlight, setFindOpener } from "../lib/findBridge";
import {
  firstFrom,
  matchKey,
  prepareCode,
  searchConversation,
  searchFile,
  skipReason,
  type ChunkSpan,
  type FindMatch,
  type FindScope,
  type FindingText,
  type PagePlace,
  type PreparedCode,
  type ReviewPage,
} from "../lib/findInReview";
import { compileFind, type FindOptions } from "../lib/findQuery";
import { markdownText, type FindMarksOptions } from "../lib/markdownText";
import { pageMatchEnv, readingPlace, revealMatch } from "../lib/findReveal";
import {
  CONVERSATION_CARD,
  findingItem,
  threadItem,
  type FocusItem,
} from "../lib/focusModel";
import { lineTarget } from "../lib/keyboardFlow";
import { pauseScrollAnchor } from "../lib/scrollAnchor";
import { useAiStore } from "./aiStore";
import {
  loadFileSides,
  prefetchFile,
  useCodeViewStore,
  waitsForLoadAnyway,
} from "./codeViewStore";
import { useCommentStore } from "./commentStore";
import { useFocusStore } from "./focusStore";
import { useLayoutStore } from "./layoutStore";
import { useReviewStore } from "./reviewStore";
import { useThreadStore } from "./threadStore";

/** Typing waits this long before it searches. */
export const FIND_DEBOUNCE_MS = 150;
/** Results are flushed to the bar at most this often while files load. */
const FLUSH_MS = 100;
/** Files loaded at once. */
const PARALLEL_LOADS = 6;
/** A search slice's budget before it yields to the page. */
const SLICE_MS = 8;
/** More matches than this are not listed ("10000+"). */
export const MAX_MATCHES = 10000;

export interface SkippedFile {
  path: string;
  reason: string;
}

export interface FindState {
  open: boolean;
  reviewId: ReviewId | null;
  query: string;
  options: FindOptions;
  scope: FindScope;
  /** The compiled query; null when empty or invalid. */
  re: RegExp | null;
  /** An invalid regex, in a few words. */
  error: string | null;
  matches: FindMatch[];
  /** The current match's index; -1 for none. */
  current: number;
  /** More than `MAX_MATCHES`: the rest are not listed. */
  capped: boolean;
  /** A search is running (files still loading or being searched). */
  searching: boolean;
  /** Files whose code is searched (or failed), of all searchable ones. */
  progress: { done: number; total: number };
  /** Files whose code is not searched, with why. */
  skipped: SkippedFile[];
  /** Grows when next/previous wrapped around (the bar's hint). */
  wrapSeq: number;
  /** Grows when the bar must take the focus and select its text. */
  focusSeq: number;

  /**
   * Mod+F: from the sidebar, its list filter when it has one; else open
   * (or refocus) the bar on the open review, the selection as the query.
   * False when there is no review to search.
   */
  openBar: (opts?: { target?: EventTarget | null }) => boolean;
  /** Escape / Close: the focus goes back where it was (unless `false`). */
  close: (opts?: { restoreFocus?: boolean }) => void;
  setQuery: (query: string) => void;
  setOption: (key: keyof FindOptions, value: boolean) => void;
  setScope: (scope: FindScope) => void;
  /** Enter / Mod+G: false when there is nothing to go to. */
  next: () => boolean;
  /** Shift+Enter / Mod+Shift+G. */
  prev: () => boolean;
}

const NO_PROGRESS = { done: 0, total: 0 };

// -- Run state (one search at a time) ------------------------------------------

/** Bumped by every new run and by closing: an older run stops. */
let gen = 0;
let debounce: ReturnType<typeof setTimeout> | null = null;
let returnFocus: HTMLElement | null = null;
/** Where the next results jump from (the reading position, or the match). */
let anchor: PagePlace | null = null;
/** The next results pick the first match from `anchor` and reveal it. */
let autoJump = false;
let cancelReveal: (() => void) | null = null;

/** Prepared contents per review / sha / file, as the cache keys them. */
const prepared = new Map<string, PreparedCode>();
const MAX_PREPARED = 400;

function remember(key: string, code: PreparedCode) {
  prepared.delete(key);
  prepared.set(key, code);
  if (prepared.size > MAX_PREPARED) {
    const oldest = prepared.keys().next().value;
    if (oldest !== undefined) prepared.delete(oldest);
  }
}

/** Tests: forget the prepared contents and any run. */
export function resetFindForTests() {
  prepared.clear();
  gen += 1;
  if (debounce) clearTimeout(debounce);
  debounce = null;
  returnFocus = null;
  anchor = null;
  autoJump = false;
  cancelReveal?.();
  cancelReveal = null;
  useFindStore.setState(INITIAL);
}

function yieldToPage(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

/** The two sides' changed chunks, as the diff editor computes them. */
async function chunksOf(
  original: string,
  modified: string,
  hideWhitespace: boolean,
): Promise<readonly ChunkSpan[]> {
  const [{ Chunk }, { Text }, { editorDiffConfig }] = await Promise.all([
    import("@codemirror/merge"),
    import("@codemirror/state"),
    import("../lib/whitespaceDiff"),
  ]);
  return Chunk.build(
    Text.of(original.split("\n")),
    Text.of(modified.split("\n")),
    editorDiffConfig(hideWhitespace),
  );
}

// -- The page as the stores have it ------------------------------------------------

function pageOf(reviewId: ReviewId): ReviewPage {
  const code = useCodeViewStore.getState().byReview[reviewId];
  const ai = useAiStore.getState();
  const enabled = !!ai.runs[reviewId]?.enabled;
  const findings: FindingText[] = enabled
    ? (ai.byReview[reviewId]?.findings ?? [])
        .filter((f) => ai.showDismissed || !f.dismissed)
        .map((f) => ({
          finding: f,
          title: f.title,
          body: f.dismissed ? null : (ai.ui[f.id]?.edit ?? f.body),
        }))
    : [];
  return {
    threads: useThreadStore.getState().byReview[reviewId]?.threads ?? [],
    files: code?.files?.files ?? null,
    failed: !!code?.error,
    order: code?.order.length ? code.order : [CONVERSATION_CARD],
    since: code?.files?.sinceLastReview ?? false,
    findings,
  };
}

function preparedKey(reviewId: ReviewId, f: FileEntry): string | null {
  const files = useCodeViewStore.getState().byReview[reviewId]?.files;
  if (!files) return null;
  return [
    reviewId,
    files.baseSha,
    files.headSha,
    f.status,
    f.oldPath ?? "",
    f.path,
    useLayoutStore.getState().hideWhitespace ? "w" : "",
  ].join("|");
}

// -- The store ---------------------------------------------------------------------

const INITIAL = {
  open: false,
  reviewId: null,
  query: "",
  options: { caseSensitive: false, wholeWord: false, regex: false },
  scope: "everything" as FindScope,
  re: null,
  error: null,
  matches: [] as FindMatch[],
  current: -1,
  capped: false,
  searching: false,
  progress: NO_PROGRESS,
  skipped: [] as SkippedFile[],
  wrapSeq: 0,
  focusSeq: 0,
};

/** The selected text where Mod+F was pressed: one line, not too long. */
function selectionText(active: Element | null): string | null {
  let text = "";
  if (
    active instanceof HTMLTextAreaElement ||
    (active instanceof HTMLInputElement && active.type === "text")
  ) {
    const { selectionStart: s, selectionEnd: e } = active;
    if (s !== null && e !== null && e > s) text = active.value.slice(s, e);
  } else if (typeof window.getSelection === "function") {
    text = window.getSelection()?.toString() ?? "";
  }
  return text && !text.includes("\n") && text.length <= 200 ? text : null;
}

export const useFindStore = create<FindState>((set, get) => ({
  ...INITIAL,

  openBar: (opts) => {
    const target = opts?.target ?? null;
    // In the sidebar: its own filter, when the list has one.
    if (target instanceof Element) {
      const filter = target
        .closest('[aria-label="Sidebar"]')
        ?.querySelector<HTMLInputElement>("[data-list-filter]");
      if (filter) {
        filter.focus();
        filter.select();
        return true;
      }
    }
    const reviewId = useReviewStore.getState().selectedId;
    if (!reviewId) return false;
    const active =
      typeof document !== "undefined"
        ? (document.activeElement as HTMLElement | null)
        : null;
    const inBar = !!active?.closest("[data-find-bar]");
    if (!inBar) {
      // From a dialog (the palette): it gives the focus back itself.
      returnFocus =
        active && active !== document.body && !active.closest('[role="dialog"]')
          ? active
          : null;
    }
    const prefill = inBar ? null : selectionText(active);
    const s = get();
    const query = prefill ?? s.query;
    const fresh = !s.open || s.reviewId !== reviewId || query !== s.query;
    set({
      open: true,
      reviewId,
      query,
      focusSeq: s.focusSeq + 1,
    });
    if (fresh) {
      anchor = readingPlace(pageOf(reviewId).order);
      autoJump = true;
      void run({ jump: false });
    }
    return true;
  },

  close: (opts) => {
    gen += 1;
    if (debounce) clearTimeout(debounce);
    debounce = null;
    cancelReveal?.();
    cancelReveal = null;
    autoJump = false;
    set({
      open: false,
      matches: [],
      current: -1,
      capped: false,
      searching: false,
      progress: NO_PROGRESS,
      skipped: [],
    });
    const back = returnFocus;
    returnFocus = null;
    if (opts?.restoreFocus !== false && back?.isConnected) {
      back.focus({ preventScroll: true });
    }
  },

  setQuery: (query) => {
    if (query === get().query) return;
    set({ query });
    schedule(FIND_DEBOUNCE_MS, true);
  },

  setOption: (key, value) => {
    set({ options: { ...get().options, [key]: value } });
    schedule(0, true);
  },

  setScope: (scope) => {
    set({ scope });
    schedule(0, true);
  },

  next: () => step(1),
  prev: () => step(-1),
}));

/** Search again after `ms`; `jump`: from the reading position (or the match). */
function schedule(ms: number, jump: boolean) {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    debounce = null;
    void run({ jump });
  }, ms);
}

/** Where a match sits on the page. */
function placeOf(m: FindMatch, order: readonly string[]): PagePlace {
  return { card: Math.max(0, order.indexOf(m.card)), row: m.row };
}

async function run({ jump }: { jump: boolean }) {
  const my = ++gen;
  const alive = () => my === gen && useFindStore.getState().open;
  const s = useFindStore.getState();
  const reviewId = s.reviewId;
  if (!s.open || !reviewId) return;
  const { re, error } = compileFind(s.query, s.options);
  if (!re) {
    autoJump = false;
    useFindStore.setState({
      re: null,
      error,
      matches: [],
      current: -1,
      capped: false,
      searching: false,
      progress: NO_PROGRESS,
      skipped: [],
    });
    return;
  }
  const scope = s.scope;
  const page = pageOf(reviewId);
  if (jump) {
    const cur = s.matches[s.current];
    anchor = cur ? placeOf(cur, page.order) : readingPlace(page.order);
    autoJump = true;
  }

  // The files whose code is searched, in page order, and the rest.
  const byPath = new Map((page.files ?? []).map((f) => [f.path, f]));
  const shown = page.order
    .filter((c) => c !== CONVERSATION_CARD)
    .map((c) => byPath.get(c))
    .filter((f): f is FileEntry => !!f);
  const skipped: SkippedFile[] = [];
  const searchable: { file: FileEntry; key: string }[] = [];
  for (const f of shown) {
    const why = skipReason(f, waitsForLoadAnyway(f));
    const key = preparedKey(reviewId, f);
    if (why) skipped.push({ path: f.path, reason: why });
    else if (key) searchable.push({ file: f, key });
  }
  const wantsCode = scope !== "comments";
  const total = wantsCode ? searchable.length : 0;

  // Results per card; `codeDone`: files whose code is in (or failed).
  const results = new Map<string, FindMatch[]>();
  const codeDone = new Set<string>();
  const searchCard = (card: string) => {
    if (card === CONVERSATION_CARD) {
      results.set(card, scope === "code" ? [] : searchConversation(page, re));
      return;
    }
    const f = byPath.get(card);
    if (!f) return;
    const key = preparedKey(reviewId, f);
    const code = key ? (prepared.get(key) ?? null) : null;
    if (code) codeDone.add(card);
    results.set(card, searchFile(page, f, code, re, scope));
  };

  useFindStore.setState({
    re,
    error: null,
    searching: true,
    skipped,
    progress: { done: 0, total },
  });

  // 1. Everything already in, in slices. Comment text is parsed once
  // (cached) and first, so a long conversation never blocks the page.
  let started = now();
  if (scope !== "code") {
    const bodies = [
      ...page.threads.flatMap((t) => t.notes.map((n) => n.body)),
      ...page.findings.flatMap((f) => (f.body === null ? [] : [f.body])),
    ];
    for (const body of bodies) {
      markdownText(body);
      if (now() - started > SLICE_MS) {
        await yieldToPage();
        if (!alive()) return;
        started = now();
      }
    }
  }
  for (const card of page.order) {
    searchCard(card);
    if (now() - started > SLICE_MS) {
      await yieldToPage();
      if (!alive()) return;
      started = now();
    }
  }
  if (!alive()) return;
  const doneCount = () =>
    wantsCode ? searchable.filter((x) => codeDone.has(x.file.path)).length : 0;
  flush(page, results, codeDone, searchable, false, doneCount());

  // 2. Load the rest, from the reading position on, then the files above.
  if (wantsCode) {
    const from = anchor?.card ?? 0;
    const index = new Map(page.order.map((c, i) => [c, i]));
    const todo = searchable
      .filter((x) => !codeDone.has(x.file.path))
      .sort((x, y) => {
        const a = index.get(x.file.path) ?? 0;
        const b = index.get(y.file.path) ?? 0;
        return (a < from ? 1 : 0) - (b < from ? 1 : 0) || a - b;
      });
    let timer: ReturnType<typeof setTimeout> | null = null;
    const dirty = () => {
      timer ??= setTimeout(() => {
        timer = null;
        if (alive()) {
          flush(page, results, codeDone, searchable, false, doneCount());
        }
      }, FLUSH_MS);
    };
    let next = 0;
    const worker = async () => {
      while (next < todo.length && alive()) {
        const { file, key } = todo[next++];
        const hideWhitespace = useLayoutStore.getState().hideWhitespace;
        try {
          let code = prepared.get(key);
          if (!code) {
            const files = useCodeViewStore.getState().byReview[reviewId]?.files;
            if (!files) throw new Error("the file list is gone");
            const [a, b] = await loadFileSides(files, file, "soon");
            if (!alive()) return;
            // Its own task: a large file's diff never blocks the page long.
            await yieldToPage();
            code = prepareCode(a, b, await chunksOf(a, b, hideWhitespace));
            remember(key, code);
          }
          if (!alive()) return;
          codeDone.add(file.path);
          results.set(file.path, searchFile(page, file, code, re, scope));
        } catch (e) {
          if (!alive()) return;
          codeDone.add(file.path);
          const message =
            typeof e === "object" && e && "message" in e
              ? String((e as { message: unknown }).message)
              : String(e);
          skipped.push({
            path: file.path,
            reason: `could not load: ${message}`,
          });
        }
        dirty();
      }
    };
    await Promise.all(
      Array.from({ length: Math.min(PARALLEL_LOADS, todo.length) }, worker),
    );
    if (timer) clearTimeout(timer);
  }
  if (!alive()) return;
  useFindStore.setState({ skipped: [...skipped] });
  flush(page, results, codeDone, searchable, true, doneCount());
}

/**
 * Publish the results: in page order, the current match kept by identity;
 * after a query change, the first match from `anchor` once the files up
 * to it are searched (then revealed).
 */
function flush(
  page: ReviewPage,
  results: ReadonlyMap<string, FindMatch[]>,
  codeDone: ReadonlySet<string>,
  searchable: readonly { file: FileEntry }[],
  done: boolean,
  doneCount: number,
) {
  const s = useFindStore.getState();
  const matches: FindMatch[] = [];
  let capped = false;
  for (const card of page.order) {
    const list = results.get(card);
    if (!list) continue;
    if (matches.length + list.length > MAX_MATCHES) {
      matches.push(...list.slice(0, MAX_MATCHES - matches.length));
      capped = true;
      break;
    }
    for (const m of list) matches.push(m);
  }
  const prev = s.matches[s.current];
  const prevKey = prev ? matchKey(prev) : null;
  let current = prevKey
    ? matches.findIndex((m) => matchKey(m) === prevKey)
    : -1;
  let reveal = false;
  if (autoJump) {
    const want = firstFrom(matches, page.order, anchor);
    const wanted = matches[want];
    let settled = done;
    if (!settled && wanted) {
      // Every searchable file from the anchor to the match is in.
      const from = anchor?.card ?? 0;
      const to = page.order.indexOf(wanted.card);
      settled =
        to >= from &&
        searchable.every((x) => {
          const i = page.order.indexOf(x.file.path);
          return i < from || i > to || codeDone.has(x.file.path);
        });
    }
    if (settled) {
      autoJump = false;
      if (wanted) {
        reveal = matchKey(wanted) !== prevKey;
        current = want;
      } else {
        current = -1;
      }
    }
  }
  useFindStore.setState({
    matches,
    current,
    capped,
    searching: !done,
    progress: { done: doneCount, total: s.progress.total },
  });
  if (reveal) revealCurrent();
}

/** Next (+1) / previous (-1), wrapping. */
function step(delta: 1 | -1): boolean {
  const s = useFindStore.getState();
  if (!s.open || !s.reviewId || s.matches.length === 0) return false;
  autoJump = false;
  const n = s.matches.length;
  let i: number;
  let wrapped = false;
  if (s.current < 0) {
    const order = pageOf(s.reviewId).order;
    const from = firstFrom(s.matches, order, readingPlace(order));
    i = delta > 0 ? from : from - 1;
    if (i < 0) i = n - 1;
  } else {
    i = s.current + delta;
    if (i >= n || i < 0) {
      wrapped = true;
      i = (i + n) % n;
    }
  }
  useFindStore.setState({
    current: i,
    wrapSeq: wrapped ? s.wrapSeq + 1 : s.wrapSeq,
  });
  revealCurrent();
  return true;
}

/** The focus item a match belongs to. */
function itemOf(m: FindMatch, reviewId: ReviewId): FocusItem | null {
  const since =
    useCodeViewStore.getState().byReview[reviewId]?.files?.sinceLastReview ??
    false;
  if (m.kind === "code") {
    return {
      kind: "line",
      path: m.card,
      side: m.side ?? "new",
      line: m.line ?? 1,
    };
  }
  if (m.threadId) {
    const t = useThreadStore
      .getState()
      .byReview[reviewId]?.threads.find((x) => x.threadId === m.threadId);
    return t ? threadItem(t, m.card, since) : null;
  }
  if (m.findingId) {
    const f = useAiStore
      .getState()
      .byReview[reviewId]?.findings.find((x) => x.id === m.findingId);
    return f ? findingItem(f, since) : null;
  }
  return null;
}

/** Focus and reveal the current match. */
function revealCurrent() {
  const s = useFindStore.getState();
  const m = s.matches[s.current];
  const reviewId = s.reviewId;
  if (!m || !reviewId) return;
  anchor = placeOf(m, pageOf(reviewId).order);
  cancelReveal?.();
  // Navigation: the page's scroll anchoring stands aside while a folded
  // thread or card opens on the way.
  pauseScrollAnchor();
  const item = itemOf(m, reviewId);
  if (item) useFocusStore.getState().focusFound(reviewId, item);
  if (item?.kind === "line") {
    // The comment key (c) comments on the found line.
    const key = matchKey(m);
    void lineTarget(reviewId, item.path, item.side, item.line).then(
      (target) => {
        const now = useFindStore.getState();
        const cur = now.matches[now.current];
        if (!cur || matchKey(cur) !== key) return;
        useCommentStore.getState().setFocus({
          reviewId,
          path: item.path,
          side: item.side,
          line: item.line,
          target,
        });
      },
    );
  }
  cancelReveal = revealMatch(
    m,
    pageMatchEnv(m, (path) => prefetchFile(reviewId, path)),
  );
}

// -- Following the page --------------------------------------------------------------

/** The highlighting follows the bar (CodeMirror through the bridge). */
useFindStore.subscribe((s) => {
  const m = s.open ? s.matches[s.current] : undefined;
  publishFindHighlight({
    re: s.open ? s.re : null,
    scope: s.scope,
    current: m
      ? { unit: m.unit, line: m.line, start: m.start, end: m.end, side: m.side }
      : null,
  });
});

/** What the page shows changed (threads re-read, files, findings): again. */
function refresh(reviewId: ReviewId | null) {
  const s = useFindStore.getState();
  if (s.open && s.reviewId === reviewId && s.re) schedule(300, false);
}

useThreadStore.subscribe((s, prev) => {
  const id = useFindStore.getState().reviewId;
  if (id && s.byReview[id]?.threads !== prev.byReview[id]?.threads) refresh(id);
});

useCodeViewStore.subscribe((s, prev) => {
  const id = useFindStore.getState().reviewId;
  if (!id) return;
  const a = s.byReview[id];
  const b = prev.byReview[id];
  if (a?.files !== b?.files || a?.order !== b?.order) refresh(id);
});

// Whitespace hidden or shown: the diff, so the removed lines, change.
useLayoutStore.subscribe((s, prev) => {
  if (s.hideWhitespace !== prev.hideWhitespace) {
    refresh(useFindStore.getState().reviewId);
  }
});

useAiStore.subscribe((s, prev) => {
  const id = useFindStore.getState().reviewId;
  if (!id) return;
  if (
    s.byReview[id]?.findings !== prev.byReview[id]?.findings ||
    s.showDismissed !== prev.showDismissed ||
    s.runs[id]?.enabled !== prev.runs[id]?.enabled
  ) {
    refresh(id);
  }
});

useReviewStore.subscribe((s, prev) => {
  if (s.selectedId !== prev.selectedId && useFindStore.getState().open) {
    useFindStore.getState().close({ restoreFocus: false });
  }
});

// Mod+F inside an editor (its own key handling runs first).
setFindOpener((target) => useFindStore.getState().openBar({ target }));

// -- Hooks ------------------------------------------------------------------------------

/**
 * The find marks for a comment's unit (`Markdown`, `FindText`): the
 * query's expression and which of the unit's matches is current; null
 * while the bar shows nothing for comments.
 */
export function useFindMarks(
  unit: string | undefined,
): FindMarksOptions | null {
  const re = useFindStore((s) =>
    unit && s.open && s.scope !== "code" ? s.re : null,
  );
  const current = useFindStore((s) => {
    if (!unit) return null;
    const m = s.matches[s.current];
    return m && m.unit === unit ? m.ordinal : null;
  });
  return useMemo(() => (re ? { re, current } : null), [re, current]);
}

/** Whether the current match is in `unit` (a folded long note opens). */
export function useFindCurrentIn(unit: string | undefined): boolean {
  return useFindStore((s) => {
    const m = s.matches[s.current];
    return !!unit && !!m && m.unit === unit;
  });
}
