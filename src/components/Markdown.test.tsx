import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import { describe, expect, it, vi } from "vitest";

const { openUrlMock } = vi.hoisted(() => ({
  openUrlMock: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: openUrlMock }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
}));

import { languageForInfo } from "../lib/codemirror";
import { Markdown } from "./Markdown";

// jsdom has no layout: CodeMirror asks ranges for their rectangles.
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () =>
    ({
      length: 0,
      item: () => null,
      [Symbol.iterator]: [][Symbol.iterator],
    }) as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

describe("Markdown", () => {
  it("opens links in the browser instead of navigating the webview", () => {
    render(<Markdown>{"See [the issue](https://example.test/i/1)."}</Markdown>);
    const link = screen.getByRole("link", { name: "the issue" });
    expect(link).toHaveAttribute("target", "_blank");
    // preventDefault() was called: the webview never navigates.
    expect(fireEvent.click(link)).toBe(false);
    expect(openUrlMock).toHaveBeenCalledWith("https://example.test/i/1");
  });

  it("drops unsafe links", () => {
    render(<Markdown>{"[click](javascript:alert(1))"}</Markdown>);
    fireEvent.click(screen.getByText("click"));
    expect(openUrlMock).not.toHaveBeenCalled();
  });

  it("renders headings, lists, quotes, tables and read-only task lists", () => {
    const { container } = render(
      <Markdown>
        {[
          "## Plan",
          "",
          "- [x] tests",
          "- [ ] docs",
          "",
          "> quoted",
          "",
          "| a | b |",
          "| - | - |",
          "| 1 | 2 |",
          "",
          "Use `cache.get()`.",
        ].join("\n")}
      </Markdown>,
    );
    expect(screen.getByRole("heading", { name: "Plan" })).toBeInTheDocument();
    const boxes = screen.getAllByRole("checkbox");
    expect(boxes).toHaveLength(2);
    expect(boxes[0]).toBeChecked();
    expect(boxes[1]).not.toBeChecked();
    for (const b of boxes) expect(b).toBeDisabled();
    expect(container.querySelector("blockquote")).toHaveTextContent("quoted");
    expect(screen.getByRole("table")).toBeInTheDocument();
    expect(container.querySelector("code")).toHaveTextContent("cache.get()");
  });

  it("never renders raw HTML", () => {
    const { container } = render(
      <Markdown>
        {
          'Hi <script>window.pwned = 1</script> <img src="x" onerror="1"> <b>bold</b>'
        }
      </Markdown>,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("b")).toBeNull();
    // Shown as the text it is.
    expect(container).toHaveTextContent("<b>bold</b>");
    expect((window as { pwned?: number }).pwned).toBeUndefined();
  });

  it("shows images as links, without loading them", () => {
    const { container } = render(
      <Markdown>{"![diagram](https://example.test/d.png)"}</Markdown>,
    );
    expect(container.querySelector("img")).toBeNull();
    fireEvent.click(screen.getByRole("link", { name: "diagram" }));
    expect(openUrlMock).toHaveBeenCalledWith("https://example.test/d.png");
  });

  it("highlights a fenced block by its language, with a copy button", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const { container } = render(
      <Markdown>{'```jq\n.items[] | select(.ok) | "done"\n```'}</Markdown>,
    );
    expect(screen.getByText("jq")).toBeInTheDocument();
    // CodeMirror replaces the plain fallback, with highlighted tokens.
    await waitFor(() =>
      expect(
        container.querySelector(
          '.argus-code-block[data-language="jq"] .cm-editor',
        ),
      ).not.toBeNull(),
    );
    await waitFor(() =>
      expect(
        container.querySelectorAll(".cm-line span").length,
      ).toBeGreaterThan(0),
    );
    expect(container.querySelector(".cm-content")).toHaveTextContent(
      '.items[] | select(.ok) | "done"',
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy code" }));
    expect(writeText).toHaveBeenCalledWith('.items[] | select(.ok) | "done"');
    expect(
      await screen.findByRole("button", { name: "Copied" }),
    ).toBeInTheDocument();
  });

  it("hands fences to renderFence by their info string", () => {
    render(
      <Markdown
        renderFence={(info, code) =>
          info.startsWith("suggestion") ? (
            <p>
              custom {info}: {code}
            </p>
          ) : null
        }
      >
        {"```suggestion:-1+0\nlet x = 1;\n```\n\n```ts\nconst y = 2;\n```"}
      </Markdown>,
    );
    expect(
      screen.getByText("custom suggestion:-1+0: let x = 1;"),
    ).toBeInTheDocument();
    // Other fences stay code blocks.
    expect(screen.getByText("ts")).toBeInTheDocument();
  });
});

describe("languageForInfo", () => {
  it("finds the language by name, alias or extension; jq has its own", async () => {
    expect((await languageForInfo("ts"))?.language.name).toBe("typescript");
    expect((await languageForInfo("rust"))?.language.name).toBe("rust");
    expect((await languageForInfo("py"))?.language.name).toBe("python");
    expect((await languageForInfo("jq"))?.language.name).toBe("jq");
    expect(await languageForInfo("no-such-language")).toBeNull();
    expect(await languageForInfo("")).toBeNull();
  });
  it("keeps code blocks mounted when it renders again", async () => {
    let mounts = 0;
    function Fence({ code }: { code: string }) {
      useEffect(() => {
        mounts += 1;
      }, []);
      return <p>fence {code}</p>;
    }
    const body = "```suggestion\nnew()\n```\n\n```rust\nfn a() {}\n```";
    // A new renderFence each render, as a thread card makes one.
    const view = () => (
      <Markdown
        renderFence={(info, code) =>
          info === "suggestion" ? <Fence code={code} /> : null
        }
      >
        {body}
      </Markdown>
    );
    const { container, rerender } = render(view());
    await screen.findByText("fence new()");
    const block = container.querySelector("[class*=fence]");
    rerender(view());
    rerender(view());
    expect(mounts).toBe(1);
    expect(container.querySelector("[class*=fence]")).toBe(block);
  });
});
