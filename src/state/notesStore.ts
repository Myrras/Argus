/**
 * Notes store — the sample of the app's state/IPC pattern.
 *
 * The store is the single place the UI learns about notes: actions call the
 * typed IPC wrappers, then refresh from the backend so the store mirrors the
 * core's state rather than guessing at it. Errors are normalized to `IpcError`
 * and surfaced via `lastError` for the UI to show.
 */

import { create } from "zustand";

import { noteAdd, noteList, noteRemove, toIpcError } from "../ipc/commands";
import type { IpcError, Note, NoteId } from "../ipc/types";

interface NotesState {
  notes: Note[];
  lastError: IpcError | null;
  refresh: () => Promise<void>;
  add: (text: string) => Promise<void>;
  remove: (id: NoteId) => Promise<void>;
}

export const useNotesStore = create<NotesState>((set) => ({
  notes: [],
  lastError: null,
  refresh: async () => {
    try {
      set({ notes: await noteList(), lastError: null });
    } catch (e) {
      set({ lastError: toIpcError(e) });
    }
  },
  add: async (text: string) => {
    try {
      await noteAdd(text);
      set({ notes: await noteList(), lastError: null });
    } catch (e) {
      set({ lastError: toIpcError(e) });
    }
  },
  remove: async (id: NoteId) => {
    try {
      await noteRemove(id);
      set({ notes: await noteList(), lastError: null });
    } catch (e) {
      set({ lastError: toIpcError(e) });
    }
  },
}));
