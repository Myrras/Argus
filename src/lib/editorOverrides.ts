/**
 * Settings → Editors: the per-extension editor overrides
 * (`Settings.editorOverrides`, extension → application path), ported from
 * Iris. The core opens a review's file from the local clone with the app
 * set for its extension (case-insensitive, the last extension only), else
 * `defaultEditor`, else the OS default. Validation mirrors the core's
 * normalization (trimmed, lower-case, no leading dot) so mistakes show
 * inline; the core still has the final word.
 */

/** As the core stores it: trimmed, leading dots dropped, lower-case. */
export function normalizeExtension(raw: string): string {
  return raw.trim().replace(/^\.+/, "").toLowerCase();
}

/**
 * Why `ext` → `app` cannot be saved, or `null`. `original` is the
 * extension of the row being edited (`null` when adding).
 */
export function overrideError(
  ext: string,
  app: string,
  existing: Record<string, string>,
  original: string | null,
): string | null {
  if (!ext) return "Enter an extension, like md.";
  // Only the last extension counts (`a.tar.gz` opens as `gz`).
  if (/[\s./\\]/.test(ext)) {
    return "Use a single extension without dots or spaces, like md.";
  }
  if (ext !== original && ext in existing) {
    return `.${ext} already has an editor.`;
  }
  if (!app.trim()) return "Choose an application.";
  return null;
}

/** The overrides with `original` replaced by (or `ext` added as) `app`. */
export function withOverride(
  existing: Record<string, string>,
  ext: string,
  app: string,
  original: string | null,
): Record<string, string> {
  const next = { ...existing };
  if (original !== null) delete next[original];
  next[ext] = app.trim();
  return next;
}

/** The overrides without `ext`. */
export function withoutOverride(
  existing: Record<string, string>,
  ext: string,
): Record<string, string> {
  const next = { ...existing };
  delete next[ext];
  return next;
}

/** An application's display name: `Typora` for `/Applications/Typora.app`. */
export function appName(path: string): string {
  const name =
    path
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .pop() ?? path;
  return name.replace(/\.(app|exe)$/i, "") || path;
}
