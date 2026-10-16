import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

// jsdom has no Tauri bridge: stub `invoke` and assert the store's IPC wiring
// (command names, camelCase argument keys, resync after a mutation).
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { useNotesStore } from "./notesStore";

const mockInvoke = vi.mocked(invoke);

describe("notesStore", () => {
  beforeEach(() => {
    useNotesStore.setState({ notes: [], lastError: null });
  });

  it("adds a note, then re-reads the list from the backend", async () => {
    const note = { id: "00000000-0000-4000-8000-000000000001", text: "draft" };
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "note_list" ? [note] : note),
    );

    await useNotesStore.getState().add("draft");

    expect(mockInvoke).toHaveBeenCalledWith("note_add", { text: "draft" });
    expect(mockInvoke).toHaveBeenCalledWith("note_list");
    expect(useNotesStore.getState().notes).toEqual([note]);
  });

  it("removes by noteId and surfaces an IpcError as lastError", async () => {
    mockInvoke.mockRejectedValue({
      kind: "notFound",
      message: "note not found",
    });

    await useNotesStore.getState().remove("missing");

    expect(mockInvoke).toHaveBeenCalledWith("note_remove", {
      noteId: "missing",
    });
    expect(useNotesStore.getState().lastError).toEqual({
      kind: "notFound",
      message: "note not found",
    });
  });
});
