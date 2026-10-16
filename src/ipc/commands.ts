/**
 * Typed wrappers over every Tauri IPC command.
 *
 * All command rejections are `IpcError` shapes from the Rust side;
 * `toIpcError` normalizes anything else (bridge failures, plugin errors) into
 * the same shape so callers have exactly one error type to handle.
 */

import { invoke } from "@tauri-apps/api/core";

import type { IpcError, Note, NoteId } from "./types";

/** Narrow an unknown rejection to the structured `IpcError` shape. */
export function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as IpcError).message === "string" &&
    typeof (value as IpcError).kind === "string"
  );
}

/** Coerce any rejection into an `IpcError` (kind `"unknown"` as fallback). */
export function toIpcError(value: unknown): IpcError {
  if (isIpcError(value)) return value;
  if (value instanceof Error)
    return { message: value.message, kind: "unknown" };
  return { message: String(value), kind: "unknown" };
}

/** Greet `name`; rejects with an `invalidInput` `IpcError` for blank input. */
export function greet(name: string): Promise<string> {
  return invoke("greet", { name });
}

/** Add a note and return it. */
export function noteAdd(text: string): Promise<Note> {
  return invoke("note_add", { text });
}

/** All notes, in insertion order. */
export function noteList(): Promise<Note[]> {
  return invoke("note_list");
}

/** Remove a note by id. */
export function noteRemove(noteId: NoteId): Promise<void> {
  return invoke("note_remove", { noteId });
}
