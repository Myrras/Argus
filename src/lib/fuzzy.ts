/**
 * Fuzzy matching for the command palette. Pure.
 *
 * Case-insensitive; a blank query matches everything (score 0). The query's
 * characters must appear in order. A plain substring beats a scattered
 * match, an earlier one a later one; scattered matches score consecutive
 * runs and word starts (after a space, `/`, `-`, `_`, `.` or `!`/`#`)
 * higher. Spaces in the query separate words that each must match.
 */

export interface FuzzyMatch {
  score: number;
  /** Indices into the text of the matched characters (for highlighting). */
  indices: number[];
}

const WORD_BREAK = /[\s/\-_.!#:()]/;

function wordStart(text: string, i: number): boolean {
  return i === 0 || WORD_BREAK.test(text[i - 1]);
}

/** One query word against the text, or null. */
function matchWord(
  text: string,
  lower: string,
  word: string,
): FuzzyMatch | null {
  const at = lower.indexOf(word);
  if (at >= 0) {
    const indices = Array.from({ length: word.length }, (_, k) => at + k);
    // A substring: high, better at a word start and earlier.
    return {
      score: 1000 + (wordStart(text, at) ? 200 : 0) - Math.min(at, 200),
      indices,
    };
  }
  return (
    scattered(text, lower, word, true) ?? scattered(text, lower, word, false)
  );
}

/**
 * The characters in order; `jump`: skip ahead to a later word start holding
 * the character when the next one is neither consecutive nor a word start.
 */
function scattered(
  text: string,
  lower: string,
  word: string,
  jump: boolean,
): FuzzyMatch | null {
  const indices: number[] = [];
  let score = 0;
  let prev = -2;
  for (const ch of word) {
    let i = lower.indexOf(ch, prev + 1);
    if (i < 0) return null;
    if (jump && i !== prev + 1 && !wordStart(text, i)) {
      for (
        let j = lower.indexOf(ch, i + 1);
        j >= 0;
        j = lower.indexOf(ch, j + 1)
      ) {
        if (wordStart(text, j)) {
          i = j;
          break;
        }
      }
    }
    score += i === prev + 1 ? 15 : wordStart(text, i) ? 10 : 1;
    indices.push(i);
    prev = i;
  }
  return { score, indices };
}

export function fuzzyMatch(text: string, query: string): FuzzyMatch | null {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return { score: 0, indices: [] };
  const lower = text.toLowerCase();
  let score = 0;
  const indices = new Set<number>();
  for (const w of words) {
    const m = matchWord(text, lower, w);
    if (!m) return null;
    score += m.score;
    for (const i of m.indices) indices.add(i);
  }
  return { score, indices: [...indices].sort((a, b) => a - b) };
}

/**
 * The items matching `query`, best first (ties keep their order); each with
 * its match on `textOf(item)`.
 */
export function fuzzyFilter<T>(
  items: readonly T[],
  query: string,
  textOf: (item: T) => string,
): { item: T; match: FuzzyMatch }[] {
  const out: { item: T; match: FuzzyMatch; at: number }[] = [];
  items.forEach((item, at) => {
    const match = fuzzyMatch(textOf(item), query);
    if (match) out.push({ item, match, at });
  });
  out.sort((a, b) => b.match.score - a.match.score || a.at - b.at);
  return out.map(({ item, match }) => ({ item, match }));
}
