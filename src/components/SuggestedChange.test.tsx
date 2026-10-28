import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { StoredThread } from "../ipc/types";
import { clearContentsCache } from "../state/codeViewStore";
import { thread } from "../test/fixtures";
import { ThreadBlock } from "./ThreadBlock";

const mockInvoke = vi.mocked(invoke);
const NOW = Date.parse("2026-10-16T12:00:00Z");
const FILE = "fn a() {}\nlet one = 1;\nlet two = 2;\nfn b() {}\n";

function withNote(t: StoredThread, body: string): StoredThread {
  return {
    ...t,
    mine: true,
    notes: [
      {
        id: "n-sugg",
        author: "rev-fixture",
        body,
        createdAt: "2026-10-15T09:00:00Z",
        system: false,
      },
    ],
  };
}

const rows = (el: HTMLElement, kind: "add" | "del") =>
  [...el.querySelectorAll(`[data-kind="${kind}"]`)].map((r) =>
    r.textContent?.replace(/^\d*[-+]/, ""),
  );

beforeEach(() => {
  clearContentsCache();
  mockInvoke.mockImplementation(((cmd: string) =>
    Promise.resolve(cmd === "file_contents" ? FILE : null)) as typeof invoke);
});

describe("SuggestedChange", () => {
  it("shows a GitHub suggestion as removed and added lines from the file", async () => {
    const base = thread();
    const t = withNote(
      { ...base, anchor: { ...base.anchor!, lineNew: 3, startLine: 2 } },
      "Combine these:\n\n```suggestion\nlet both = (1, 2);\n```",
    );
    render(<ThreadBlock thread={t} provider="github" now={NOW} />);
    const box = screen.getByRole("figure", { name: "Suggested change" });
    expect(within(box).getByText("lines 2–3")).toBeInTheDocument();
    expect(await within(box).findByText("let one = 1;")).toBeInTheDocument();
    expect(rows(box, "del")).toEqual(["let one = 1;", "let two = 2;"]);
    expect(rows(box, "add")).toEqual(["let both = (1, 2);"]);
    // Read at the comment's own commit.
    expect(mockInvoke).toHaveBeenCalledWith("file_contents", {
      reviewId: t.reviewId,
      sha: t.anchor!.headSha,
      path: "src/cache.rs",
    });
    // Argus never applies a suggestion.
    expect(
      within(box).queryByRole("button", { name: /apply|commit/i }),
    ).toBeNull();
    expect(screen.getByText("Combine these:")).toBeInTheDocument();
  });

  it("uses GitHub's diff hunk when the file cannot be read", async () => {
    mockInvoke.mockImplementation((() =>
      Promise.reject({ kind: "provider", message: "nope" })) as typeof invoke);
    const base = thread();
    const t = withNote(
      {
        ...base,
        anchor: {
          ...base.anchor!,
          lineNew: 3,
          diffHunk: "@@ -1,2 +1,3 @@\n fn a() {}\n let one = 1;\n+let two = 2;",
        },
      },
      "```suggestion\n```",
    );
    render(<ThreadBlock thread={t} provider="github" now={NOW} />);
    const box = screen.getByRole("figure", { name: "Suggested change" });
    expect(rows(box, "del")).toEqual(["let two = 2;"]);
    // An empty suggestion deletes the line.
    expect(rows(box, "add")).toEqual([]);
    expect(within(box).getByText("deletes the lines")).toBeInTheDocument();
  });

  it("reads GitLab's -N+M range around the anchored line", async () => {
    const base = thread();
    const t = withNote(
      { ...base, anchor: { ...base.anchor!, lineNew: 2 } },
      "```suggestion:-1+1\nfn a() { 1 }\nlet one = 1;\nlet two = 2;\n```",
    );
    render(<ThreadBlock thread={t} provider="gitlab" now={NOW} />);
    const box = screen.getByRole("figure", { name: "Suggested change" });
    expect(within(box).getByText("lines 1–3")).toBeInTheDocument();
    expect(await within(box).findByText("fn a() {}")).toBeInTheDocument();
    expect(rows(box, "del")).toEqual([
      "fn a() {}",
      "let one = 1;",
      "let two = 2;",
    ]);
    expect(rows(box, "add")).toHaveLength(3);
  });

  it("shows only the added lines when the original is unknown", () => {
    const t = withNote(
      { ...thread(), anchor: null },
      "```suggestion\nlet x = 1;\n```",
    );
    render(<ThreadBlock thread={t} provider="github" now={NOW} />);
    const box = screen.getByRole("figure", { name: "Suggested change" });
    expect(
      within(box).getByText("Original lines unavailable"),
    ).toBeInTheDocument();
    expect(rows(box, "del")).toEqual([]);
    expect(rows(box, "add")).toEqual(["let x = 1;"]);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
