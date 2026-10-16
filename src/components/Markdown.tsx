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
 *   navigates.
 * - Images are links, not loaded: the CSP blocks remote images, and most
 *   comment images sit behind the platform's login anyway.
 * - Fenced code is highlighted by CodeMirror (`CodeEditorBlock`, lazy; the
 *   plain text shows until it loads), compact, with a copy button.
 * - `renderFence` lets the caller take over a fence by its info string (a
 *   thread renders ` ```suggestion ` blocks as a diff).
 */

import {
  createContext,
  lazy,
  Suspense,
  useContext,
  useEffect,
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

import { openExternal } from "../lib/links";
import { CheckIcon, CopyIcon, ImageIcon } from "./icons";
import styles from "./Markdown.module.css";

const CodeEditorBlock = lazy(() =>
  import("./CodeEditorBlock").then((m) => ({ default: m.CodeEditorBlock })),
);

/** Takes over a fenced block: `info` is its info string (may be ""). */
export type RenderFence = (info: string, code: string) => ReactNode | null;

function MarkdownLink({
  href,
  children,
  node: _node,
  ...rest
}: AnchorHTMLAttributes<HTMLAnchorElement> & ExtraProps) {
  return (
    <a
      {...rest}
      href={href}
      title={href}
      target="_blank"
      rel="noreferrer"
      onClick={(e) => {
        e.preventDefault();
        if (href) openExternal(href);
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
  const href = typeof src === "string" ? src : "";
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
export function CodeBlock({ info, code }: { info: string; code: string }) {
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
        <CodeEditorBlock code={code} info={info} />
      </Suspense>
    </div>
  );
}

/** The caller's fence renderer, for the module-level `pre` below. */
const FenceContext = createContext<RenderFence | undefined>(undefined);

function MarkdownPre({ node }: ExtraProps) {
  const renderFence = useContext(FenceContext);
  const { info, code } = fenceOf(node);
  return renderFence?.(info, code) ?? <CodeBlock info={info} code={code} />;
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
}: {
  children: string;
  renderFence?: RenderFence;
}) {
  return (
    <div className={styles.prose}>
      <FenceContext.Provider value={renderFence}>
        <ReactMarkdown remarkPlugins={REMARK} components={COMPONENTS}>
          {children}
        </ReactMarkdown>
      </FenceContext.Provider>
    </div>
  );
}
