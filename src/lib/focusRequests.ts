/**
 * One-shot focus requests for fields that mount inside CodeMirror widgets.
 * A widget's React root is unmounted when it scrolls out of the editor's
 * viewport and mounted again when it scrolls back; a field that focused on
 * every mount would then steal the focus from wherever the user is. So the
 * action that opens it (a click, a key) asks once (`requestFocus`), and the
 * field takes the focus only when it finds a request (`takeFocus`).
 */

/** When each request was made; one older than this is stale. */
const pending = new Map<string, number>();
const FRESH_MS = 1000;

/** The field `key` should take the focus when it mounts next. */
export function requestFocus(key: string): void {
  pending.set(key, Date.now());
}

/**
 * Whether the field `key` should take the focus now: a request made just
 * before (a field already shown never mounts to take it, so it goes stale).
 * Cleared after this task, so a React StrictMode double mount still finds it.
 */
export function takeFocus(key: string): boolean {
  const at = pending.get(key);
  if (at === undefined || Date.now() - at > FRESH_MS) return false;
  setTimeout(() => pending.delete(key), 0);
  return true;
}

/** The focus keys of the composers. */
export const FOCUS = {
  composer: "composer",
  reply: (threadId: string) => `reply:${threadId}`,
  finding: (findingId: string) => `finding:${findingId}`,
};
