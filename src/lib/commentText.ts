/**
 * Plain text from a markdown comment body, for the one-line preview of a
 * folded thread: code fences become a short label, links and images keep
 * their text, the markup characters go. Pure.
 */

/** The first `max` characters of `body` as plain text (with "…" if cut). */
export function previewText(body: string, max = 90): string {
  const text = body
    // Fenced blocks: a suggestion says so, other code collapses to a label.
    .replace(
      /^(```|~~~)\s*([^\n]*)\n[\s\S]*?(^\1[ \t]*$|$(?![\s\S]))/gm,
      (_m, _f, info: string) =>
        /^suggestion\b/.test(info.trim()) ? " [suggested change] " : " [code] ",
    )
    // Images and links: their text.
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    // Tables: drop the separator row, the cell pipes become spaces.
    .replace(/^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/gm, "")
    .replace(/\|/g, " ")
    // Task boxes, headings, quotes, list markers at a line start.
    .replace(/^\s*[-*+]\s+\[[ xX]\]\s+/gm, "")
    .replace(/^\s{0,3}(#{1,6}|>+|[-*+]|\d+[.)])\s+/gm, "")
    // Emphasis and inline code markers.
    .replace(/(\*\*|__|~~|`)/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
