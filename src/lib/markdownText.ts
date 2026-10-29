/**
 * A comment's visible text, for the find bar (`lib/findInReview`), and the
 * rehype plugin that marks the find matches in it (`Markdown`).
 *
 * Both walk the same tree: the hast that `react-markdown` renders (remark
 * + remark-gfm + remark-rehype with `allowDangerousHtml`, its own options).
 * Raw HTML (an HTML comment such as Argus' hidden `<!-- argus:reply-to … -->`
 * marker, a `<details>` tag) is a `raw` node there; react-markdown prints
 * it as literal text after the plugins ran, but it is markup, not what the
 * comment says, so neither the search nor the marks count it. The markdown
 * markers (`**`, `#`, link targets) are not text nodes at all. The text is
 * the text nodes in document order, so a match's offset in it is the same
 * for the search and the highlighting.
 *
 * Fenced code counts (it is shown), but CodeMirror renders it
 * (`CodeEditorBlock`), not the text nodes: the plugin leaves the code's
 * text alone and tells the `pre` where its text starts in the comment
 * (`data-find-base`), so the code block highlights its own matches
 * (`lib/findHighlight`).
 */

import type { Element, ElementContent, Root, RootContent, Text } from "hast";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";

import { textMatches } from "./findQuery";

const processor = unified()
  .use(remarkParse)
  .use(remarkGfm)
  .use(remarkRehype, { allowDangerousHtml: true });

/** The hast tree `react-markdown` renders for `body`. */
function markdownTree(body: string): Root {
  return processor.runSync(processor.parse(body)) as Root;
}

type Parent = Root | Element;
type Child = RootContent | ElementContent;

/** The text nodes of `tree` in order, each with its parent and offset. */
interface TextSpot {
  node: Text;
  parent: Parent;
  start: number;
  /** Inside a `pre` (CodeMirror renders it, not the text nodes). */
  code: boolean;
}

function collect(tree: Root): {
  text: string;
  spots: TextSpot[];
  pres: Map<Element, number>;
} {
  const spots: TextSpot[] = [];
  const pres = new Map<Element, number>();
  let text = "";
  const walk = (parent: Parent, code: boolean) => {
    for (const child of parent.children as Child[]) {
      if (child.type === "text") {
        spots.push({ node: child, parent, start: text.length, code });
        text += child.value;
      } else if (child.type === "element") {
        const pre = child.tagName === "pre";
        if (pre) pres.set(child, text.length);
        walk(child, code || pre);
      }
      // `raw` (HTML) and `comment` nodes: never shown.
    }
  };
  walk(tree, false);
  return { text, spots, pres };
}

const cache = new Map<string, string>();
const MAX_CACHED = 4000;

/** The text a comment shows (see the module comment); cached by body. */
export function markdownText(body: string): string {
  let text = cache.get(body);
  if (text === undefined) {
    text = collect(markdownTree(body)).text;
    cache.set(body, text);
    if (cache.size > MAX_CACHED) {
      const oldest = cache.keys().next().value;
      if (oldest !== undefined) cache.delete(oldest);
    }
  }
  return text;
}

export interface FindMarksOptions {
  re: RegExp;
  /** The match (by its index among the comment's matches) to stand out. */
  current: number | null;
}

/** One `<mark>` around a match's part. */
function mark(value: string, ordinal: number, current: boolean): Element {
  return {
    type: "element",
    tagName: "mark",
    properties: {
      className: ["argus-find-match"],
      dataFindMatch: String(ordinal),
      ...(current ? { dataFindCurrent: "true" } : {}),
    },
    children: [{ type: "text", value }],
  };
}

/**
 * Wrap every match of `re` in the tree's text in `<mark>` (a match that
 * spans a link or emphasis edge gets a mark per part, so links keep
 * working), the current one with `data-find-current`. Code in a `pre` gets
 * `data-find-base` instead.
 */
export function markMatches(tree: Root, opts: FindMarksOptions): void {
  const { text, spots, pres } = collect(tree);
  const matches = textMatches(text, opts.re);
  for (const [pre, base] of pres) {
    pre.properties = { ...pre.properties, dataFindBase: String(base) };
  }
  if (matches.length === 0) return;
  let first = 0;
  // Replacements per parent, applied after the walk (indices stay valid).
  const replaced = new Map<Text, Child[]>();
  for (const spot of spots) {
    const from = spot.start;
    const to = from + spot.node.value.length;
    while (first < matches.length && matches[first][1] <= from) first++;
    if (spot.code) continue;
    const parts: Child[] = [];
    let at = from;
    for (let k = first; k < matches.length && matches[k][0] < to; k++) {
      const s = Math.max(matches[k][0], from);
      const e = Math.min(matches[k][1], to);
      if (s > at) parts.push({ type: "text", value: text.slice(at, s) });
      parts.push(mark(text.slice(s, e), k, k === opts.current));
      at = e;
    }
    if (parts.length === 0) continue;
    if (at < to) parts.push({ type: "text", value: text.slice(at, to) });
    replaced.set(spot.node, parts);
  }
  const apply = (parent: Parent) => {
    const out: Child[] = [];
    for (const child of parent.children as Child[]) {
      const parts = child.type === "text" ? replaced.get(child) : undefined;
      if (parts) out.push(...parts);
      else {
        if (child.type === "element") apply(child);
        out.push(child);
      }
    }
    (parent as { children: Child[] }).children = out;
  };
  apply(tree);
}

/** The rehype plugin `Markdown` adds while the find bar has a query. */
export function rehypeFindMarks(opts: FindMarksOptions) {
  return (tree: Root) => markMatches(tree, opts);
}
