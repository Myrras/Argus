import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { Verdict } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { aiRun, verdict } from "../test/aiFixtures";
import { thread } from "../test/fixtures";
import { ThreadBlock } from "./ThreadBlock";

const mockInvoke = vi.mocked(invoke);
const initial = useAiStore.getState();
const t = thread({ mine: true, author: "rev-fixture" });
const NOW = Date.parse("2026-10-16T12:00:00Z");

let stored: Verdict;

function seed(v: Verdict | null, enabled = true) {
  useAiStore.setState(
    {
      ...initial,
      runs: { [t.reviewId]: aiRun({ reviewId: t.reviewId, enabled }) },
      byReview: {
        [t.reviewId]: {
          findings: [],
          checklist: [],
          verdicts: v ? [v] : [],
          loading: false,
          error: null,
        },
      },
    },
    true,
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
  stored = verdict({ threadId: t.threadId, reviewId: t.reviewId });
  mockInvoke.mockImplementation(((cmd: string, args?: unknown) => {
    const a = (args ?? {}) as { label?: Verdict["label"] };
    switch (cmd) {
      case "set_verdict_override":
        stored = {
          ...stored,
          userOverride: a.label ?? null,
          label: a.label ?? stored.aiLabel,
        };
        return Promise.resolve(stored);
      case "list_verdicts":
        return Promise.resolve([stored]);
      default:
        return Promise.resolve([]);
    }
  }) as typeof invoke);
  seed(stored);
});

describe("VerdictBadge", () => {
  it("shows the AI's label with its reason and the Argus-only note", () => {
    render(<ThreadBlock thread={t} provider="github" now={NOW} />);
    const badge = screen.getByRole("button", {
      name: "Verdict: AI: partly addressed",
    });
    expect(badge).toHaveAttribute("data-label", "partly");
    expect(badge.title).toContain(
      "The fixture guard was added, the test is missing.",
    );
    expect(badge.title).toContain(
      "A hint inside Argus only: it never changes the thread on GitHub.",
    );
  });

  it("sets and clears the user's override, shown as such", async () => {
    render(<ThreadBlock thread={t} provider="gitlab" now={NOW} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Verdict: AI: partly addressed" }),
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark addressed" }));
    expect(mockInvoke).toHaveBeenCalledWith("set_verdict_override", {
      threadId: t.threadId,
      label: "addressed",
    });
    const overridden = await screen.findByRole("button", {
      name: "Verdict: you: addressed · AI: partly",
    });
    expect(overridden).toHaveAttribute("data-override", "true");
    expect(overridden.title).toContain("Your call: Addressed.");

    fireEvent.click(overridden);
    expect(
      screen.getByRole("menuitem", { name: "Mark addressed" }),
    ).toBeDisabled();
    fireEvent.click(
      screen.getByRole("menuitem", { name: "Clear my override" }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("set_verdict_override", {
      threadId: t.threadId,
      label: null,
    });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Verdict: AI: partly addressed" }),
      ).toBeInTheDocument(),
    );
    // Local only: nothing went to the platform.
    expect(mockInvoke.mock.calls.map(([c]) => c)).not.toContain(
      "reply_to_thread",
    );
  });

  it("is absent without a verdict or with the AI pass off", () => {
    seed(null);
    const { rerender } = render(
      <ThreadBlock thread={t} provider="github" now={NOW} />,
    );
    expect(screen.queryByRole("button", { name: /Verdict/ })).toBeNull();
    seed(stored, false);
    rerender(<ThreadBlock thread={t} provider="github" now={NOW} />);
    expect(screen.queryByRole("button", { name: /Verdict/ })).toBeNull();
  });
});
