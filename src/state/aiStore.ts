/**
 * The AI pass in the UI: each review's latest run (`ai_state`, kept current
 * by `ai:state`), and for the reviews the user opened their findings,
 * "look here yourself" checklist and per-thread verdicts. Those are loaded
 * when a review opens and re-read when a run finishes (`ai:state` done) or
 * the review changed (`review:changed`, also emitted after a post).
 *
 * Per finding the UI keeps whether it is selected for posting, the user's
 * edit of its text, an open editor and the reason its last post failed. Only
 * `postSelected` writes to a platform, and only from the "Post selected"
 * button: selecting, editing, dismissing, ticking the checklist and
 * overriding a verdict are local (the last three stored by Argus, never on
 * GitHub/GitLab). Findings the run set aside as already raised in the
 * review are listed apart (`skipped`) and never posted; "Show anyway" makes
 * one a normal finding (local only). It posts what each finding shows: an open editor's text,
 * else the saved edit, else its own text. While a review posts, Save edit
 * waits; text that changed meanwhile on a posted finding stays in its
 * editor.
 *
 * With the AI pass off (`AiRunState.enabled` false) the core returns no
 * output and the UI shows none of it.
 */

import { create } from "zustand";

import {
  aiState,
  cancelAi,
  dismissFinding,
  listChecklist,
  listFindings,
  listSkippedFindings,
  listVerdicts,
  postFindings,
  runAi,
  setChecklistChecked,
  setVerdictOverride,
  showFindingAnyway,
  toIpcError,
} from "../ipc/commands";
import { onAiState, onReviewChanged } from "../ipc/events";
import type {
  AiRunState,
  ChecklistItem,
  Finding,
  FindingId,
  IpcError,
  PostError,
  ReviewId,
  ThreadId,
  Verdict,
  VerdictLabel,
} from "../ipc/types";
import {
  currentRound,
  defaultBody,
  findingCount,
  postable,
  sortFindings,
} from "../lib/findings";
import { FOCUS, requestFocus } from "../lib/focusRequests";
import { latestOnly } from "../lib/latest";
import { openExternal } from "../lib/links";
import { useBackendSettingsStore } from "./backendSettingsStore";
import { useReviewStore } from "./reviewStore";
import {
  toastError,
  toastInfo,
  toastSuccess,
  useToastStore,
} from "./toastStore";

export interface ReviewAi {
  findings: Finding[];
  /**
   * Set aside as already raised in the review (`repeatOf` says by what);
   * absent until the first read.
   */
  skipped?: Finding[];
  checklist: ChecklistItem[];
  verdicts: Verdict[];
  loading: boolean;
  error: IpcError | null;
}

/** The UI's own state of one finding. */
export interface FindingUi {
  selected: boolean;
  /** The user's text, posted instead of the default; null: none. */
  edit: string | null;
  /** The open editor's text; null: closed. */
  editor: string | null;
  /** Why its last post failed. */
  error: PostError | null;
}

const NO_UI: FindingUi = {
  selected: false,
  edit: null,
  editor: null,
  error: null,
};

/** A request to scroll to a finding in the code view. */
export interface Reveal {
  findingId: FindingId;
  path: string;
  seq: number;
}

interface AiStoreState {
  runs: Record<ReviewId, AiRunState>;
  byReview: Record<ReviewId, ReviewAi>;
  ui: Record<FindingId, FindingUi>;
  /** "Post selected" is running for the review. */
  posting: Record<ReviewId, boolean>;
  /** A run or cancel request is in flight for the review. */
  starting: Record<ReviewId, boolean>;
  showDismissed: boolean;
  /** The finding last clicked: what the toggle-selection key acts on. */
  focused: FindingId | null;
  reveal: Reveal | null;

  loadState: (id: ReviewId) => Promise<void>;
  /** The review opened: its run and its output. */
  load: (id: ReviewId) => Promise<void>;
  /** Re-read the findings, checklist and verdicts. */
  reloadOutput: (id: ReviewId) => Promise<void>;
  /** `ai:state`. */
  applyState: (state: AiRunState) => void;
  /** `review:changed`: re-read the output of a loaded review. */
  applyChanged: (id: ReviewId) => Promise<void>;

  run: (id: ReviewId) => Promise<void>;
  cancel: (id: ReviewId) => Promise<void>;

  dismiss: (f: Finding, dismissed: boolean) => Promise<void>;
  /** "Show anyway": a set-aside finding becomes a normal one (local only). */
  showAnyway: (f: Finding) => Promise<void>;
  setChecked: (item: ChecklistItem, checked: boolean) => Promise<void>;
  setOverride: (
    reviewId: ReviewId,
    threadId: ThreadId,
    label: VerdictLabel | null,
  ) => Promise<void>;

  setSelected: (id: FindingId, selected: boolean) => void;
  /** The toggle-selection key: the focused finding, when it can be posted. */
  toggleFocused: (reviewId: ReviewId) => boolean;
  openEditor: (f: Finding) => void;
  setEditorText: (id: FindingId, text: string) => void;
  /** Keep the editor's text as the finding's edit (no write). */
  saveEditor: (f: Finding) => void;
  closeEditor: (id: FindingId) => void;
  /** Drop the edit: the finding posts with its own text again. */
  resetEdit: (id: FindingId) => void;
  /** "Post selected": the review's selected findings, with their edits. */
  postSelected: (reviewId: ReviewId) => Promise<void>;

  setShowDismissed: (show: boolean) => void;
  focus: (id: FindingId | null) => void;
  /** Scroll the code view to a finding (its file card selects and opens). */
  revealFinding: (f: Finding) => void;
}

const EMPTY: ReviewAi = {
  findings: [],
  skipped: [],
  checklist: [],
  verdicts: [],
  loading: false,
  error: null,
};

let revealSeq = 0;
/** Only the newest output read of a review applies its answer. */
const outputReads = latestOnly<ReviewId>();

export const useAiStore = create<AiStoreState>((set, get) => {
  const patchReview = (id: ReviewId, p: Partial<ReviewAi>) =>
    set((s) => ({
      byReview: { ...s.byReview, [id]: { ...(s.byReview[id] ?? EMPTY), ...p } },
    }));
  const patchUi = (id: FindingId, p: Partial<FindingUi>) =>
    set((s) => ({
      ui: { ...s.ui, [id]: { ...(s.ui[id] ?? NO_UI), ...p } },
    }));
  const setRun = (state: AiRunState) =>
    set((s) => ({ runs: { ...s.runs, [state.reviewId]: state } }));
  const findingOf = (reviewId: ReviewId, id: FindingId) =>
    get().byReview[reviewId]?.findings.find((f) => f.id === id);

  return {
    runs: {},
    byReview: {},
    ui: {},
    posting: {},
    starting: {},
    showDismissed: false,
    focused: null,
    reveal: null,

    loadState: async (id) => {
      try {
        const state = await aiState(id);
        if (state) setRun(state);
      } catch {
        // A review that went away; the list re-reads anyway.
      }
    },

    load: async (id) => {
      await Promise.all([get().loadState(id), get().reloadOutput(id)]);
    },

    reloadOutput: async (id) => {
      const current = outputReads.begin(id);
      patchReview(id, { loading: true });
      try {
        const [findings, skipped, checklist, verdicts] = await Promise.all([
          listFindings(id),
          listSkippedFindings(id),
          listChecklist(id),
          listVerdicts(id),
        ]);
        if (!current()) return;
        patchReview(id, {
          findings: findings ?? [],
          skipped: skipped ?? [],
          checklist: checklist ?? [],
          verdicts: verdicts ?? [],
          loading: false,
          error: null,
        });
      } catch (e) {
        if (!current()) return;
        patchReview(id, { loading: false, error: toIpcError(e) });
      }
    },

    applyState: (state) => {
      const prev = get().runs[state.reviewId];
      setRun(state);
      const finished =
        state.status === "done" &&
        (prev?.status !== "done" || prev.finishedAt !== state.finishedAt);
      if (finished && get().byReview[state.reviewId]) {
        void get().reloadOutput(state.reviewId);
      }
    },

    applyChanged: async (id) => {
      if (get().byReview[id]) await get().reloadOutput(id);
    },

    run: async (id) => {
      if (get().starting[id]) return;
      set((s) => ({ starting: { ...s.starting, [id]: true } }));
      try {
        setRun(await runAi(id));
      } catch (e) {
        toastError("Could not start the AI pass", toIpcError(e).message);
        await get().loadState(id);
      } finally {
        set((s) => ({ starting: { ...s.starting, [id]: false } }));
      }
    },

    cancel: async (id) => {
      if (get().starting[id]) return;
      set((s) => ({ starting: { ...s.starting, [id]: true } }));
      try {
        setRun(await cancelAi(id));
      } catch (e) {
        toastError("Could not stop the AI pass", toIpcError(e).message);
      } finally {
        set((s) => ({ starting: { ...s.starting, [id]: false } }));
      }
    },

    dismiss: async (f, dismissed) => {
      try {
        await dismissFinding(f.id, dismissed);
        if (dismissed) patchUi(f.id, { selected: false, editor: null });
      } catch (e) {
        toastError(
          dismissed
            ? "Could not dismiss the finding"
            : "Could not bring the finding back",
          toIpcError(e).message,
        );
      }
      await get().reloadOutput(f.reviewId);
    },

    showAnyway: async (f) => {
      try {
        await showFindingAnyway(f.id);
      } catch (e) {
        toastError("Could not show the finding", toIpcError(e).message);
      }
      await get().reloadOutput(f.reviewId);
    },

    setChecked: async (item, checked) => {
      try {
        await setChecklistChecked(item.id, checked);
      } catch (e) {
        toastError("Could not update the checklist", toIpcError(e).message);
      }
      await get().reloadOutput(item.reviewId);
    },

    setOverride: async (reviewId, threadId, label) => {
      try {
        await setVerdictOverride(threadId, label);
      } catch (e) {
        toastError("Could not change the verdict", toIpcError(e).message);
      }
      await get().reloadOutput(reviewId);
    },

    setSelected: (id, selected) => patchUi(id, { selected }),

    toggleFocused: (reviewId) => {
      const id = get().focused;
      const f = id ? findingOf(reviewId, id) : undefined;
      if (!f || !postable(f)) return false;
      patchUi(f.id, { selected: !(get().ui[f.id]?.selected ?? false) });
      return true;
    },

    openEditor: (f) => {
      const ui = get().ui[f.id] ?? NO_UI;
      if (ui.editor !== null || !postable(f)) return;
      requestFocus(FOCUS.finding(f.id));
      patchUi(f.id, { editor: ui.edit ?? defaultBody(f) });
    },

    setEditorText: (id, text) => patchUi(id, { editor: text }),

    saveEditor: (f) => {
      const text = get().ui[f.id]?.editor;
      if (text == null || text.trim() === "") return;
      if (get().posting[f.reviewId]) {
        // The finding may be on its way with its old text; a saved edit
        // would be wiped when it posts. The text stays in the editor.
        toastInfo("Wait for the post to finish", "Then save; your text stays.");
        return;
      }
      // The default text unchanged is no edit.
      patchUi(f.id, {
        edit: text === defaultBody(f) ? null : text,
        editor: null,
        error: null,
      });
    },

    closeEditor: (id) => {
      const ui = get().ui[id];
      const text = ui?.editor ?? null;
      patchUi(id, { editor: null });
      if (text === null || text.trim() === "") return;
      const f = Object.values(get().byReview)
        .flatMap((r) => r.findings)
        .find((x) => x.id === id);
      if (f && text === (ui?.edit ?? defaultBody(f))) return;
      // Esc is one key away from typing: keep the text one Undo away.
      useToastStore.getState().push({
        kind: "info",
        message: "Edit discarded",
        action: {
          label: "Undo",
          onClick: () => {
            if (get().ui[id]?.editor != null) return;
            requestFocus(FOCUS.finding(id));
            patchUi(id, { editor: text });
          },
        },
      });
    },

    resetEdit: (id) => patchUi(id, { edit: null, editor: null }),

    postSelected: async (reviewId) => {
      if (get().posting[reviewId]) return;
      const { ui } = get();
      const selected = sortFindings(
        (get().byReview[reviewId]?.findings ?? []).filter(
          (f) => postable(f) && ui[f.id]?.selected,
        ),
      );
      // What posts is what the user sees: an open editor's text, else the
      // saved edit. An emptied editor shows nothing to post: left out.
      const chosen = selected.filter((f) => ui[f.id]?.editor?.trim() !== "");
      const emptied = selected.length - chosen.length;
      if (emptied > 0) {
        toastInfo(
          `${findingCount(emptied)} with empty text not posted`,
          "Write something in the editor or close it.",
        );
      }
      if (chosen.length === 0) return;
      const bodyOf = (f: Finding): string | null => {
        const text = ui[f.id]?.editor ?? ui[f.id]?.edit ?? null;
        return text === null || text === defaultBody(f) ? null : text;
      };
      set((s) => ({ posting: { ...s.posting, [reviewId]: true } }));
      try {
        const results = await postFindings(
          reviewId,
          chosen.map((f) => ({ findingId: f.id, body: bodyOf(f) })),
        );
        let posted = 0;
        let lastUrl: string | null = null;
        let failed = 0;
        let keptEdits = 0;
        for (const r of results) {
          if (r.status === "posted") {
            posted += 1;
            lastUrl = r.postedUrl;
            // An editor opened (or changed) while it posted is not what was
            // posted: it stays. So does a saved edit that changed: back in
            // the editor, the only place a posted finding shows it.
            const sent = ui[r.findingId];
            const now = get().ui[r.findingId];
            const openNow = now?.editor ?? null;
            const editNow = now?.edit ?? null;
            const openKept =
              openNow !== (sent?.editor ?? null) ? openNow : null;
            const editKept = editNow !== (sent?.edit ?? null) ? editNow : null;
            if (openKept === null && editKept !== null) keptEdits += 1;
            patchUi(r.findingId, { ...NO_UI, editor: openKept ?? editKept });
          } else {
            failed += 1;
            patchUi(r.findingId, {
              selected: true,
              error: { kind: r.kind, message: r.message },
            });
          }
        }
        if (keptEdits > 0) {
          toastInfo(
            "Your newer edit stays in its editor",
            "The finding was posted with the text it had when you pressed Post selected.",
          );
        }
        if (failed === 0) {
          const url = posted === 1 ? lastUrl : null;
          if (url) {
            useToastStore.getState().push({
              kind: "success",
              message: "Posted 1 finding",
              action: { label: "Open", onClick: () => openExternal(url) },
            });
          } else {
            toastSuccess(`Posted ${findingCount(posted)}`);
          }
        } else {
          if (posted > 0) toastSuccess(`Posted ${findingCount(posted)}`);
          toastError(
            `${failed} of ${findingCount(chosen.length)} not posted`,
            "They stay selected with the reason; edit them or post again.",
          );
        }
      } catch (e) {
        // Nothing was tried: every finding stays as it was.
        toastError("Could not post the findings", toIpcError(e).message);
      } finally {
        set((s) => {
          const next = { ...s.posting };
          delete next[reviewId];
          return { posting: next };
        });
      }
      await get().reloadOutput(reviewId);
    },

    setShowDismissed: (show) => set({ showDismissed: show }),

    focus: (id) => set({ focused: id }),

    revealFinding: (f) => {
      revealSeq += 1;
      set({
        focused: f.id,
        reveal: { findingId: f.id, path: f.path, seq: revealSeq },
      });
    },
  };
});

// -- Hooks ----------------------------------------------------------------------

/** The review's latest run; undefined before it was read. */
export function useAiRun(reviewId: ReviewId | null): AiRunState | undefined {
  return useAiStore((s) => (reviewId ? s.runs[reviewId] : undefined));
}

/** The note shown where the agent cannot be held to reading only. */
export const NOT_READ_ONLY_NOTE =
  "Auggie can still use MCP tools from your own Auggie setup and read local secret files in the clone (.env files, keys); Argus cannot guarantee this run only reads.";

/**
 * Whether the configured agent can be held to reading only: read from the
 * settings, not from the runs (none may be loaded, or still report the
 * agent they last ran with). Auggie cannot be held to it.
 */
export function useReadOnlyGuaranteed(): boolean {
  return useBackendSettingsStore((s) => s.settings?.ai.adapter !== "auggie");
}

/** The AI pass is on for this review (else none of its UI shows). */
export function useAiEnabled(reviewId: ReviewId | null): boolean {
  return useAiStore((s) => (reviewId ? !!s.runs[reviewId]?.enabled : false));
}

const NO_FINDINGS: Finding[] = [];
const NO_VERDICT = null;

/** The review's findings (a stable empty list when none or AI is off). */
export function useFindings(reviewId: ReviewId): Finding[] {
  return useAiStore((s) =>
    s.runs[reviewId]?.enabled
      ? (s.byReview[reviewId]?.findings ?? NO_FINDINGS)
      : NO_FINDINGS,
  );
}

/** The review's findings set aside as already raised (none with AI off). */
export function useSkippedFindings(reviewId: ReviewId): Finding[] {
  return useAiStore((s) =>
    s.runs[reviewId]?.enabled
      ? (s.byReview[reviewId]?.skipped ?? NO_FINDINGS)
      : NO_FINDINGS,
  );
}

export function useVerdict(
  reviewId: ReviewId,
  threadId: ThreadId,
): Verdict | null {
  return useAiStore((s) =>
    s.runs[reviewId]?.enabled
      ? (s.byReview[reviewId]?.verdicts.find((v) => v.threadId === threadId) ??
        NO_VERDICT)
      : NO_VERDICT,
  );
}

/** The round the review's AI output is current for (`currentRound`). */
export function useCurrentRound(reviewId: ReviewId): number | null {
  return useAiStore((s) => {
    const out = s.byReview[reviewId];
    return currentRound(
      s.runs[reviewId],
      out?.findings ?? NO_FINDINGS,
      out?.checklist ?? [],
    );
  });
}

/** One finding of the review, as stored. */
export function useFinding(
  reviewId: ReviewId,
  id: FindingId,
): Finding | undefined {
  return useAiStore((s) =>
    s.byReview[reviewId]?.findings.find((f) => f.id === id),
  );
}

export function useFindingUi(id: FindingId): FindingUi {
  return useAiStore((s) => s.ui[id] ?? NO_UI);
}

/** Open a posted finding's comment in the browser. */
export function openPosted(f: Finding) {
  if (f.postedUrl) openExternal(f.postedUrl);
}

// -- Events -----------------------------------------------------------------------

/**
 * Follow `ai:state` and `review:changed`; read the run of every listed
 * review (the list's running indicator); re-read everything when the AI
 * settings change (the pass turned on or off). Resolves to an unsubscribe.
 */
export async function subscribeAiEvents(): Promise<() => void> {
  const offs = await Promise.all([
    onAiState((state) => useAiStore.getState().applyState(state)),
    onReviewChanged((e) => void useAiStore.getState().applyChanged(e.reviewId)),
  ]);

  const known = new Set<ReviewId>();
  const readNew = (ids: ReviewId[]) => {
    for (const id of ids) {
      if (known.has(id)) continue;
      known.add(id);
      void useAiStore.getState().loadState(id);
    }
  };
  readNew(useReviewStore.getState().reviews.map((r) => r.id));
  const offReviews = useReviewStore.subscribe((s, prev) => {
    if (s.reviews !== prev.reviews) readNew(s.reviews.map((r) => r.id));
  });

  const offSettings = useBackendSettingsStore.subscribe((s, prev) => {
    const now = s.settings?.ai;
    const before = prev.settings?.ai;
    if (!now || !before) return;
    // Switching the pass or the agent changes what the runs report
    // (`enabled`, `readOnlyGuaranteed`): read them again.
    if (now.enabled === before.enabled && now.adapter === before.adapter) {
      return;
    }
    const store = useAiStore.getState();
    for (const id of Object.keys(store.runs)) void store.loadState(id);
    for (const id of Object.keys(store.byReview)) void store.reloadOutput(id);
  });

  return () => {
    offs.forEach((off) => off());
    offReviews();
    offSettings();
  };
}
