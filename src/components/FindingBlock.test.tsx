import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));
// CodeMirror needs a real layout engine: the stub lists where the line
// items go and renders them in place.
vi.mock("./DiffEditor", () => ({
  DiffEditor: (p: {
    path: string;
    items?: {
      key: string;
      side: string;
      line: number;
      render: (onResize: () => void) => React.ReactNode;
    }[];
  }) => (
    <div
      data-testid={`editor-${p.path}`}
      data-items={(p.items ?? [])
        .map((i) => `${i.key}@${i.side}${i.line}`)
        .join(",")}
    >
      {(p.items ?? []).map((i) => (
        <div key={i.key}>{i.render(() => undefined)}</div>
      ))}
    </div>
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import type { Finding } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { clearContentsCache } from "../state/codeViewStore";
import { clearCommentableCache } from "../state/commentStore";
import { aiRun, finding } from "../test/aiFixtures";
import { file, reviewFiles } from "../test/fixtures";
import { FileDiffCard } from "./FileDiffCard";

const mockInvoke = vi.mocked(invoke);
const initial = useAiStore.getState();
const files = reviewFiles();
const ID = files.reviewId;
const f = file({ path: "src/cache.rs" });

let findings: Finding[];

function seed(enabled = true) {
  useAiStore.setState(
    {
      ...initial,
      runs: { [ID]: aiRun({ reviewId: ID, enabled }) },
      byReview: {
        [ID]: {
          findings,
          checklist: [],
          verdicts: [],
          loading: false,
          error: null,
        },
      },
      ui: {},
    },
    true,
  );
}

beforeEach(() => {
  clearContentsCache();
  clearCommentableCache();
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "file_contents":
        // Three lines on either side.
        return Promise.resolve("1\n2\n3");
      case "commentable_lines":
        return Promise.resolve({
          reviewId: ID,
          path: f.path,
          baseSha: files.baseSha,
          headSha: files.headSha,
          hunks: [],
        });
      case "list_findings":
        return Promise.resolve(findings);
      default:
        return Promise.resolve([]);
    }
  }) as typeof invoke);
  findings = [
    finding({
      reviewId: ID,
      path: f.path,
      line: 2,
      severity: "high",
      title: "Unbounded cache",
    }),
    finding({
      reviewId: ID,
      path: f.path,
      line: 1,
      side: "old",
      severity: "low",
      title: "Old side nit",
    }),
    finding({ reviewId: ID, path: f.path, line: 99, title: "Off the diff" }),
    finding({
      reviewId: ID,
      path: "src/other.rs",
      line: 1,
      title: "Other file",
    }),
    finding({
      reviewId: ID,
      path: f.path,
      line: 3,
      title: "Was dismissed",
      dismissed: true,
    }),
  ];
  seed();
});

function renderCard() {
  return render(
    <FileDiffCard
      review={files}
      file={f}
      threads={[]}
      provider="github"
      layout="unified"
      selected
      onSelect={() => undefined}
      onToggleViewed={() => undefined}
      cardRef={() => undefined}
    />,
  );
}

describe("findings in the code view", () => {
  it("sit under their line, and the rest at the top of their file", async () => {
    renderCard();
    const editor = await screen.findByTestId("editor-src/cache.rs");
    expect(editor.dataset.items).toBe(
      `finding:${findings[0].id}@b2,finding:${findings[1].id}@a1`,
    );
    const inline = within(editor).getByRole("article", {
      name: "AI finding: Unbounded cache",
    });
    expect(inline).toHaveAttribute("data-severity", "high");
    expect(within(inline).getByText("AI")).toBeInTheDocument();
    // Not in the diff: listed with where it is; another file's: not here.
    expect(screen.getByText("AI findings on this file")).toBeInTheDocument();
    const off = screen.getByRole("article", {
      name: "AI finding: Off the diff",
    });
    expect(within(off).getByText("src/cache.rs:99")).toBeInTheDocument();
    expect(screen.queryByText("Other file")).toBeNull();
    expect(screen.queryByText("Was dismissed")).toBeNull();
  });

  it("show dismissed ones only on request, with Undismiss", async () => {
    useAiStore.setState({ showDismissed: true });
    renderCard();
    const block = await screen.findByRole("article", {
      name: "AI finding: Was dismissed",
    });
    fireEvent.click(within(block).getByRole("button", { name: "Undismiss" }));
    expect(mockInvoke).toHaveBeenCalledWith("dismiss_finding", {
      findingId: findings[4].id,
      dismissed: false,
    });
  });

  it("select, edit with a preview, and dismiss inline; nothing posts", async () => {
    renderCard();
    const block = await screen.findByRole("article", {
      name: "AI finding: Unbounded cache",
    });
    const id = findings[0].id;
    fireEvent.click(
      within(block).getByRole("checkbox", {
        name: 'Select "Unbounded cache" for posting',
      }),
    );
    expect(useAiStore.getState().ui[id]?.selected).toBe(true);

    fireEvent.click(within(block).getByRole("button", { name: "Edit" }));
    const field = within(block).getByRole("textbox", {
      name: 'Edit finding "Unbounded cache"',
    });
    expect(field).toHaveValue(`**Unbounded cache**\n\n${findings[0].body}`);
    fireEvent.change(field, { target: { value: "**Cap it**\n\nUse an LRU." } });
    fireEvent.click(within(block).getByRole("tab", { name: "Preview" }));
    expect(within(block).getByTestId("composer-preview")).toHaveTextContent(
      "Use an LRU.",
    );
    fireEvent.click(within(block).getByRole("button", { name: "Save edit" }));
    expect(useAiStore.getState().ui[id]?.edit).toBe(
      "**Cap it**\n\nUse an LRU.",
    );
    expect(within(block).getByText("Edited")).toBeInTheDocument();
    expect(within(block).getByText("Use an LRU.")).toBeInTheDocument();

    // Back to the AI's text.
    fireEvent.click(within(block).getByRole("button", { name: "Edit" }));
    fireEvent.click(
      within(block).getByRole("button", { name: "Reset to AI text" }),
    );
    expect(useAiStore.getState().ui[id]?.edit).toBeNull();

    fireEvent.click(within(block).getByRole("button", { name: "Dismiss" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("dismiss_finding", {
        findingId: id,
        dismissed: true,
      }),
    );
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "post_findings",
      expect.anything(),
    );
  });

  it("show a failed post's reason and keep the edit", async () => {
    renderCard();
    const id = findings[0].id;
    useAiStore.setState({
      ui: {
        [id]: {
          selected: true,
          edit: "My text",
          editor: null,
          error: {
            kind: "lineNotInDiff",
            message: "The line is not in the diff any more.",
          },
        },
      },
    });
    const block = await screen.findByRole("article", {
      name: "AI finding: Unbounded cache",
    });
    expect(within(block).getByRole("alert")).toHaveTextContent(
      "Not posted: The line is not in the diff any more.",
    );
    expect(within(block).getByText("My text")).toBeInTheDocument();
    expect(within(block).getByRole("checkbox")).toBeChecked();
  });

  it("are absent with the AI pass off", async () => {
    seed(false);
    renderCard();
    const editor = await screen.findByTestId("editor-src/cache.rs");
    expect(editor.dataset.items).toBe("");
    expect(screen.queryByText("AI findings on this file")).toBeNull();
  });
});
