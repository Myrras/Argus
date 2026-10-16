/**
 * Wire types for the Tauri IPC bridge.
 *
 * These mirror the serde shapes in `src-tauri` / `argus-core` exactly — change
 * them only in lockstep with the Rust side:
 * - Ids are plain UUID strings (`#[serde(transparent)]`).
 * - Structs are camelCase (`rename_all = "camelCase"` where applicable).
 */

export type NoteId = string;

/** One note (`argus_core::Note`). */
export interface Note {
  id: NoteId;
  text: string;
}

/** Structured error every IPC command can reject with (`IpcError`). */
export interface IpcError {
  /** Human-readable, sanitized description. */
  message: string;
  /** Stable machine-readable category, e.g. `"invalidInput"`. */
  kind: string;
}
