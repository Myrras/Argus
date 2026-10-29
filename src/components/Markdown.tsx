/**
 * A comment body as markdown, ported from Podium's `Markdown`
 * (`react-markdown` + `remark-gfm`: tables, task lists, strikethrough,
 * autolinks). react-markdown builds real DOM nodes — no
 * `dangerouslySetInnerHTML`, no `rehype-raw` — so HTML in a comment shows as
 * text and never runs: comment text is untrusted. Its default URL transform
 * drops `javascript:` and other unsafe links.
 *
 * Argus additions:
 * - Links open in the browser through the opener plugin; the webview never
 *   navigates. Relative ones resolve against the open review's page
 *   (`resolveLink`), footnotes jump inside the comment, and a link that
 *   resolves to nothing http(s) shows as text.
 * - Images are links, not loaded: the CSP blocks remote images, and most
 *   comment images sit behind the platform's login anyway.
 * - Fenced code is highlighted by CodeMirror (`CodeEditorBlock`, lazy; the
 *   plain text shows until it loads), compact, with a copy button.
 * - `renderFence` lets the caller take over a fence by its info string (a
 *   thread renders ` ```suggestion ` blocks as a diff).
 * - `findUnit`: the find bar's matches in this text are marked
 *   (`lib/markdownText.rehypeFindMarks`: `<mark>` in the text nodes, so
 *   links keep working; a fence's code block highlights its own), the
 *   current one standing out. The wrapper carries `data-find-unit`.
 */

import {
  createContext,
  lazy,
  Suspense,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type AnchorHTMLAttributes,
  type ImgHTMLAttributes,
  type ReactNode,
} from "react";
import ReactMarkdown, {
  type Components,
  type ExtraProps,
} from "react-markdown";
import remarkGfm from "remark-gfm";

import { openExternal, resolveLink, type LinkBase } from "../lib/links";
import { textMatches } from "../lib/findQuery";
import { rehypeFindMarks } from "../lib/markdownText";
import { useFindMarks } from "../state/findStore";
import { useReviewStore } from "../state/reviewStore";
import { CheckIcon, CopyIcon, ImageIcon } from "./icons";
import styles from "./Markdown.module.css";

const CodeEditorBlock = lazy(() =>
  import("./CodeEditorBlock").then((m) => ({ default: m.CodeEditorBlock })),
);

/** Takes over a fenced block: `info` is its info string (may be ""). */
export type RenderFence = (info: string, code: string) => ReactNode | null;

/** The open review's page, for relative links (a store: widget roots). */
function useLinkBase(): LinkBase | null {
  const url = useReviewStore(
    (s) => s.reviews.find((r) => r.id === s.selectedId)?.url ?? null,
  );
  const provider = useReviewStore(
    (s) => s.reviews.find((r) => r.id === s.selectedId)?.provider ?? null,
  );
  return url && provider ? { url, provider } : null;
}

/** react-markdown's prefix for footnote and heading ids. */
const LOCAL = "#user-content-";

function MarkdownLink({
  href,
  children,
  node: _node,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement> & ExtraProps) {
  const base = useLinkBase();
  const raw = href ?? "";
  const local = raw.startsWith(LOCAL);
  const target = local ? null : resolveLink(raw, base);
  if (!local && !target) {
    return <span title={raw || undefined}>{children}</span>;
  }
  return (
    <a
      {...rest}
      href={target ?? raw}
      title={target ?? undefined}
      target={local ? undefined : "_blank"}
      rel="noreferrer"
      onClick={(e) => {
        e.preventDefault();
        if (target) {
          openExternal(target);
          return;
        }
        // A footnote (or its way back): within this comment.
        const id = raw.slice(1);
        const prose = e.currentTarget.closest(`.${styles.prose}`);
        const to = [...(prose?.querySelectorAll("[id]") ?? [])].find(
          (el) => el.id === id,
        );
        to?.scrollIntoView?.({ block: "nearest" });
      }}
    >
      {children}
    </a>
  );
}

function MarkdownImage({
  src,
  alt,
  node: _node,
}: ImgHTMLAttributes<HTMLImageElement> & ExtraProps) {
  const base = useLinkBase();
  const href = resolveLink(typeof src === "string" ? src : "", base);
  if (!href) {
    return (
      <span className={styles.image}>
        <ImageIcon />
        {alt || "Image"}
      </span>
    );
  }
  return (
    <a
      className={styles.image}
      href={href}
      title={href}
      target="_blank"
      rel="noreferrer"
      onClick={(e) => {
        e.preventDefault();
        if (href) openExternal(href);
      }}
    >
      <ImageIcon />
      {alt || "Image"}
    </a>
  );
}

type HastNode = NonNullable<ExtraProps["node"]>;
type HastChild = HastNode["children"][number];

function textOf(node: HastChild | HastNode): string {
  if (node.type === "text") return node.value;
  if ("children" in node) return node.children.map(textOf).join("");
  return "";
}

/** A fence's info string and text from its `<pre><code>` (hast). */
function fenceOf(pre: HastNode | undefined): { info: string; code: string } {
  const code = pre?.children.find(
    (c): c is HastNode => c.type === "element" && c.tagName === "code",
  );
  if (!code) return { info: "", code: "" };
  const classes = code.properties.className;
  const lang = (Array.isArray(classes) ? classes : [])
    .map(String)
    .find((c) => c.startsWith("language-"));
  const meta =
    typeof code.data === "object" && code.data && "meta" in code.data
      ? String((code.data as { meta?: unknown }).meta ?? "")
      : "";
  const info = [lang ? lang.slice("language-".length) : "", meta]
    .filter(Boolean)
    .join(" ");
  // The fence's text ends with the newline before the closing fence.
  return { info, code: textOf(code).replace(/\n$/, "") };
}

/** A fenced code block: label, copy button, highlighted text. */
export function CodeBlock({
  info,
  code,
  find = null,
}: {
  info: string;
  code: string;
  /** The comment's find unit and where this code starts in its text. */
  find?: { unit: string; base: number } | null;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const lang = info.split(/\s+/)[0] ?? "";
  const copy = () => {
    void navigator.clipboard?.writeText(code).then(() => {
      setCopied(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <div className={styles.fence}>
      <div className={styles.fenceBar}>
        <span className={`${styles.fenceLang} mono`}>{lang}</span>
        <button
          type="button"
          className={`ghost ${styles.copy}`}
          onClick={copy}
          aria-label={copied ? "Copied" : "Copy code"}
          title={copied ? "Copied" : "Copy code"}
        >
          {copied ? <CheckIcon size={12} /> : <CopyIcon />}
        </button>
      </div>
      <Suspense fallback={<pre className={`${styles.plain} mono`}>{code}</pre>}>
        <CodeEditorBlock code={code} info={info} find={find} />
      </Suspense>
    </div>
  );
}

/** The caller's fence renderer, for the module-level `pre` below. */
const FenceContext = createContext<RenderFence | undefined>(undefined);
/** The text's find unit, for a fence's code block. */
const FindUnitContext = createContext<string | undefined>(undefined);

function MarkdownPre({ node }: ExtraProps) {
  const renderFence = useContext(FenceContext);
  const unit = useContext(FindUnitContext);
  const { info, code } = fenceOf(node);
  const base = Number(node?.properties.dataFindBase);
  const find = unit && Number.isFinite(base) ? { unit, base } : null;
  return (
    renderFence?.(info, code) ?? (
      <CodeBlock info={info} code={code} find={find} />
    )
  );
}

/**
 * One components object for every render: react-markdown takes each entry
 * as a component type, so a new `pre` per render would remount every code
 * block (its CodeMirror) and suggested change on each re-render.
 */
const COMPONENTS: Components = {
  a: MarkdownLink,
  img: MarkdownImage,
  pre: MarkdownPre,
};
const REMARK = [remarkGfm];

export function Markdown({
  children,
  renderFence,
  findUnit,
}: {
  children: string;
  renderFence?: RenderFence;
  /** The find bar's unit of this text (`lib/findInReview`). */
  findUnit?: string;
}) {
  const marks = useFindMarks(findUnit);
  const rehype = useMemo(
    () => (marks ? [[rehypeFindMarks, marks] as const] : undefined),
    [marks],
  );
  return (
    <div className={styles.prose} data-find-unit={findUnit}>
      <FenceContext.Provider value={renderFence}>
        <FindUnitContext.Provider value={findUnit}>
          <ReactMarkdown
            remarkPlugins={REMARK}
            rehypePlugins={rehype as never}
            components={COMPONENTS}
          >
            {children}
          </ReactMarkdown>
        </FindUnitContext.Provider>
      </FenceContext.Provider>
    </div>
  );
}

/**
 * Plain text (a thread's author, a finding's title) with the find bar's
 * matches marked, as `Markdown` marks a comment's.
 */
export function FindText({ text, unit }: { text: string; unit: string }) {
  const marks = useFindMarks(unit);
  if (!marks) return <>{text}</>;
  const parts: ReactNode[] = [];
  let at = 0;
  textMatches(text, marks.re).forEach(([s, e], k) => {
    if (s > at) parts.push(text.slice(at, s));
    parts.push(
      <mark
        key={k}
        className="argus-find-match"
        data-find-match={k}
        data-find-current={k === marks.current ? "true" : undefined}
      >
        {text.slice(s, e)}
      </mark>,
    );
    at = e;
  });
  if (at < text.length) parts.push(text.slice(at));
  return <span data-find-unit={unit}>{parts}</span>;
}
