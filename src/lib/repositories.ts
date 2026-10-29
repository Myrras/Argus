/**
 * Settings → Repositories: pure rules for the per-repository setup (local
 * clone, review skill, review language; all keyed `host/repo` in
 * `Settings`) and for the review language picker the AI tab shares.
 */

import type {
  CloneState,
  RepoSetup,
  ReviewLanguage,
  Settings,
} from "../ipc/types";

/** Above this many repositories the tab offers a filter. */
export const FILTER_FROM = 7;

/** Nothing set: no clone, the skill on Auto, the default language. */
export function isSetUp(settings: Settings | null, key: string): boolean {
  return (
    !!settings?.clonePaths?.[key] ||
    !!settings?.reviewSkills?.[key] ||
    !!settings?.reviewLanguages?.[key]
  );
}

/** The repositories whose key contains `query` (any case). */
export function filterRepos(repos: RepoSetup[], query: string): RepoSetup[] {
  const q = query.trim().toLowerCase();
  return q ? repos.filter((r) => r.key.toLowerCase().includes(q)) : repos;
}

/** What the clone line says about its folder. */
export const CLONE_STATE_LABEL: Record<CloneState, string> = {
  notSet: "Not set",
  found: "Found",
  missing: "Folder not found",
  notRepository: "Not a git repository",
};

export const AUTO_LANGUAGE: ReviewLanguage = { kind: "auto" };

/** A language as a short label: `Auto`, `English`, `Nederlands`, `Deutsch`. */
export function languageLabel(language: ReviewLanguage | null | undefined) {
  switch (language?.kind ?? "auto") {
    case "english":
      return "English";
    case "dutch":
      return "Nederlands";
    case "other":
      return language?.kind === "other" ? language.name : "Other";
    default:
      return "Auto";
  }
}

/** The global default for repositories without their own (`auto` unset). */
export function globalLanguage(settings: Settings | null): ReviewLanguage {
  return settings?.ai?.reviewLanguage ?? AUTO_LANGUAGE;
}

/** The repository's own language; `null` follows the global default. */
export function languageFor(
  settings: Settings | null,
  key: string,
): ReviewLanguage | null {
  return settings?.reviewLanguages?.[key] ?? null;
}

export function sameLanguage(
  a: ReviewLanguage | null,
  b: ReviewLanguage | null,
): boolean {
  if (a === null || b === null) return a === b;
  if (a.kind !== b.kind) return false;
  return a.kind !== "other" || (b.kind === "other" && a.name === b.name);
}

/** The languages with `key` set to `language` (`null` drops the entry). */
export function withLanguage(
  existing: Record<string, ReviewLanguage> | undefined,
  key: string,
  language: ReviewLanguage | null,
): Record<string, ReviewLanguage> {
  const next = { ...(existing ?? {}) };
  if (language === null) delete next[key];
  else next[key] = language;
  return next;
}

/** The core keeps a language name to this many characters. */
export const MAX_LANGUAGE_NAME = 40;

/** A typed language name as the core keeps it; `null` when blank. */
export function otherLanguage(name: string): ReviewLanguage | null {
  const tidy = name.replace(/\s+/g, " ").trim().slice(0, MAX_LANGUAGE_NAME);
  return tidy ? { kind: "other", name: tidy.trim() } : null;
}

/** The picker's option values: `default` only where a default exists. */
export type LanguageOption = "default" | ReviewLanguage["kind"];

export function optionOf(language: ReviewLanguage | null): LanguageOption {
  return language === null ? "default" : language.kind;
}

/**
 * The language a picked option stands for; `undefined` for `other`, whose
 * name the user still types (and saves with its own button).
 */
export function languageOfOption(
  option: LanguageOption,
): ReviewLanguage | null | undefined {
  switch (option) {
    case "default":
      return null;
    case "other":
      return undefined;
    default:
      return { kind: option };
  }
}
