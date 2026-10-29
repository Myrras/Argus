import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const { openUrlMock } = vi.hoisted(() => ({
  openUrlMock: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: openUrlMock }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import type { FindMatch } from "../lib/findInReview";
import { compileFind } from "../lib/findQuery";
import { markdownText } from "../lib/markdownText";
import { resetFindForTests, useFindStore } from "../state/findStore";
import { FindText, Markdown } from "./Markdown";

const UNIT = "note:t1:n1";

/** The bar open on `query`, the `current`-th match of `unit` current. */
function findOn(query: string, unit: string, current: number | null) {
  const re = compileFind(query, {
    caseSensitive: false,
    wholeWord: false,
    regex: false,
  }).re;
  const m = {
    kind: "note",
    card: "x",
    unit,
    ordinal: current ?? 0,
    start: 0,
    end: 1,
    row: 0,
  } as FindMatch;
  act(() => {
    useFindStore.setState({
      open: true,
      re,
      scope: "everything",
      matches: current === null ? [] : [m],
      current: current === null ? -1 : 0,
    });
  });
}

afterEach(() => {
  act(() => resetFindForTests());
});

describe("find marks in comment text", () => {
  it("marks every match, the current one standing out, and keeps links working", () => {
    const body =
      "Use [the cache](https://example.test/c) — cache it **cached**.";
    findOn("cache", UNIT, 1);
    const { container } = render(<Markdown findUnit={UNIT}>{body}</Markdown>);
    const marks = [...container.querySelectorAll("mark.argus-find-match")];
    expect(marks.map((m) => m.textContent)).toEqual([
      "cache",
      "cache",
      "cache",
    ]);
    expect(marks.map((m) => m.getAttribute("data-find-current"))).toEqual([
      null,
      "true",
      null,
    ]);
    // The first is inside the link, which still opens in the browser.
    const link = screen.getByRole("link", { name: "the cache" });
    expect(link.querySelector("mark")).not.toBeNull();
    expect(fireEvent.click(link)).toBe(false);
    expect(openUrlMock).toHaveBeenCalledWith("https://example.test/c");
    // As many as the search counts in the same text.
    expect(markdownText(body).match(/cache/gi)).toHaveLength(3);
  });

  it("marks a match across an emphasis edge in parts, one match", () => {
    findOn("ab", UNIT, 0);
    const { container } = render(
      <Markdown findUnit={UNIT}>{"a**b**"}</Markdown>,
    );
    const marks = [...container.querySelectorAll("mark")];
    expect(marks.map((m) => m.textContent)).toEqual(["a", "b"]);
    expect(marks.every((m) => m.getAttribute("data-find-match") === "0")).toBe(
      true,
    );
  });

  it("shows nothing for another unit, the code scope or a closed bar", () => {
    findOn("cache", "note:other", 0);
    const { container, rerender } = render(
      <Markdown findUnit={UNIT}>{"cache"}</Markdown>,
    );
    // Marked (all matches), but none current here.
    expect(container.querySelector("mark")).not.toBeNull();
    expect(container.querySelector("[data-find-current]")).toBeNull();
    act(() => useFindStore.setState({ scope: "code" }));
    rerender(<Markdown findUnit={UNIT}>{"cache"}</Markdown>);
    expect(container.querySelector("mark")).toBeNull();
    act(() => useFindStore.setState({ scope: "everything", open: false }));
    rerender(<Markdown findUnit={UNIT}>{"cache"}</Markdown>);
    expect(container.querySelector("mark")).toBeNull();
  });

  it("marks plain text (an author, a title)", () => {
    findOn("bob", "thread:t1", 0);
    const { container } = render(
      <FindText text="bob-fixture" unit="thread:t1" />,
    );
    expect(container.querySelector("mark")).toHaveTextContent("bob");
    expect(container.querySelector("mark")).toHaveAttribute(
      "data-find-current",
      "true",
    );
  });
});
