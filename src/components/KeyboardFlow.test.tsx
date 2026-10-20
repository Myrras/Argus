import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/platform", () => ({ isMac: false, isWindows: false }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import { useAppShortcuts } from "../lib/useAppShortcuts";
import { usePageFocus } from "../lib/usePageFocus";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import { useCommentStore } from "../state/commentStore";
import { useFocusStore } from "../state/focusStore";
import { useHistoryStore } from "../state/historyStore";
import { useLocalFileStore } from "../state/localFileStore";
import { useOverlayStore } from "../state/overlayStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useSettingsStore } from "../state/settingsStore";
import { useThreadStore } from "../state/threadStore";
import { file, review, reviewFiles, thread } from "../test/fixtures";
import { CommandPalette } from "./CommandPalette";
import { ReviewDialogs } from "./ReviewDialogs";
import { ShortcutHelp } from "./ShortcutHelp";
import { ThreadBlock } from "./ThreadBlock";

const NOW = Date.parse("2026-10-16T12:00:00Z");
const r = review({ title: "Fixture cache change" });
const other = review({ title: "Fixture docs tweak", status: "needsReReview" });
const t = thread({ reviewId: r.id, author: "bob-fixture", anchor: null });
const mockInvoke = vi.mocked(invoke);

/** The writes; the palette never runs one by itself. */
const WRITES = [
  "post_comments",
  "reply_to_thread",
  "approve_review",
  "submit_review",
  "post_findings",
  "resolve_thread",
];
const noWrites = () => {
  for (const cmd of WRITES) {
    expect(mockInvoke).not.toHaveBeenCalledWith(cmd, expect.anything());
  }
};

function Shell() {
  useAppShortcuts();
  usePageFocus(r.id);
  return (
    <>
      <CommandPalette />
      <ReviewDialogs />
      <ShortcutHelp />
    </>
  );
}

function press(init: KeyboardEventInit & { key: string }) {
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  });
}

function openPalette() {
  press({ key: "k", code: "KeyK", ctrlKey: true });
  return screen.getByRole("combobox");
}

beforeEach(() => {
  localStorage.clear();
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(() => Promise.resolve([]));
  useSettingsStore.getState().resetShortcuts();
  useOverlayStore.setState({ open: null, confirm: null });
  useSettingsRequestStore.getState().clear();
  useCommentStore.setState({ drafts: {}, composer: null, replies: {} });
  useReviewStore.setState({
    reviews: [r, other],
    selectedId: r.id,
    cursorId: r.id,
    tab: "needsReview",
  });
  useCodeViewStore.setState({
    byReview: {
      [r.id]: {
        since: false,
        files: reviewFiles({
          reviewId: r.id,
          files: [
            file({ path: "src/cache.rs" }),
            file({ path: "docs/readme.md" }),
          ],
        }),
        loading: false,
        error: null,
        selected: null,
        order: [CONVERSATION, "src/cache.rs", "docs/readme.md"],
      },
    },
  });
  useThreadStore.setState({
    byReview: { [r.id]: { threads: [t], loading: false, error: null } },
  });
  useFocusStore.setState({ reviewId: r.id, item: null, zone: "page" });
  useHistoryStore.setState({ open: {} });
});

describe("ThreadBlock and the focus", () => {
  it("shows the ring, opens on Enter and on the key, replies on r", async () => {
    render(
      <>
        <Shell />
        <ThreadBlock thread={t} provider="github" now={NOW} />
      </>,
    );
    const card = screen.getByRole("article", { name: "Thread by bob-fixture" });
    expect(card).toHaveAccessibleDescription(
      "1 comment, on the review as a whole",
    );
    expect(card).toHaveAttribute("data-focused", "false");
    expect(card).toHaveAttribute("data-open", "false");

    press({ key: "n", code: "KeyN" });
    expect(card).toHaveAttribute("data-focused", "true");
    // The page puts the DOM focus on it (screen readers announce it).
    await waitFor(() => expect(card).toHaveFocus());

    fireEvent.keyDown(card, { key: "Enter" });
    expect(card).toHaveAttribute("data-open", "true");
    press({ key: "t", code: "KeyT" });
    expect(card).toHaveAttribute("data-open", "false");

    press({ key: "r", code: "KeyR" });
    expect(card).toHaveAttribute("data-open", "true");
    const field = await screen.findByRole("textbox", { name: "Reply" });
    expect(field).toHaveFocus();
    // Escape cancels; nothing was posted.
    fireEvent.keyDown(field, { key: "Escape" });
    expect(screen.queryByRole("textbox", { name: "Reply" })).toBeNull();
    noWrites();
  });

  it("Enter after ↑/↓ in the list is the list's", () => {
    render(<ThreadBlock thread={t} provider="github" now={NOW} />);
    const card = screen.getByRole("article", { name: "Thread by bob-fixture" });
    act(() =>
      useFocusStore.getState().focusItem(r.id, {
        kind: "thread",
        threadId: t.threadId,
        mine: false,
        card: CONVERSATION,
        line: 0,
        side: null,
      }),
    );
    act(() => useFocusStore.getState().setZone("list"));
    fireEvent.keyDown(card, { key: "Enter" });
    expect(card).toHaveAttribute("data-open", "false");
  });
});

describe("Enter and t outside the review list", () => {
  it("t folds the focused thread even with a link focused", () => {
    render(
      <>
        <Shell />
        <ThreadBlock thread={t} provider="github" now={NOW} />
      </>,
    );
    const card = screen.getByRole("article", { name: "Thread by bob-fixture" });
    press({ key: "n", code: "KeyN" });
    const link = document.createElement("a");
    link.href = "#fixture";
    link.textContent = "fixture link";
    document.body.appendChild(link);
    link.focus();
    fireEvent.keyDown(link, { key: "t", code: "KeyT" });
    expect(card).toHaveAttribute("data-open", "true");
    link.remove();
  });

  it("Enter on a checkbox never opens the review under the list cursor", () => {
    render(<Shell />);
    act(() => useReviewStore.setState({ cursorId: other.id }));
    const box = document.createElement("input");
    box.type = "checkbox";
    document.body.appendChild(box);
    box.focus();
    fireEvent.keyDown(box, { key: "Enter", code: "Enter" });
    expect(useReviewStore.getState().selectedId).toBe(r.id);
    box.remove();
  });
});

describe("CommandPalette", () => {
  it("lists the actions with their current shortcuts and runs one", () => {
    render(<Shell />);
    const field = openPalette();
    expect(
      screen.getByRole("dialog", { name: "Command palette" }),
    ).toBeVisible();
    // The field has the focus on open, so typing filters at once.
    expect(field).toHaveFocus();
    fireEvent.change(field, { target: { value: "next file" } });
    const first = screen.getAllByRole("option")[0];
    expect(first).toHaveTextContent("Next file");
    expect(within(first).getByText("J")).toBeInTheDocument();
    expect(first).toHaveAttribute("aria-selected", "true");
    expect(field).toHaveAttribute("aria-activedescendant", first.id);
    fireEvent.keyDown(field, { key: "Enter" });
    expect(useOverlayStore.getState().open).toBeNull();
    expect(useCodeViewStore.getState().byReview[r.id].selected).toBe(
      CONVERSATION,
    );
  });

  it("refreshes the open review alone from “Refresh this review”", () => {
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "refresh this review" } });
    const first = screen.getAllByRole("option")[0];
    expect(first).toHaveTextContent("Refresh this review");
    expect(within(first).getByText("Alt+R")).toBeInTheDocument();
    fireEvent.keyDown(field, { key: "Enter" });
    expect(mockInvoke).toHaveBeenCalledWith("refresh_review", {
      reviewId: r.id,
    });
    expect(mockInvoke).not.toHaveBeenCalledWith("refresh_reviews");
    noWrites();
  });

  it("offers “Refresh this review” only with a review open", () => {
    useReviewStore.setState({ selectedId: null });
    render(<Shell />);
    fireEvent.change(openPalette(), {
      target: { value: "refresh this review" },
    });
    expect(
      screen
        .queryAllByRole("option")
        .some((o) => o.textContent?.includes("Refresh this review")),
    ).toBe(false);
  });

  it("shows a rebound shortcut", () => {
    useSettingsStore.getState().setShortcut("nextFile", ["Mod+KeyJ"]);
    render(<Shell />);
    fireEvent.change(openPalette(), { target: { value: "next file" } });
    expect(
      within(screen.getAllByRole("option")[0]).getByText("Ctrl+J"),
    ).toBeInTheDocument();
  });

  it("filters fuzzily; ↑/↓ move the active option", () => {
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "shrtc" } });
    const options = screen.getAllByRole("option");
    expect(options[0]).toHaveTextContent("Keyboard shortcuts");
    fireEvent.change(field, { target: { value: "zzzz" } });
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("Nothing matches.")).toBeInTheDocument();
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.keyDown(field, { key: "ArrowDown" });
    expect(screen.getAllByRole("option")[1]).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("Go to review… switches lists and opens it", () => {
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "go to review" } });
    fireEvent.keyDown(field, { key: "Enter" });
    // A mode: the palette stays, only reviews are listed.
    expect(screen.getByText("Review")).toBeInTheDocument();
    fireEvent.change(field, { target: { value: "docs" } });
    expect(screen.getAllByRole("option")).toHaveLength(1);
    fireEvent.keyDown(field, { key: "Enter" });
    expect(useReviewStore.getState().selectedId).toBe(other.id);
    expect(useReviewStore.getState().tab).toBe("needsReReview");
  });

  it("Go to file… selects and focuses the file", () => {
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "go to file" } });
    fireEvent.keyDown(field, { key: "Enter" });
    fireEvent.change(field, { target: { value: "readme" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(useCodeViewStore.getState().byReview[r.id].selected).toBe(
      "docs/readme.md",
    );
    expect(useFocusStore.getState().item).toEqual({
      kind: "file",
      path: "docs/readme.md",
    });
    // Backspace on an empty field leaves a mode.
    const again = openPalette();
    fireEvent.change(again, { target: { value: "go to file" } });
    fireEvent.keyDown(again, { key: "Enter" });
    fireEvent.keyDown(again, { key: "Backspace" });
    expect(screen.queryByText("File")).toBeNull();
  });

  it("Approve… opens Submit review with Approve chosen; nothing posts", () => {
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "approve" } });
    fireEvent.keyDown(field, { key: "Enter" });
    const dialog = screen.getByRole("dialog", {
      name: `Submit review — acme-fixture/widgets#${r.number}`,
    });
    expect(dialog).toBeVisible();
    expect(
      within(dialog).getByRole("radio", { name: /Approve/ }),
    ).toBeChecked();
    noWrites();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(useOverlayStore.getState().confirm).toBeNull();
    noWrites();
  });

  it("Post all… asks first; only its button posts", async () => {
    useCommentStore.setState({
      drafts: {
        [r.id]: [
          {
            id: "draft-1",
            path: "src/cache.rs",
            side: "new",
            line: 2,
            startLine: null,
            body: "Fixture remark",
            headSha: r.headSha,
            baseSha: r.baseSha,
            createdAt: "2026-10-16T11:00:00Z",
            error: null,
          },
        ],
      },
    });
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "post_comments" ? [{ status: "posted" }] : []),
    );
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "post all" } });
    fireEvent.keyDown(field, { key: "Enter" });
    const dialog = screen.getByRole("dialog", {
      name: "Post 1 pending comment",
    });
    noWrites();
    fireEvent.click(within(dialog).getByRole("button", { name: "Post all" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "post_comments",
        expect.objectContaining({ reviewId: r.id }),
      ),
    );
  });

  it("leaves out the composer keys and needs no review for the list ones", () => {
    useReviewStore.setState({ selectedId: null });
    render(<Shell />);
    fireEvent.change(openPalette(), { target: { value: "comment" } });
    const labels = screen.queryAllByRole("option").map((o) => o.textContent);
    expect(labels.join(" ")).not.toContain("Submit comment");
    expect(labels.join(" ")).not.toContain("Comment on line");
  });
});

describe("CommandPalette: resolving the focused thread", () => {
  const focusOn = (threadId: string) =>
    act(() =>
      useFocusStore.getState().focusItem(r.id, {
        kind: "thread",
        threadId,
        mine: false,
        card: CONVERSATION,
        line: 0,
        side: null,
      }),
    );

  it("offers Resolve thread and runs it once on Enter", async () => {
    const open = { ...t, resolvable: true };
    useThreadStore.setState({
      byReview: { [r.id]: { threads: [open], loading: false, error: null } },
    });
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "resolve_thread"
          ? { ...open, resolved: true }
          : cmd === "list_threads"
            ? [{ ...open, resolved: true }]
            : [],
      ),
    );
    render(<Shell />);
    focusOn(open.threadId);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "resolve thread" } });
    const first = screen.getAllByRole("option")[0];
    expect(first).toHaveTextContent("Resolve thread");
    expect(within(first).getByText("S")).toBeInTheDocument();
    // Listing it writes nothing.
    noWrites();
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() =>
      expect(useThreadStore.getState().byReview[r.id].threads[0].resolved).toBe(
        true,
      ),
    );
    expect(
      mockInvoke.mock.calls.filter(([cmd]) => cmd === "resolve_thread"),
    ).toEqual([
      [
        "resolve_thread",
        { reviewId: r.id, threadId: open.threadId, resolved: true },
      ],
    ]);
  });

  it("offers Unresolve thread on a resolved one, nothing on a conversation", () => {
    const done = { ...t, resolvable: true, resolved: true };
    useThreadStore.setState({
      byReview: { [r.id]: { threads: [done], loading: false, error: null } },
    });
    render(<Shell />);
    focusOn(done.threadId);
    fireEvent.change(openPalette(), { target: { value: "resolve thread" } });
    expect(screen.getAllByRole("option")[0]).toHaveTextContent(
      "Unresolve thread",
    );
    press({ key: "Escape" });
    act(() =>
      useThreadStore.setState({
        byReview: { [r.id]: { threads: [t], loading: false, error: null } },
      }),
    );
    act(() => useOverlayStore.setState({ open: null }));
    fireEvent.change(openPalette(), { target: { value: "resolve thread" } });
    expect(
      screen
        .getAllByRole("option")
        .map((o) => o.textContent)
        .some((text) => /^(Un)?resolve thread/i.test(text ?? "")),
    ).toBe(false);
    noWrites();
  });
});

describe("ShortcutHelp", () => {
  it("opens on ? and lists the shortcuts by where they work", () => {
    useSettingsStore.getState().setShortcut("nextItem", ["BracketRight"]);
    render(<Shell />);
    press({ key: "?", code: "Slash", shiftKey: true });
    const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    const review = within(dialog).getByRole("region", { name: "In a review" });
    expect(within(review).getByText("Next thread or finding")).toBeVisible();
    // The live keymap, rebinding included.
    expect(within(review).getByText("]")).toBeVisible();
    expect(within(review).getByText("Reply to thread")).toBeVisible();
    expect(within(review).getByText("Resolve or reopen thread")).toBeVisible();
    // No default key any more: the header's arrows run it.
    const jump = within(review)
      .getByText("Next unresolved thread")
      .closest("div")!;
    expect(within(jump).getByText("Not bound")).toBeVisible();
    expect(within(review).queryByText("↓")).toBeNull();
    const anywhere = within(dialog).getByRole("region", { name: "Anywhere" });
    expect(within(anywhere).getByText("Refresh")).toBeVisible();
    expect(within(anywhere).getByText("Shift+R")).toBeVisible();
    expect(within(anywhere).getByText("?")).toBeVisible();
    expect(
      within(dialog).getByRole("region", { name: "In a comment" }),
    ).toBeVisible();
  });

  it("goes to Settings → Shortcuts", () => {
    render(<Shell />);
    press({ key: "?", code: "Slash", shiftKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Change shortcuts…" }));
    expect(useOverlayStore.getState().open).toBeNull();
    expect(useSettingsRequestStore.getState().request).toBe("shortcuts");
  });
});

describe("Open in editor in the palette and the help", () => {
  it("is listed with its key and opens the focused file", async () => {
    useLocalFileStore.setState({
      byReview: {
        [r.id]: {
          "src/cache.rs": {
            path: "src/cache.rs",
            absolutePath: "/Users/fixture/widgets/src/cache.rs",
            exists: true,
            cloneHead: null,
            matchesReviewHead: false,
          },
        },
      },
    });
    useFocusStore.setState({ item: { kind: "file", path: "src/cache.rs" } });
    render(<Shell />);
    const field = openPalette();
    fireEvent.change(field, { target: { value: "open in editor" } });
    const first = screen.getAllByRole("option")[0];
    expect(first).toHaveTextContent("Open in editor");
    expect(within(first).getByText("O")).toBeInTheDocument();
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("open_in_editor", {
        reviewId: r.id,
        path: "src/cache.rs",
        useDefault: false,
      }),
    );
    noWrites();

    press({ key: "?", code: "Slash", shiftKey: true });
    const help = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    const review = within(help).getByRole("region", { name: "In a review" });
    expect(within(review).getByText("Open in editor")).toBeVisible();
    expect(within(review).getByText("Show in Finder / Explorer")).toBeVisible();
  });
});
