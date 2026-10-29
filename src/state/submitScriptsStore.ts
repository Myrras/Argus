/**
 * What the Scripts tab needs from the core besides the settings: where
 * Argus saves the scripts written with the AI (`submit_scripts_dir`),
 * whether "Write with AI" can run (`script_generation_status`: the AI pass
 * on, its agent found), the "Write with AI" drafts (`list_script_drafts` at
 * start, then every `script-draft:state`; the higher `revision` wins) and
 * the one open writer (`writer`: a draft, or a new one for a new script or
 * for changing the one of `scriptId`).
 *
 * A draft runs in the background: closing the writer or Settings never
 * stops it. When one ends while its writer is not open, a toast says so
 * ("Script ready: …" with Open, or the failure).
 */

import { create } from "zustand";

import {
  listScriptDrafts,
  scriptGenerationStatus,
  submitScriptsDir,
  toIpcError,
} from "../ipc/commands";
import { onScriptDraftProgress, onScriptDraftState } from "../ipc/events";
import type {
  ScriptDraft,
  ScriptDraftProgressEvent,
  ScriptGenerationStatus,
} from "../ipc/types";
import { upsertDraft, withProgress } from "../lib/submitScripts";
import { useSettingsRequestStore } from "./settingsRequestStore";
import { toastError, useToastStore } from "./toastStore";

/**
 * The "Write with AI" dialog: an existing draft (`draftId`), or one not
 * sent yet (`draftId` null) for a new script or changing `scriptId`.
 */
export interface WriterRequest {
  draftId: string | null;
  scriptId: string | null;
  /** Each open is a new dialog; sending its first request keeps it. */
  session: number;
}

/**
 * The thinking of a draft's last finished turn, kept in this window only
 * (the core drops it when the turn ends) so the writer can show it folded.
 */
export interface Thought {
  turn: number;
  thinking: string;
  thinkingCut: boolean;
  /** How long the turn ran. */
  ms: number;
}

interface SubmitScriptsState {
  dir: string | null;
  /** `null` until read. */
  generation: ScriptGenerationStatus | null;
  /** Every draft of this session, oldest first. */
  drafts: ScriptDraft[];
  /** Per draft id: its last finished turn's thinking (this window only). */
  thoughts: Record<string, Thought>;
  writer: WriterRequest | null;
  /** Re-read the folder and whether generation can run. */
  refresh: () => Promise<void>;
  loadDrafts: () => Promise<void>;
  /** Take a draft from an IPC reply or event (toasts when one ended). */
  applyDraft: (draft: ScriptDraft) => void;
  /** Take a `script-draft:progress` report. */
  applyProgress: (event: ScriptDraftProgressEvent) => void;
  /** Drop a discarded draft (the core reports nothing more about it). */
  removeDraft: (draftId: string) => void;
  /**
   * Open the writer for a new script (`null`) or for changing `scriptId`:
   * the draft linked to that script when there is one.
   */
  openWriter: (scriptId: string | null) => void;
  openDraft: (draftId: string) => void;
  /** The open writer's first request started `draftId`. */
  attachDraft: (draftId: string) => void;
  closeWriter: () => void;
}

/** Ids discarded in this session: a late event never brings one back. */
const discarded = new Set<string>();
let session = 0;

export const useSubmitScriptsStore = create<SubmitScriptsState>((set, get) => ({
  dir: null,
  generation: null,
  drafts: [],
  thoughts: {},
  writer: null,
  refresh: async () => {
    const [dir, generation] = await Promise.all([
      submitScriptsDir().catch(() => null),
      scriptGenerationStatus().catch((e: unknown): ScriptGenerationStatus => ({
        available: false,
        adapter: "",
        reason: toIpcError(e).message,
      })),
    ]);
    set({ dir: dir ?? null, generation: generation ?? null });
  },
  loadDrafts: async () => {
    const drafts = await listScriptDrafts().catch(() => [] as ScriptDraft[]);
    let next = get().drafts;
    for (const d of drafts ?? []) {
      if (!discarded.has(d.id)) next = upsertDraft(next, d);
    }
    set({ drafts: next });
  },
  applyDraft: (draft) => {
    if (discarded.has(draft.id)) return;
    const before = get().drafts.find((d) => d.id === draft.id);
    const next = upsertDraft(get().drafts, draft);
    set({ drafts: next });
    const now = next.find((d) => d.id === draft.id);
    // The turn ended: keep what it thought, for the writer to show folded.
    const p = before?.progress;
    if (p && now && !now.progress && p.thinking.trim()) {
      set((s) => ({
        thoughts: {
          ...s.thoughts,
          [draft.id]: {
            turn: p.turn,
            thinking: p.thinking,
            thinkingCut: p.thinkingCut,
            ms: Date.parse(now.updatedAt) - Date.parse(p.startedAt),
          },
        },
      }));
    }
    if (before?.status === "running" && now && now.status !== "running") {
      ended(now, get().writer?.draftId === now.id);
    }
  },
  applyProgress: (event) => {
    if (discarded.has(event.draftId)) return;
    set((s) => ({ drafts: withProgress(s.drafts, event) }));
  },
  removeDraft: (draftId) => {
    discarded.add(draftId);
    set((s) => ({
      drafts: s.drafts.filter((d) => d.id !== draftId),
      writer: s.writer?.draftId === draftId ? null : s.writer,
    }));
  },
  openWriter: (scriptId) => {
    const linked = scriptId
      ? get().drafts.find((d) => d.scriptId === scriptId)
      : undefined;
    session += 1;
    set({ writer: { draftId: linked?.id ?? null, scriptId, session } });
  },
  openDraft: (draftId) => {
    const d = get().drafts.find((x) => x.id === draftId);
    session += 1;
    set({ writer: { draftId, scriptId: d?.scriptId ?? null, session } });
  },
  attachDraft: (draftId) =>
    set((s) => ({ writer: s.writer ? { ...s.writer, draftId } : null })),
  closeWriter: () => set({ writer: null }),
}));

/** Open Settings → Scripts on a draft's writer (the toast's Open). */
export function showDraft(draftId: string): void {
  useSettingsRequestStore.getState().open("scripts");
  useSubmitScriptsStore.getState().openDraft(draftId);
}

/** A run ended: say so unless its writer is open (it shows there). */
function ended(d: ScriptDraft, writerOpen: boolean): void {
  if (writerOpen) return;
  if (d.status === "idle") {
    useToastStore.getState().push({
      kind: "success",
      message: `Script ready: ${d.title}`,
      action: { label: "Open", onClick: () => showDraft(d.id) },
    });
  } else if (d.status === "failed") {
    toastError(`Couldn't write ${d.title}`, d.error ?? undefined, {
      action: { label: "Open", onClick: () => showDraft(d.id) },
    });
  }
}

/** Listen for `script-draft:state` and `:progress`; resolves to an unsubscribe. */
export async function subscribeScriptDraftEvents(): Promise<() => void> {
  const offs = await Promise.all([
    onScriptDraftState((d) => useSubmitScriptsStore.getState().applyDraft(d)),
    onScriptDraftProgress((e) =>
      useSubmitScriptsStore.getState().applyProgress(e),
    ),
  ]);
  return () => offs.forEach((off) => off());
}
