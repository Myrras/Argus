/**
 * The mounted diffs, by file path, so a keyboard move can ask the right one
 * to bring a block (a thread, a finding) into view. CodeMirror renders only
 * the lines in or near the viewport, so a block far down a long file has no
 * DOM until its editor scrolls there (`lib/revealItem`). No CodeMirror
 * import here: the editor itself loads lazily.
 */

/** What a mounted `DiffEditor` offers the reveal. */
export interface DiffRevealer {
  /**
   * Scroll the editor (and the page column around it) to the line holding
   * block `key`; false when this editor does not hold that block (yet).
   */
  reveal: (key: string) => boolean;
}

/** A thread's block key in a `DiffEditor`. */
export function threadBlockKey(threadId: string): string {
  return `t:${threadId}`;
}

/** A finding's block key (`FileDiffCard`'s line item). */
export function findingBlockKey(findingId: string): string {
  return `finding:${findingId}`;
}

const editors = new Map<string, DiffRevealer>();

/** Register the diff of `path`; returns the unregister. */
export function registerDiffEditor(
  path: string,
  editor: DiffRevealer,
): () => void {
  editors.set(path, editor);
  return () => {
    // A remount (another layout) may have registered its own already.
    if (editors.get(path) === editor) editors.delete(path);
  };
}

/** The mounted diff of `path`, if any. */
export function diffEditorFor(path: string): DiffRevealer | null {
  return editors.get(path) ?? null;
}
