/**
 * The find bar's query (Cmd+F, `lib/findInReview`): its text and options
 * compiled into one regular expression, and the matches of that expression
 * in a line or a text. Pure; shared by the search, the CodeMirror
 * highlighting (`lib/findHighlight`) and the comment highlighting
 * (`lib/markdownText`), so all three find the same matches.
 *
 * Matches never cross a line break: the code is searched line by line, and
 * comment text is too, so a match's place is the same wherever it shows.
 * Empty matches (`^`, `a*`) are skipped.
 */

export interface FindOptions {
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
}

export interface CompiledFind {
  /** Null for an empty query or an invalid regex. */
  re: RegExp | null;
  /** Why the regex is invalid, in a few words. */
  error: string | null;
}

/** At most this many matches per line (a pathological regex stays cheap). */
const MAX_PER_LINE = 1000;

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The engine's message without its "Invalid regular expression: /…/:" lead. */
function regexError(e: unknown): string {
  const text = e instanceof Error ? e.message : String(e);
  const tail = text.split(": ").pop()?.trim() ?? "";
  return tail ? `Invalid regex: ${tail}` : "Invalid regex";
}

/**
 * The query as one global regular expression. Whole word: no word
 * character right before or after the match (works for any query, also
 * one that starts or ends with punctuation).
 */
export function compileFind(text: string, opts: FindOptions): CompiledFind {
  if (text === "") return { re: null, error: null };
  let source = opts.regex ? text : escapeRegExp(text);
  if (opts.wholeWord) source = `(?<!\\w)(?:${source})(?!\\w)`;
  try {
    return {
      re: new RegExp(source, opts.caseSensitive ? "g" : "gi"),
      error: null,
    };
  } catch (e) {
    return { re: null, error: regexError(e) };
  }
}

/** The matches of `re` in one line, as `[start, end)` pairs. */
export function lineMatches(line: string, re: RegExp): [number, number][] {
  const out: [number, number][] = [];
  re.lastIndex = 0;
  for (let m = re.exec(line); m; m = re.exec(line)) {
    if (m[0].length === 0) {
      // An empty match: step past it (a surrogate pair at once).
      const code = line.charCodeAt(re.lastIndex);
      re.lastIndex += code >= 0xd800 && code <= 0xdbff ? 2 : 1;
      if (re.lastIndex > line.length) break;
      continue;
    }
    out.push([m.index, m.index + m[0].length]);
    if (out.length >= MAX_PER_LINE) break;
  }
  re.lastIndex = 0;
  return out;
}

/** The matches of `re` in a text, line by line, as offsets in the text. */
export function textMatches(text: string, re: RegExp): [number, number][] {
  const out: [number, number][] = [];
  let at = 0;
  for (const line of text.split("\n")) {
    for (const [s, e] of lineMatches(line, re)) out.push([at + s, at + e]);
    at += line.length + 1;
  }
  return out;
}
