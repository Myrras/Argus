/**
 * Settings → AI → Local clones: the local checkout per repository
 * (`Settings.clonePaths`, `host/repo` → folder), Iris's editor-overrides
 * pattern. The core looks a review's clone up by `host/repo` exactly as the
 * provider reports it, so the key is only tidied (trimmed, scheme, `.git`
 * and slashes at the ends dropped) here, never re-cased.
 */

/** `https://github.com/acme/widgets.git/` → `github.com/acme/widgets`. */
export function normalizeRepo(raw: string): string {
  return raw
    .trim()
    .replace(/^[a-z]+:\/\//i, "")
    .replace(/\/+$/, "")
    .replace(/\.git$/i, "")
    .replace(/^\/+/, "");
}

/**
 * Why `repo` → `path` cannot be saved, or `null`. `original` is the key of
 * the row being edited (`null` when adding).
 */
export function cloneError(
  repo: string,
  path: string,
  existing: Record<string, string>,
  original: string | null,
): string | null {
  if (!repo) return "Enter a repository, like github.com/acme/widgets.";
  // A host, then at least owner/name (GitLab groups may nest deeper).
  if (/\s/.test(repo) || !/^[^/]+\/[^/]+\/.+$/.test(repo)) {
    return "Use host/owner/name, like github.com/acme/widgets.";
  }
  if (repo !== original && repo in existing) {
    return `${repo} already has a clone.`;
  }
  if (!path.trim()) return "Choose a folder.";
  return null;
}

/** The clones with `original` replaced by (or `repo` added as) `path`. */
export function withClone(
  existing: Record<string, string>,
  repo: string,
  path: string,
  original: string | null,
): Record<string, string> {
  const next = { ...existing };
  if (original !== null) delete next[original];
  next[repo] = path.trim();
  return next;
}

/** The clones without `repo`. */
export function withoutClone(
  existing: Record<string, string>,
  repo: string,
): Record<string, string> {
  const next = { ...existing };
  delete next[repo];
  return next;
}
