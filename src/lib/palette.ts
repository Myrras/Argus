/**
 * The command palette's rows (M6): which items a mode and query show, in
 * section order, best match first within a section. Pure.
 */

import { fuzzyFilter, type FuzzyMatch } from "./fuzzy";

export type PaletteMode = "all" | "reviews" | "files";
export type PaletteSection = "Commands" | "Actions" | "Reviews" | "Files";

export interface PaletteItem {
  id: string;
  section: PaletteSection;
  label: string;
  /** Matched besides the label (not highlighted). */
  extra?: string;
  detail?: string;
  keys?: string[];
  /** Keep the palette open (a "Go to" mode). */
  stay?: boolean;
  run: () => void;
}

const SECTIONS: PaletteSection[] = ["Commands", "Actions", "Reviews", "Files"];
/** Per section with a query in the "all" mode; a "Go to" mode shows more. */
const LIMIT_ALL = 8;
const LIMIT_MODE = 200;

export interface PaletteRow {
  item: PaletteItem;
  match: FuzzyMatch;
}

/** The rows shown for `mode` and `query`, grouped by section. */
export function paletteRows(
  items: readonly PaletteItem[],
  mode: PaletteMode,
  query: string,
): PaletteRow[] {
  const pool = items.filter((i) =>
    mode === "reviews"
      ? i.section === "Reviews"
      : mode === "files"
        ? i.section === "Files"
        : // Without a query the "all" mode lists the commands and actions.
          query.trim() !== "" ||
          i.section === "Commands" ||
          i.section === "Actions",
  );
  const hits = fuzzyFilter(pool, query, (i) =>
    i.extra ? `${i.label} ${i.extra}` : i.label,
  );
  const limit = mode === "all" && query.trim() !== "" ? LIMIT_ALL : LIMIT_MODE;
  const rows: PaletteRow[] = [];
  for (const section of SECTIONS) {
    const inSection = hits.filter((h) => h.item.section === section);
    rows.push(
      ...(section === "Reviews" || section === "Files"
        ? inSection.slice(0, limit)
        : inSection),
    );
  }
  return rows;
}
