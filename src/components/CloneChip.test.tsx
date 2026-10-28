import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { CloneStatus } from "../ipc/types";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useCloneStatusStore } from "../state/cloneStatusStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useToastStore } from "../state/toastStore";
import { review, settings } from "../test/fixtures";
import { CloneChip } from "./CloneChip";

const mockInvoke = vi.mocked(invoke);
const HEAD = "a".repeat(40);
const r = review({ headSha: HEAD });
const PATH = "/Users/fixture/vcs/acme/widgets";

let status: CloneStatus | Error;

const found = (over: Partial<CloneStatus> = {}): CloneStatus => ({
  configured: true,
  path: PATH,
  found: true,
  head: HEAD,
  matchesReviewHead: true,
  ...over,
});

const calls = (name: string) =>
  mockInvoke.mock.calls.filter(([cmd]) => cmd === name);

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(((cmd: string) => {
    if (cmd === "clone_status") {
      return status instanceof Error
        ? Promise.reject(status)
        : Promise.resolve(status);
    }
    return Promise.resolve(null);
  }) as typeof invoke);
  status = found();
  useCloneStatusStore.setState({ byReview: {} });
  useSettingsRequestStore.setState({ request: null });
  useToastStore.setState({ toasts: [] });
  useBackendSettingsStore.setState({
    settings: settings({ clonePaths: { [`${r.host}/${r.repo}`]: PATH } }),
  });
});

describe("CloneChip", () => {
  it("shows a found clone's short path and its details in the tooltip", async () => {
    render(<CloneChip review={r} />);
    const chip = await screen.findByRole("button", {
      name: `Local clone: ${PATH}`,
    });
    expect(chip).toHaveTextContent("~/…/widgets");
    expect(chip).toHaveAttribute("data-state", "found");
    expect(chip.title).toContain(PATH);
    expect(chip.title).toContain("HEAD aaaaaaa: at the review's head");
  });

  it("says when the clone is on another commit", async () => {
    status = found({ head: "c".repeat(40), matchesReviewHead: false });
    render(<CloneChip review={r} />);
    const chip = await screen.findByRole("button", { name: /Local clone:/ });
    expect(chip.title).toContain(
      "HEAD ccccccc: on another commit (Argus reads objects only)",
    );
  });

  it("shows the folder in the file manager on click, and nothing else", async () => {
    render(<CloneChip review={r} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /Local clone:/ }),
    );
    await waitFor(() => expect(calls("reveal_clone")).toHaveLength(1));
    expect(calls("reveal_clone")[0][1]).toEqual({ reviewId: r.id });
    expect(useSettingsRequestStore.getState().request).toBeNull();
  });

  it("toasts the core's text when the folder cannot be shown", async () => {
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === "clone_status"
        ? Promise.resolve(found())
        : cmd === "reveal_clone"
          ? Promise.reject({ kind: "invalidInput", message: "gone" })
          : Promise.resolve(null)) as typeof invoke);
    render(<CloneChip review={r} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /Local clone:/ }),
    );
    await waitFor(() =>
      expect(useToastStore.getState().toasts[0]?.detail).toContain("gone"),
    );
  });

  it("warns when the clone is set but not found, and opens Settings → AI", async () => {
    status = found({ found: false, head: null, matchesReviewHead: false });
    render(<CloneChip review={r} />);
    const chip = await screen.findByRole("button", {
      name: `Local clone not found: ${PATH}`,
    });
    expect(chip).toHaveAttribute("data-state", "missing");
    expect(chip.title).toContain(`Local clone not found: ${PATH}`);
    fireEvent.click(chip);
    expect(useSettingsRequestStore.getState().request).toBe("ai");
    expect(calls("reveal_clone")).toHaveLength(0);
  });

  it("is a quiet 'No local clone' that opens Settings → AI when none is set", async () => {
    status = {
      configured: false,
      path: null,
      found: false,
      head: null,
      matchesReviewHead: false,
    };
    useBackendSettingsStore.setState({ settings: settings() });
    render(<CloneChip review={r} />);
    const chip = await screen.findByRole("button", { name: "No local clone" });
    expect(chip).toHaveAttribute("data-state", "none");
    fireEvent.click(chip);
    expect(useSettingsRequestStore.getState().request).toBe("ai");
    expect(calls("reveal_clone")).toHaveLength(0);
  });

  it("is left out when the status cannot be read", async () => {
    status = new Error("boom");
    render(<CloneChip review={r} />);
    await waitFor(() => expect(calls("clone_status")).toHaveLength(1));
    await waitFor(() =>
      expect(useCloneStatusStore.getState().byReview[r.id]).toBeNull(),
    );
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("re-reads when the window gets the focus back and when the review's head moves", async () => {
    const { rerender } = render(<CloneChip review={r} />);
    await screen.findByRole("button", { name: /Local clone:/ });
    expect(calls("clone_status")).toHaveLength(1);
    window.dispatchEvent(new Event("focus"));
    await waitFor(() => expect(calls("clone_status")).toHaveLength(2));
    rerender(<CloneChip review={{ ...r, headSha: "d".repeat(40) }} />);
    await waitFor(() => expect(calls("clone_status")).toHaveLength(3));
  });

  it("writes nothing when only read", async () => {
    render(<CloneChip review={r} />);
    await screen.findByRole("button", { name: /Local clone:/ });
    expect(calls("reveal_clone")).toHaveLength(0);
    expect(calls("open_in_editor")).toHaveLength(0);
  });
});
