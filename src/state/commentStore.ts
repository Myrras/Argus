/**
 * Writing comments: the pending drafts per review, the one open composer,
 * the open reply composers, and the posting calls. Nothing here posts on its
 * own: every write (`commentNow`, `postAll`, `retryDraft`, `sendReply`) runs
 * from a button or shortcut handler, never from an effect or a timer, and a
 * draft never sends itself.
 *
 * Drafts are kept per review in `localStorage` (`argus.drafts.v1.<id>`), so
 * a reload or a crash loses nothing; posted drafts are removed, failed ones
 * keep their text and the platform's reason. A posted comment comes back as a
 * real thread through `review:changed` (the shell emits it after a write).
 *
 * What posts is what the user sees: "Post all" with a draft's edit open
 * posts the edit's text (an emptied edit is left out, with a notice). The
 * edit's text reaches the stored draft only by Save, so a refused post
 * keeps the draft's own text. While a review posts, Save on one of its
 * drafts' edits waits; a posted draft whose stored text changed meanwhile
 * stays as a new draft. Cancel (or Esc) on a composer or reply with text
 * keeps it recoverable: the toast's Undo brings it back.
 *
 * Also caches each file's commentable lines (`commentable_lines`) per
 * (review, head, review base, path), like the file contents.
 */

import { create } from "zustand";

import {
  commentableLinesBatch,
  postComments,
  replyToThread,
  toIpcError,
} from "../ipc/commands";
import type {
  CommentableLines,
  NewComment,
  OutsideDiff,
  PostError,
  ReviewId,
  Side,
  ThreadId,
} from "../ipc/types";
import { clampRange, type LineRange } from "../lib/commentRanges";
import { FOCUS, requestFocus } from "../lib/focusRequests";
import { createRequestQueue } from "../lib/requestQueue";
import { useCodeViewStore } from "./codeViewStore";
import { useHistoryStore } from "./historyStore";
import {
  toastError,
  toastInfo,
  toastSuccess,
  useToastStore,
} from "./toastStore";

/** What "Post all" did: counts, or why nothing could be tried. */
export interface PostAllOutcome {
  posted: number;
  failed: number;
  error: PostError | null;
}

/** A pending comment: queued with "Add to review", sent with "Post all". */
export interface Draft {
  id: string;
  path: string;
  side: Side;
  line: number;
  startLine: number | null;
  body: string;
  /** The head the lines were read at (what the code view showed). */
  headSha: string;
  /** The diff's left side, for an old-side line; null in "since" mode. */
  baseSha: string | null;
  /**
   * On unchanged lines outside the diff: what the platform makes of it
   * (`native` line comment, `fallback` file comment); absent or null inside.
   */
  outside?: OutsideDiff | null;
  createdAt: string;
  /** Why the last post of it failed. */
  error: PostError | null;
}

/** Where a new comment goes. */
export interface CommentTarget {
  reviewId: ReviewId;
  path: string;
  side: Side;
  /** The line the selection started on (shift-click extends from it). */
  anchor: number;
  line: number;
  startLine: number | null;
  headSha: string;
  baseSha: string | null;
  /** As `Draft.outside`. */
  outside?: OutsideDiff | null;
}

/** The open composer (one at a time): a new comment or a draft's edit. */
export interface Composer extends CommentTarget {
  body: string;
  /** Editing this draft instead of writing a new comment. */
  draftId: string | null;
  busy: boolean;
  error: PostError | null;
  /**
   * The lines (head side) the last inserted suggestion was written for, so
   * a range changed since can be flagged (GitHub's fence has no range of
   * its own). Not kept with a draft.
   */
  suggestedFor?: { start: number; end: number } | null;
}

/** The line last clicked in a diff: what the Comment-on-line key uses. */
export interface FocusedLine {
  reviewId: ReviewId;
  path: string;
  side: Side;
  line: number;
  /** Where a comment on it goes; null when the line is not commentable. */
  target: CommentTarget | null;
}

export interface ReplyState {
  body: string;
  busy: boolean;
  error: PostError | null;
  /** Bumped when Reply is used on an open composer: scroll to it again. */
  revealSeq?: number;
}

interface CommentState {
  drafts: Record<ReviewId, Draft[]>;
  composer: Composer | null;
  replies: Record<ThreadId, ReplyState>;
  /** A "Post all" or a retry is running for the review. */
  posting: Record<ReviewId, boolean>;
  focus: FocusedLine | null;

  setFocus: (focus: FocusedLine | null) => void;
  /**
   * The Comment-on-line key: open the composer on the focused line of
   * `reviewId`; false when there is none or it is not commentable.
   */
  commentOnFocus: (reviewId: ReviewId) => boolean;
  /**
   * Open the composer on `target`. An open composer with text is kept (the
   * user finishes or cancels it first); false then.
   */
  openComposer: (
    target: Omit<CommentTarget, "anchor"> & { anchor?: number },
  ) => boolean;
  /**
   * Shift-click: extend the open composer's range to `line`, in its region
   * (one hunk, or one stretch outside the diff; `docLines` bounds the last).
   */
  extendComposer: (
    lines: CommentableLines | null,
    line: number,
    docLines?: number,
  ) => boolean;
  setComposerBody: (body: string) => void;
  /** A suggestion was inserted for these lines. */
  setSuggestedFor: (range: { start: number; end: number }) => void;
  /**
   * Put a new comment back on `range` (the lines its suggestion was written
   * for); only within its region, which a range it had always is.
   */
  setComposerRange: (range: { start: number; end: number }) => void;
  closeComposer: () => void;
  /** "Add to review" (or "Save" for a draft's edit): no write. */
  saveComposer: () => void;
  /** "Comment now": post just this comment. */
  commentNow: () => Promise<void>;

  editDraft: (reviewId: ReviewId, id: string) => void;
  removeDraft: (reviewId: ReviewId, id: string) => void;
  /** "Post all": every draft of the review, one by one. */
  /**
   * Resolves to what happened (null when nothing was posted: none to post,
   * or the review is posting already); Submit review aborts on a failure.
   */
  postAll: (reviewId: ReviewId) => Promise<PostAllOutcome | null>;
  /** Post one (failed) draft again. */
  retryDraft: (reviewId: ReviewId, id: string) => Promise<void>;

  openReply: (threadId: ThreadId) => void;
  setReplyBody: (threadId: ThreadId, body: string) => void;
  closeReply: (threadId: ThreadId) => void;
  sendReply: (reviewId: ReviewId, threadId: ThreadId) => Promise<void>;
}

// -- Persistence ------------------------------------------------------------

const PREFIX = "argus.drafts.v1.";

function isDraft(v: unknown): v is Draft {
  const d = v as Draft;
  return (
    typeof d === "object" &&
    d !== null &&
    typeof d.id === "string" &&
    typeof d.path === "string" &&
    (d.side === "old" || d.side === "new") &&
    typeof d.line === "number" &&
    typeof d.body === "string" &&
    typeof d.headSha === "string"
  );
}

/** Every review's drafts from `localStorage`; unreadable entries are skipped. */
export function readStoredDrafts(): Record<ReviewId, Draft[]> {
  const out: Record<ReviewId, Draft[]> = {};
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key?.startsWith(PREFIX)) continue;
      try {
        const list: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
        const drafts = Array.isArray(list) ? list.filter(isDraft) : [];
        if (drafts.length) {
          out[key.slice(PREFIX.length)] = drafts.map((d) => ({
            ...d,
            startLine: d.startLine ?? null,
            baseSha: d.baseSha ?? null,
            error: d.error ?? null,
          }));
        }
      } catch {
        // A corrupt entry: leave it, show nothing for it.
      }
    }
  } catch {
    // No storage (private mode): drafts live in memory only.
  }
  return out;
}

/** The "not saved" warning shows once a session. */
let warnedPersist = false;

function persist(reviewId: ReviewId, drafts: Draft[]) {
  try {
    if (drafts.length) {
      localStorage.setItem(PREFIX + reviewId, JSON.stringify(drafts));
    } else {
      localStorage.removeItem(PREFIX + reviewId);
    }
  } catch {
    // Storage full or blocked: the drafts stay in memory, and the user
    // should know a restart would lose them.
    if (!warnedPersist) {
      warnedPersist = true;
      toastError(
        "Pending comments not saved",
        "They stay until Argus closes; post them before you quit.",
      );
    }
  }
}

// -- Helpers ------------------------------------------------------------------

let seq = 0;
function draftId(): string {
  seq += 1;
  return `draft-${Date.now().toString(36)}-${seq}`;
}

function toNewComment(
  d: Omit<Draft, "id" | "createdAt" | "error">,
): NewComment {
  return {
    path: d.path,
    line: d.line,
    side: d.side,
    startLine: d.startLine,
    body: d.body,
    headSha: d.headSha,
    baseSha: d.baseSha,
  };
}

function asPostError(e: unknown): PostError {
  const ipc = toIpcError(e);
  return { kind: ipc.kind as PostError["kind"], message: ipc.message };
}

const plural = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;

/** The open composer when it edits one of `reviewId`'s drafts. */
function editOf(
  s: Pick<CommentState, "composer">,
  reviewId: ReviewId,
): (Composer & { draftId: string }) | null {
  const c = s.composer;
  return c && c.reviewId === reviewId && c.draftId && !c.busy
    ? (c as Composer & { draftId: string })
    : null;
}

/**
 * What "Post all" sends for the review: each draft as the user sees it, an
 * open edit's text in place of its draft's, and a draft whose open edit was
 * emptied left out.
 */
export function postableDrafts(
  s: Pick<CommentState, "drafts" | "composer">,
  reviewId: ReviewId,
): Draft[] {
  const list = s.drafts[reviewId] ?? [];
  const edit = editOf(s, reviewId);
  if (!edit) return list;
  return list.flatMap((d) => {
    if (d.id !== edit.draftId) return [d];
    return edit.body.trim() === "" ? [] : [{ ...d, body: edit.body }];
  });
}

// -- Store ----------------------------------------------------------------------

export const useCommentStore = create<CommentState>((set, get) => {
  const setDrafts = (reviewId: ReviewId, drafts: Draft[]) => {
    persist(reviewId, drafts);
    set((s) => {
      const next = { ...s.drafts };
      if (drafts.length) next[reviewId] = drafts;
      else delete next[reviewId];
      return { drafts: next };
    });
  };
  const draftsOf = (reviewId: ReviewId) => get().drafts[reviewId] ?? [];
  const patchComposer = (p: Partial<Composer>) =>
    set((s) => (s.composer ? { composer: { ...s.composer, ...p } } : {}));
  const patchReply = (threadId: ThreadId, p: Partial<ReplyState>) =>
    set((s) => {
      const cur = s.replies[threadId];
      return cur
        ? { replies: { ...s.replies, [threadId]: { ...cur, ...p } } }
        : {};
    });

  /** Post `drafts` (in order); apply each result; returns the counts. */
  const post = async (reviewId: ReviewId, drafts: Draft[]) => {
    const ids = drafts.map((d) => d.id);
    // The stored text at send time: a posted draft whose text changed since
    // was not posted with that text, so it stays.
    const storedBody = new Map(draftsOf(reviewId).map((d) => [d.id, d.body]));
    const sentBody = new Map(drafts.map((d) => [d.id, d.body]));
    set((s) => ({ posting: { ...s.posting, [reviewId]: true } }));
    try {
      const results = await postComments(reviewId, drafts.map(toNewComment));
      const posted = new Set<string>();
      const failed = new Map<string, PostError>();
      results.forEach((r, i) => {
        const id = ids[i];
        if (!id) return;
        if (r.status === "posted") posted.add(id);
        else failed.set(id, { kind: r.kind, message: r.message });
      });
      // By id: a draft edited or deleted meanwhile keeps that change.
      const renamed = new Map<string, string>();
      const next = draftsOf(reviewId).flatMap((d): Draft[] => {
        if (failed.has(d.id)) return [{ ...d, error: failed.get(d.id)! }];
        if (!posted.has(d.id)) return [d];
        if (d.body === storedBody.get(d.id)) return [];
        const id = draftId();
        renamed.set(d.id, id);
        return [{ ...d, id, error: null }];
      });
      setDrafts(reviewId, next);
      // An edit of a draft that went out, still holding what was sent (opened
      // before or during the post), is done with: Save would post it twice.
      // Text typed since stays open (Save keeps it as a new draft).
      const open = get().composer;
      if (
        open?.draftId &&
        open.reviewId === reviewId &&
        posted.has(open.draftId) &&
        !renamed.has(open.draftId) &&
        open.body === sentBody.get(open.draftId)
      ) {
        set({ composer: null });
      }
      if (renamed.size > 0) {
        // An edit still open on one follows it to its new id.
        const c = get().composer;
        const to = c?.draftId ? renamed.get(c.draftId) : undefined;
        if (to) patchComposer({ draftId: to });
        toastInfo(
          renamed.size === 1
            ? "Saved as a new pending comment"
            : `Saved ${renamed.size} as new pending comments`,
          "Their text changed while posting; the earlier text was posted.",
        );
      }
      return { posted: posted.size, failed: failed.size, error: null };
    } catch (e) {
      // Nothing was tried: every draft stays as it was.
      return { posted: 0, failed: drafts.length, error: asPostError(e) };
    } finally {
      set((s) => {
        const next = { ...s.posting };
        delete next[reviewId];
        return { posting: next };
      });
    }
  };

  return {
    drafts: readStoredDrafts(),
    composer: null,
    replies: {},
    posting: {},
    focus: null,

    setFocus: (focus) => set({ focus }),

    commentOnFocus: (reviewId) => {
      const f = get().focus;
      if (!f || f.reviewId !== reviewId || !f.target) return false;
      return get().openComposer(f.target);
    },

    openComposer: (target) => {
      const cur = get().composer;
      if (cur && cur.body.trim() !== "" && !cur.busy) {
        const same =
          cur.reviewId === target.reviewId &&
          cur.path === target.path &&
          cur.side === target.side &&
          cur.line === target.line &&
          cur.startLine === target.startLine;
        if (!same) {
          toastInfo("Finish or cancel the open comment first");
        }
        return false;
      }
      if (cur?.busy) return false;
      requestFocus(FOCUS.composer);
      set({
        composer: {
          ...target,
          anchor: target.anchor ?? target.startLine ?? target.line,
          body: "",
          draftId: null,
          busy: false,
          error: null,
        },
      });
      return true;
    },

    extendComposer: (lines, line, docLines) => {
      const cur = get().composer;
      if (!cur || cur.busy || cur.draftId) return false;
      const range: LineRange | null = clampRange(
        lines,
        cur.side,
        cur.anchor,
        line,
        docLines,
      );
      if (!range) return false;
      patchComposer(range);
      return true;
    },

    setComposerBody: (body) => patchComposer({ body, error: null }),

    setSuggestedFor: (range) => patchComposer({ suggestedFor: range }),

    setComposerRange: (range) => {
      const cur = get().composer;
      if (!cur || cur.busy || cur.draftId) return;
      patchComposer({
        line: range.end,
        startLine: range.start < range.end ? range.start : null,
      });
    },

    closeComposer: () => {
      const c = get().composer;
      if (!c || c.busy) return;
      set({ composer: null });
      const kept = c.draftId
        ? draftsOf(c.reviewId).find((d) => d.id === c.draftId)?.body
        : undefined;
      if (c.body.trim() === "" || c.body === kept) return;
      // Cancel is one key away from typing: keep the text one Undo away.
      useToastStore.getState().push({
        kind: "info",
        message: "Comment discarded",
        action: {
          label: "Undo",
          onClick: () => {
            const cur = get().composer;
            if (cur && (cur.busy || cur.body.trim() !== "")) {
              toastInfo("Finish or cancel the open comment first");
              return;
            }
            // A draft posted or deleted since comes back as a new comment.
            requestFocus(FOCUS.composer);
            const still =
              c.draftId && draftsOf(c.reviewId).some((d) => d.id === c.draftId);
            set({
              composer: {
                ...c,
                draftId: still ? c.draftId : null,
                busy: false,
                error: null,
              },
            });
          },
        },
      });
    },

    saveComposer: () => {
      const c = get().composer;
      if (!c || c.busy || c.body.trim() === "") return;
      const list = draftsOf(c.reviewId);
      const own = !!c.draftId && list.some((d) => d.id === c.draftId);
      if (own && get().posting[c.reviewId]) {
        // The draft may be on its way with its old text; saving now would
        // land in a draft about to be dropped. The text stays here.
        toastInfo("Wait for the post to finish", "Then save; your text stays.");
        return;
      }
      if (own) {
        setDrafts(
          c.reviewId,
          list.map((d) =>
            d.id === c.draftId ? { ...d, body: c.body, error: null } : d,
          ),
        );
      } else {
        if (c.draftId) {
          // Its draft was posted while the edit stayed open: keep the newer
          // text rather than drop it.
          toastInfo(
            "Saved as a new pending comment",
            "The one you were editing was already posted.",
          );
        }
        setDrafts(c.reviewId, [
          ...list,
          {
            id: draftId(),
            path: c.path,
            side: c.side,
            line: c.line,
            startLine: c.startLine,
            body: c.body,
            headSha: c.headSha,
            baseSha: c.baseSha,
            outside: c.outside ?? null,
            createdAt: new Date().toISOString(),
            error: null,
          },
        ]);
      }
      set({ composer: null });
    },

    commentNow: async () => {
      const c = get().composer;
      if (!c || c.busy || c.draftId || c.body.trim() === "") return;
      patchComposer({ busy: true, error: null });
      try {
        const [result] = await postComments(c.reviewId, [toNewComment(c)]);
        if (result?.status === "posted") {
          set({ composer: null });
          toastSuccess("Comment posted");
        } else {
          const error: PostError = result
            ? { kind: result.kind, message: result.message }
            : { kind: "provider", message: "The platform sent no result." };
          patchComposer({ busy: false, error });
          toastError("Could not post the comment", error.message);
        }
      } catch (e) {
        const error = asPostError(e);
        patchComposer({ busy: false, error });
        toastError("Could not post the comment", error.message);
      }
    },

    editDraft: (reviewId, id) => {
      const d = draftsOf(reviewId).find((x) => x.id === id);
      const cur = get().composer;
      if (!d || (cur && (cur.busy || cur.body.trim() !== ""))) {
        if (d && cur) toastInfo("Finish or cancel the open comment first");
        return;
      }
      requestFocus(FOCUS.composer);
      set({
        composer: {
          reviewId,
          path: d.path,
          side: d.side,
          anchor: d.startLine ?? d.line,
          line: d.line,
          startLine: d.startLine,
          headSha: d.headSha,
          baseSha: d.baseSha,
          outside: d.outside ?? null,
          body: d.body,
          draftId: d.id,
          busy: false,
          error: null,
        },
      });
    },

    removeDraft: (reviewId, id) => {
      if (get().posting[reviewId]) return;
      setDrafts(
        reviewId,
        draftsOf(reviewId).filter((d) => d.id !== id),
      );
      const c = get().composer;
      if (c?.draftId === id) set({ composer: null });
    },

    postAll: async (reviewId) => {
      if (get().posting[reviewId]) return null;
      // A draft's open edit is what the user sees of it: post that text
      // (the stored draft keeps its own until it posts), or leave the draft
      // out when the edit was emptied.
      const editing = editOf(get(), reviewId);
      const drafts = postableDrafts(get(), reviewId);
      if (
        editing &&
        editing.body.trim() === "" &&
        draftsOf(reviewId).some((d) => d.id === editing.draftId)
      ) {
        toastInfo(
          "1 comment with empty text not posted",
          "Write something in its edit or cancel it.",
        );
      }
      if (drafts.length === 0) return null;
      const r = await post(reviewId, drafts);
      if (r.error) {
        toastError("Could not post the comments", r.error.message);
      } else if (r.failed === 0) {
        toastSuccess(`Posted ${plural(r.posted, "comment")}`);
      } else {
        toastError(
          `${r.failed} of ${plural(drafts.length, "comment")} not posted`,
          "They stay as drafts with the reason; edit them or post again.",
        );
      }
      return r;
    },

    retryDraft: async (reviewId, id) => {
      const d = draftsOf(reviewId).find((x) => x.id === id);
      if (!d || get().posting[reviewId]) return;
      const r = await post(reviewId, [d]);
      if (r.error) {
        setDrafts(
          reviewId,
          draftsOf(reviewId).map((x) =>
            x.id === id ? { ...x, error: r.error } : x,
          ),
        );
        toastError("Could not post the comment", r.error.message);
      } else if (r.posted) {
        toastSuccess("Comment posted");
      } else {
        toastError(
          "Could not post the comment",
          draftsOf(reviewId).find((x) => x.id === id)?.error?.message,
        );
      }
    },

    openReply: (threadId) => {
      const open = get().replies[threadId];
      requestFocus(FOCUS.reply(threadId));
      if (open) {
        set((s) => ({
          replies: {
            ...s.replies,
            [threadId]: { ...open, revealSeq: (open.revealSeq ?? 0) + 1 },
          },
        }));
        return;
      }
      set((s) => ({
        replies: {
          ...s.replies,
          [threadId]: { body: "", busy: false, error: null },
        },
      }));
    },

    setReplyBody: (threadId, body) =>
      patchReply(threadId, { body, error: null }),

    closeReply: (threadId) => {
      const r = get().replies[threadId];
      if (!r || r.busy) return;
      set((s) => {
        const next = { ...s.replies };
        delete next[threadId];
        return { replies: next };
      });
      if (r.body.trim() === "") return;
      useToastStore.getState().push({
        kind: "info",
        message: "Reply discarded",
        action: {
          label: "Undo",
          onClick: () => {
            const cur = get().replies[threadId];
            if (cur && (cur.busy || cur.body.trim() !== "")) return;
            requestFocus(FOCUS.reply(threadId));
            set((s) => ({
              replies: {
                ...s.replies,
                [threadId]: { body: r.body, busy: false, error: null },
              },
            }));
            // Its thread unfolds so the reply shows.
            useHistoryStore.getState().setOpen(threadId, true);
          },
        },
      });
    },

    sendReply: async (reviewId, threadId) => {
      const r = get().replies[threadId];
      if (!r || r.busy || r.body.trim() === "") return;
      patchReply(threadId, { busy: true, error: null });
      try {
        await replyToThread(reviewId, threadId, r.body);
        set((s) => {
          const next = { ...s.replies };
          delete next[threadId];
          return { replies: next };
        });
        toastSuccess("Reply posted");
      } catch (e) {
        const error = asPostError(e);
        patchReply(threadId, { busy: false, error });
        toastError("Could not post the reply", error.message);
      }
    },
  };
});

/** The review's drafts (a stable empty list when none). */
const NO_DRAFTS: Draft[] = [];
export function useDrafts(reviewId: ReviewId): Draft[] {
  return useCommentStore((s) => s.drafts[reviewId] ?? NO_DRAFTS);
}

/** How many comments "Post all" would send now (`postableDrafts`). */
export function usePostableCount(reviewId: ReviewId): number {
  return useCommentStore((s) => postableDrafts(s, reviewId).length);
}

// -- Commentable lines ------------------------------------------------------

const commentable = new Map<string, Promise<CommentableLines>>();
/** (review, head, list base, lines base) pairs a file list was re-read for. */
const reread = new Set<string>();
/** The cards that load together ask in one call (`lib/requestQueue`). */
const commentableReads = createRequestQueue<string, CommentableLines>({
  send: (reviewId, paths) => commentableLinesBatch(reviewId, paths),
  maxBatch: 32,
  concurrency: 2,
});

/**
 * A file's commentable lines, fetched once per (review, head, review base,
 * path); `reviewBaseSha` is the left side of the review's own diff (see
 * `ReviewFiles`): a retarget, or a fetched clone resolving a stand-in,
 * moves it under an unchanged head, and the lines follow. A failed fetch is
 * forgotten so the next view retries.
 *
 * Lines at the same head but another base than `reviewBaseSha` mean the
 * file list is stale (the user fetched their clone since it was read, or
 * GitHub compared in one read and not the other): they are not kept under
 * this key, and the review's file list is read again, once per such pair so
 * bases that keep disagreeing cannot loop. The new list's base gives a new
 * key, which reads the lines afresh.
 */
export function loadCommentable(
  reviewId: ReviewId,
  headSha: string,
  reviewBaseSha: string,
  path: string,
): Promise<CommentableLines> {
  const key = `${reviewId}|${headSha}|${reviewBaseSha}|${path}`;
  let p = commentable.get(key);
  if (!p) {
    p = commentableReads.request(reviewId, key, path);
    commentable.set(key, p);
    p.then(
      (lines) => {
        if (lines.headSha !== headSha || lines.baseSha === reviewBaseSha) {
          return;
        }
        commentable.delete(key);
        const pair = `${reviewId}|${headSha}|${reviewBaseSha}|${lines.baseSha}`;
        if (reread.has(pair)) return;
        reread.add(pair);
        void useCodeViewStore.getState().load(reviewId);
      },
      () => commentable.delete(key),
    );
  }
  return p;
}

/** Forget the cached commentable lines: of one review, or of all (tests). */
export function clearCommentableCache(reviewId?: ReviewId) {
  if (reviewId === undefined) {
    commentable.clear();
    reread.clear();
    commentableReads.reset();
    return;
  }
  const prefix = `${reviewId}|`;
  for (const key of [...commentable.keys()]) {
    if (key.startsWith(prefix)) commentable.delete(key);
  }
  for (const key of [...reread]) {
    if (key.startsWith(prefix)) reread.delete(key);
  }
}
