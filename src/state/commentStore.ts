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
 * posts the edit's text (an emptied edit is left out). Cancel (or Esc) on a
 * composer or reply with text keeps it recoverable: the toast's Undo brings
 * it back.
 *
 * Also caches each file's commentable lines (`commentable_lines`) per
 * (review, head, path), like the file contents.
 */

import { create } from "zustand";

import {
  commentableLines,
  postComments,
  replyToThread,
  toIpcError,
} from "../ipc/commands";
import type {
  CommentableLines,
  NewComment,
  PostError,
  ReviewId,
  Side,
  ThreadId,
} from "../ipc/types";
import { clampRange, type LineRange } from "../lib/commentRanges";
import { FOCUS, requestFocus } from "../lib/focusRequests";
import { useHistoryStore } from "./historyStore";
import {
  toastError,
  toastInfo,
  toastSuccess,
  useToastStore,
} from "./toastStore";

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
}

/** The open composer (one at a time): a new comment or a draft's edit. */
export interface Composer extends CommentTarget {
  body: string;
  /** Editing this draft instead of writing a new comment. */
  draftId: string | null;
  busy: boolean;
  error: PostError | null;
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
  /** Shift-click: extend the open composer's range to `line`, in its hunk. */
  extendComposer: (lines: CommentableLines | null, line: number) => boolean;
  setComposerBody: (body: string) => void;
  closeComposer: () => void;
  /** "Add to review" (or "Save" for a draft's edit): no write. */
  saveComposer: () => void;
  /** "Comment now": post just this comment. */
  commentNow: () => Promise<void>;

  editDraft: (reviewId: ReviewId, id: string) => void;
  removeDraft: (reviewId: ReviewId, id: string) => void;
  /** "Post all": every draft of the review, one by one. */
  postAll: (reviewId: ReviewId) => Promise<void>;
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
      setDrafts(
        reviewId,
        draftsOf(reviewId)
          .filter((d) => !posted.has(d.id))
          .map((d) =>
            failed.has(d.id) ? { ...d, error: failed.get(d.id)! } : d,
          ),
      );
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

    extendComposer: (lines, line) => {
      const cur = get().composer;
      if (!cur || cur.busy || cur.draftId) return false;
      const range: LineRange | null = clampRange(
        lines,
        cur.side,
        cur.anchor,
        line,
      );
      if (!range) return false;
      patchComposer(range);
      return true;
    },

    setComposerBody: (body) => patchComposer({ body, error: null }),

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
      if (c.draftId && list.some((d) => d.id === c.draftId)) {
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
      if (get().posting[reviewId]) return;
      // A draft's open edit is what the user sees of it: post that text, or
      // leave the draft out when the edit was emptied.
      const c = get().composer;
      const editing =
        c && c.reviewId === reviewId && c.draftId && !c.busy ? c : null;
      if (editing && editing.body.trim() !== "") {
        setDrafts(
          reviewId,
          draftsOf(reviewId).map((d) =>
            d.id === editing.draftId
              ? { ...d, body: editing.body, error: null }
              : d,
          ),
        );
      }
      const drafts = draftsOf(reviewId).filter(
        (d) => !(editing?.draftId === d.id && editing.body.trim() === ""),
      );
      if (drafts.length === 0) return;
      const r = await post(reviewId, drafts);
      // The edit it posted is done with; text typed into it meanwhile stays
      // open (Save keeps it as a new draft).
      const after = get().composer;
      if (
        editing &&
        after?.draftId === editing.draftId &&
        !draftsOf(reviewId).some((d) => d.id === editing.draftId) &&
        after.body === editing.body
      ) {
        set({ composer: null });
      }
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
      if (get().replies[threadId]) return;
      requestFocus(FOCUS.reply(threadId));
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

// -- Commentable lines ------------------------------------------------------

const commentable = new Map<string, Promise<CommentableLines>>();

/**
 * A file's commentable lines, fetched once per (review, head, path); a
 * failed fetch is forgotten so the next view retries.
 */
export function loadCommentable(
  reviewId: ReviewId,
  headSha: string,
  path: string,
): Promise<CommentableLines> {
  const key = `${reviewId}|${headSha}|${path}`;
  let p = commentable.get(key);
  if (!p) {
    p = commentableLines(reviewId, path);
    commentable.set(key, p);
    p.catch(() => commentable.delete(key));
  }
  return p;
}

/** Tests: forget the cached commentable lines. */
export function clearCommentableCache() {
  commentable.clear();
}
