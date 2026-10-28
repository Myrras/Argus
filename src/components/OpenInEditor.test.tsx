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
vi.mock("./DiffEditor", () => ({
  DiffEditor: (p: { path: string }) => <div data-testid={`editor-${p.path}`} />,
}));

import { invoke } from "@tauri-apps/api/core";

import type { LocalFile, ReviewFiles } from "../ipc/types";
import { useAppShortcuts } from "../lib/useAppShortcuts";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { clearContentsCache, useCodeViewStore } from "../state/codeViewStore";
import { useFocusStore } from "../state/focusStore";
import { useLayoutStore } from "../state/layoutStore";
import { useLocalFileStore } from "../state/localFileStore";
import { useReviewStore } from "../state/reviewStore";
import { useThreadStore } from "../state/threadStore";
import { useToastStore } from "../state/toastStore";
import { file, review, reviewFiles, settings } from "../test/fixtures";
import { ReviewView } from "./ReviewView";

const mockInvoke = vi.mocked(invoke);
const HEAD = "a".repeat(40);
const r = review({ headSha: HEAD });
const CLONE = "/Users/fixture/widgets";

let listed: ReviewFiles;
/** What `local_files` answers: `null` is "no clone set". */
let clone: { head: string | null; missing: string[] } | null;

function local(path: string): LocalFile {
  return {
    path,
    absolutePath: `${CLONE}/${path}`,
    exists: !clone!.missing.includes(path),
    cloneHead: clone!.head,
    matchesReviewHead: clone!.head === HEAD,
  };
}

function backend(cmd: string, args?: unknown) {
  const a = (args ?? {}) as Record<string, unknown>;
  switch (cmd) {
    case "list_threads":
      return Promise.resolve([]);
    case "list_files":
      return Promise.resolve(listed);
    case "file_contents":
      return Promise.resolve("1\n2\n3");
    case "commentable_lines":
      return Promise.resolve({
        reviewId: r.id,
        path: a.path,
        baseSha: listed.reviewBaseSha,
        headSha: listed.headSha,
        hunks: [],
      });
    case "local_files":
      return Promise.resolve(
        clone ? (a.paths as string[]).map((p) => local(p)) : null,
      );
    default:
      return Promise.resolve(null);
  }
}

const calls = (name: string) =>
  mockInvoke.mock.calls.filter(([cmd]) => cmd === name);

function withClone() {
  useBackendSettingsStore.setState({
    settings: settings({ clonePaths: { [`${r.host}/${r.repo}`]: CLONE } }),
  });
}

function Shortcuts() {
  useAppShortcuts();
  return null;
}

beforeEach(() => {
  localStorage.clear();
  clearContentsCache();
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(backend as typeof invoke);
  listed = reviewFiles({
    reviewId: r.id,
    files: [
      file({ path: "src/cache.rs" }),
      file({ path: "src/old.rs", status: "removed" }),
    ],
  });
  clone = { head: HEAD, missing: ["src/old.rs"] };
  useCodeViewStore.setState({ byReview: {} });
  useThreadStore.setState({ byReview: {} });
  useLocalFileStore.setState({ byReview: {} });
  useFocusStore.setState({ item: null, reviewId: null });
  useToastStore.setState({ toasts: [] });
  useBackendSettingsStore.setState({ settings: settings() });
  useReviewStore.setState({ reviews: [r], selectedId: r.id });
  useLayoutStore.setState({ fileTreeOpen: true, fileListMode: "flat" });
});

const tree = () => screen.getByRole("tree", { name: "Changed files" });
const card = (path: string) =>
  screen.getByRole("region", { name: `Diff of ${path}` });
const openButton = (scope: HTMLElement, path: string) =>
  within(scope).queryByRole("button", { name: `Open ${path} in editor` });
/** The tree's button once the clone's files came in. */
const findOpen = (path: string) =>
  waitFor(() => {
    const b = openButton(tree(), path);
    expect(b).not.toBeNull();
    return b!;
  });

describe("Open in editor", () => {
  it("offers nothing without a clone for the repository", async () => {
    render(<ReviewView review={r} />);
    await screen.findByTestId("editor-src/cache.rs");
    expect(calls("local_files")).toHaveLength(0);
    expect(openButton(tree(), "src/cache.rs")).toBeNull();
    expect(openButton(card("src/cache.rs"), "src/cache.rs")).toBeNull();
  });

  it("offers nothing when the core finds no clone", async () => {
    withClone();
    clone = null;
    render(<ReviewView review={r} />);
    await waitFor(() => expect(calls("local_files")).toHaveLength(1));
    await screen.findByTestId("editor-src/cache.rs");
    expect(openButton(tree(), "src/cache.rs")).toBeNull();
  });

  it("opens a file from the tree and the card header", async () => {
    withClone();
    render(<ReviewView review={r} />);
    const inTree = await findOpen("src/cache.rs");
    expect(calls("local_files")[0][1]).toEqual({
      reviewId: r.id,
      paths: ["src/cache.rs", "src/old.rs"],
    });
    expect(inTree).toBeEnabled();
    expect(inTree).toHaveAttribute("title", "Open in editor (O)");
    fireEvent.click(inTree);
    // A second click while the first is under way opens once.
    fireEvent.click(inTree);
    await waitFor(() => expect(calls("open_in_editor")).toHaveLength(1));
    const inCard = openButton(card("src/cache.rs"), "src/cache.rs")!;
    fireEvent.click(inCard);
    await waitFor(() => expect(calls("open_in_editor")).toHaveLength(2));
    // Keyed by review and repository path: never an absolute path.
    expect(calls("open_in_editor")[0][1]).toEqual({
      reviewId: r.id,
      path: "src/cache.rs",
      useDefault: false,
    });
  });

  it("disables a file that is not in the working tree", async () => {
    withClone();
    render(<ReviewView review={r} />);
    const removed = await findOpen("src/old.rs");
    expect(removed).toBeDisabled();
    expect(removed).toHaveAttribute("title", "Not in your working tree");
    expect(openButton(card("src/old.rs"), "src/old.rs")).toBeDisabled();
  });

  it("hints when the clone is on another commit", async () => {
    withClone();
    clone = { head: "b".repeat(40), missing: [] };
    render(<ReviewView review={r} />);
    const button = await findOpen("src/cache.rs");
    expect(button).toBeEnabled();
    expect(button.title).toBe(
      "Open in editor (O)\nYour clone is on bbbbbbb, not this review's head",
    );
  });

  it("has the file menu on right-click", async () => {
    withClone();
    render(<ReviewView review={r} />);
    await findOpen("src/cache.rs");
    const row = within(tree())
      .getAllByRole("treeitem")
      .find((li) => li.textContent?.includes("src/cache.rs"))!;
    fireEvent.contextMenu(row);
    fireEvent.click(
      await screen.findByRole("menuitem", { name: /Open With Default App/ }),
    );
    await waitFor(() =>
      expect(calls("open_in_editor")[0][1]).toEqual({
        reviewId: r.id,
        path: "src/cache.rs",
        useDefault: true,
      }),
    );
    fireEvent.contextMenu(row);
    fireEvent.click(await screen.findByRole("menuitem", { name: /Show in/ }));
    await waitFor(() =>
      expect(calls("reveal_local_file")[0][1]).toEqual({
        reviewId: r.id,
        path: "src/cache.rs",
      }),
    );
  });

  it("opens the selected file with O and only then", async () => {
    withClone();
    render(
      <>
        <Shortcuts />
        <ReviewView review={r} />
      </>,
    );
    await findOpen("src/cache.rs");
    // On the Conversation: nothing to open.
    useCodeViewStore.getState().select(r.id, "\u0000conversation");
    fireEvent.keyDown(window, { key: "o", code: "KeyO" });
    expect(calls("open_in_editor")).toHaveLength(0);

    useCodeViewStore.getState().select(r.id, "src/cache.rs");
    fireEvent.keyDown(window, { key: "o", code: "KeyO" });
    await waitFor(() => expect(calls("open_in_editor")).toHaveLength(1));

    // A removed file says why instead.
    useCodeViewStore.getState().select(r.id, "src/old.rs");
    fireEvent.keyDown(window, { key: "o", code: "KeyO" });
    expect(calls("open_in_editor")).toHaveLength(1);
    const toasts = useToastStore.getState().toasts;
    expect(toasts[toasts.length - 1]?.message).toBe("Not in your working tree");
  });
});
