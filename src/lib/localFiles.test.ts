import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import type { LocalFile, Settings } from "../ipc/types";
import { CONVERSATION, useCodeViewStore } from "../state/codeViewStore";
import { useFocusStore } from "../state/focusStore";
import { useLocalFileStore } from "../state/localFileStore";
import { CONVERSATION_CARD } from "./focusModel";
import {
  focusedFilePath,
  headHint,
  localFileMenuItems,
  openTitle,
  opensRepository,
  runLocalFileAction,
} from "./localFiles";
import {
  DEFAULT_KEYMAP,
  actionDef,
  findConflict,
  parseBinding,
} from "./keymap";

const mockInvoke = vi.mocked(invoke);
const REVIEW = "00000000-0000-4000-8000-000000000001";
const HEAD = "a".repeat(40);

function local(path: string, over: Partial<LocalFile> = {}): LocalFile {
  return {
    path,
    absolutePath: `/Users/fixture/widgets/${path}`,
    exists: true,
    cloneHead: HEAD,
    matchesReviewHead: true,
    ...over,
  };
}

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(undefined);
  useFocusStore.setState({ item: null, reviewId: null });
  useCodeViewStore.setState({ byReview: {} });
  useLocalFileStore.setState({
    byReview: {
      [REVIEW]: {
        "src/cache.rs": local("src/cache.rs"),
        "src/old.rs": local("src/old.rs", { exists: false }),
      },
    },
  });
});

const opened = () =>
  mockInvoke.mock.calls.filter(([cmd]) => cmd === "open_in_editor");

describe("open in editor rules", () => {
  it("hints at a clone on another commit only when its head is known", () => {
    expect(headHint(local("a"))).toBeNull();
    expect(
      headHint(
        local("a", { cloneHead: "b".repeat(40), matchesReviewHead: false }),
      ),
    ).toBe("Your clone is on bbbbbbb, not this review's head");
    expect(
      headHint(local("a", { cloneHead: null, matchesReviewHead: false })),
    ).toBeNull();
    expect(openTitle(local("a", { exists: false }), "Open in editor (O)")).toBe(
      "Not in your working tree",
    );
  });

  it("says when the repository opens with the file", () => {
    const f = local("a");
    expect(openTitle(f, "Open in editor (O)", true)).toBe(
      "Open in editor (O)\nOpens the repository with the file",
    );
    expect(openTitle(f, "Open in editor (O)")).toBe("Open in editor (O)");
    const settings = (part: Partial<Settings>) => part as Settings;
    const base = settings({ editorOverrides: { md: "/A/Typora.app" } });
    expect(opensRepository(base, "docs/a.md")).toBe(true);
    expect(opensRepository(base, "src/a.rs")).toBe(false);
    expect(
      opensRepository({ ...base, defaultEditor: "/A/Zed.app" }, "src/a.rs"),
    ).toBe(true);
    expect(
      opensRepository(
        settings({ defaultEditor: "/A/Zed.app", openWithRepository: false }),
        "a.md",
      ),
    ).toBe(false);
  });

  it("offers the file menu only for a file of a clone", () => {
    expect(localFileMenuItems(REVIEW, "src/elsewhere.rs")).toEqual([]);
    const items = localFileMenuItems(REVIEW, "src/old.rs");
    expect(items.map((i) => i.label)).toEqual([
      "Open in Editor",
      "Open With Default App",
      expect.stringMatching(/^Show in /),
    ]);
    expect(items.every((i) => i.disabled)).toBe(true);
    expect(localFileMenuItems(REVIEW, "src/cache.rs")[0].disabled).toBe(false);
  });

  it("acts on the focused item's file, else the selected file", () => {
    useCodeViewStore.setState({
      byReview: {
        [REVIEW]: {
          since: false,
          files: null,
          loading: false,
          error: null,
          selected: "src/cache.rs",
          order: [],
        },
      },
    });
    expect(focusedFilePath(REVIEW)).toBe("src/cache.rs");
    // A thread on a file, a line, the Conversation.
    useFocusStore.setState({
      reviewId: REVIEW,
      item: {
        kind: "thread",
        threadId: "00000000-0000-4000-8000-0000000000aa",
        mine: false,
        card: "src/old.rs",
        line: 3,
        side: "new",
      },
    });
    expect(focusedFilePath(REVIEW)).toBe("src/old.rs");
    useFocusStore.setState({
      item: { kind: "line", path: "src/cache.rs", side: "new", line: 2 },
    });
    expect(focusedFilePath(REVIEW)).toBe("src/cache.rs");
    useFocusStore.setState({
      item: {
        kind: "thread",
        threadId: "00000000-0000-4000-8000-0000000000ab",
        mine: false,
        card: CONVERSATION_CARD,
        line: 0,
        side: null,
      },
    });
    expect(focusedFilePath(REVIEW)).toBeNull();
    useFocusStore.setState({ item: null });
    useCodeViewStore.setState({
      byReview: {
        [REVIEW]: {
          ...useCodeViewStore.getState().byReview[REVIEW],
          selected: CONVERSATION,
        },
      },
    });
    expect(focusedFilePath(REVIEW)).toBeNull();
  });

  it("runs only its own actions, and opens the file (no line)", () => {
    expect(runLocalFileAction("nextFile", REVIEW)).toBeNull();
    // Nothing focused or selected.
    expect(runLocalFileAction("openInEditor", REVIEW)).toBe(false);
    useFocusStore.setState({
      reviewId: REVIEW,
      item: { kind: "line", path: "src/cache.rs", side: "new", line: 7 },
    });
    expect(runLocalFileAction("openInEditor", REVIEW)).toBe(true);
    expect(opened()[0][1]).toEqual({
      reviewId: REVIEW,
      path: "src/cache.rs",
      useDefault: false,
    });
    expect(runLocalFileAction("showInFinder", REVIEW)).toBe(true);
    expect(mockInvoke).toHaveBeenCalledWith("reveal_local_file", {
      reviewId: REVIEW,
      path: "src/cache.rs",
    });
    // Without a clone: nothing.
    useLocalFileStore.setState({ byReview: { [REVIEW]: null } });
    expect(runLocalFileAction("openInEditor", REVIEW)).toBe(false);
  });

  it("binds O and Iris's ⌘⌥R, free of every other default", () => {
    expect(actionDef("openInEditor")).toMatchObject({
      scope: "files",
      group: "Files",
      defaults: ["KeyO"],
    });
    expect(actionDef("showInFinder").defaults).toEqual(["Mod+Alt+KeyR"]);
    for (const id of ["openInEditor", "showInFinder"] as const) {
      for (const b of DEFAULT_KEYMAP[id]) {
        expect(findConflict(DEFAULT_KEYMAP, id, parseBinding(b)!)).toBeNull();
      }
    }
  });
});
