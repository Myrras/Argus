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

import { invoke } from "@tauri-apps/api/core";

import type { AiRunState, Finding } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { useLayoutStore } from "../state/layoutStore";
import { aiRun, finding } from "../test/aiFixtures";
import { review } from "../test/fixtures";
import { AiRunIndicator, AiStatus } from "./AiStatus";

const mockInvoke = vi.mocked(invoke);
const initial = useAiStore.getState();
const r = review();

let run: AiRunState;
let skipped: Finding[];

beforeEach(() => {
  skipped = [];
  mockInvoke.mockReset();
  useAiStore.setState({ ...initial, runs: {}, byReview: {}, ui: {} }, true);
  useLayoutStore.setState({ aiPanelOpen: true });
  run = aiRun({ reviewId: r.id });
  mockInvoke.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "ai_state":
      case "run_ai":
      case "cancel_ai":
        return Promise.resolve(run);
      case "list_skipped_findings":
        return Promise.resolve(skipped);
      case "list_findings":
        return Promise.resolve([
          finding({ reviewId: r.id }),
          finding({ reviewId: r.id, dismissed: true }),
        ]);
      default:
        return Promise.resolve([]);
    }
  }) as typeof invoke);
});

const chip = () => screen.getByRole("status");

describe("AiStatus", () => {
  it("says done with the number of findings shown", async () => {
    render(<AiStatus review={r} />);
    await waitFor(() =>
      expect(chip()).toHaveTextContent("AI: done · 1 finding"),
    );
    expect(chip()).toHaveAttribute("data-status", "done");
    // The eye gets the short words; the count is not repeated on the toggle.
    expect(within(chip()).getByText("1 finding")).toHaveAttribute(
      "aria-hidden",
    );
    expect(
      screen.getByRole("button", { name: "Findings" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Run again" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("run_ai", { reviewId: r.id }),
    );
  });

  it("names the skipped repeats in the chip's tooltip, counted from the list", async () => {
    // The run's own count is stale (a rerun, a Show anyway): the list wins.
    run = aiRun({ reviewId: r.id, droppedDuplicates: 5 });
    skipped = [finding({ reviewId: r.id }), finding({ reviewId: r.id })];
    render(<AiStatus review={r} />);
    await waitFor(() =>
      expect(chip()).toHaveAttribute(
        "title",
        "AI: done · 1 finding — claude, round 1 — 2 skipped: already raised in the review",
      ),
    );
  });

  it("says in the chip's tooltip which rules, skill and worktree the last run used", async () => {
    run = aiRun({
      reviewId: r.id,
      context: {
        rulesFiles: ["CLAUDE.md", "src/api/CLAUDE.md"],
        skill: { name: "code-review", source: "project" },
        worktree: true,
        notes: [
          "The review skill strict-review (user) was not found; Auto used the project's own (code-review)",
        ],
      },
    });
    render(<AiStatus review={r} />);
    await waitFor(() =>
      expect(chip()).toHaveAttribute(
        "title",
        [
          "AI: done · 1 finding — claude, round 1",
          "Rules: CLAUDE.md, src/api/CLAUDE.md",
          "Review skill: code-review (project)",
          "Read a worktree at the review's head",
          "The review skill strict-review (user) was not found; Auto used the project's own (code-review)",
        ].join("\n"),
      ),
    );
    // A run without a worktree, rules or skill says so, with the reason.
    run = aiRun({
      reviewId: r.id,
      context: {
        rulesFiles: [],
        skill: null,
        worktree: false,
        notes: [
          "Couldn't create a review worktree: git worktree add failed; reviewed without it",
        ],
      },
    });
    useAiStore.setState({ runs: { [r.id]: run } });
    await waitFor(() =>
      expect(chip().getAttribute("title")).toContain(
        "Rules: none found\nReview skill: none\nCouldn't create a review worktree: git worktree add failed; reviewed without it",
      ),
    );
  });

  it("shows a running run with a spinner and Cancel", async () => {
    run = aiRun({ reviewId: r.id, status: "running" });
    const { container } = render(<AiStatus review={r} />);
    await waitFor(() => expect(chip()).toHaveTextContent("AI: running…"));
    expect(container.querySelector(".spinner")).not.toBeNull();
    // The earlier run's findings: counted on the toggle while this one runs.
    expect(
      await screen.findByRole("button", { name: "Findings, 1" }),
    ).toHaveTextContent("1");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cancel_ai", { reviewId: r.id }),
    );
  });

  it("one click on Run again starts the run and the chip shows it at once", async () => {
    render(<AiStatus review={r} />);
    await waitFor(() => expect(chip()).toHaveAttribute("data-status", "done"));
    // The core answers with the run it started.
    run = aiRun({ reviewId: r.id, status: "running", finishedAt: null });
    fireEvent.click(screen.getByRole("button", { name: "Run again" }));
    await waitFor(() => expect(chip()).toHaveTextContent("AI: running…"));
    expect(
      mockInvoke.mock.calls.filter(([cmd]) => cmd === "run_ai"),
    ).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("shows a queued run", async () => {
    run = aiRun({ reviewId: r.id, status: "done", queued: true });
    render(<AiStatus review={r} />);
    await waitFor(() => expect(chip()).toHaveTextContent("AI: queued"));
    expect(chip()).toHaveAttribute("data-status", "queued");
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("shows a failure with its reason and Retry", async () => {
    run = aiRun({
      reviewId: r.id,
      status: "failed",
      error: "`claude` was not found: install it or set its path in Settings",
    });
    render(<AiStatus review={r} />);
    await waitFor(() =>
      expect(chip()).toHaveTextContent(
        "AI: failed — `claude` was not found: install it or set its path in Settings",
      ),
    );
    expect(mockInvoke).not.toHaveBeenCalledWith("run_ai", expect.anything());
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("run_ai", { reviewId: r.id }),
    );
  });

  it("follows ai:state through the store", async () => {
    render(<AiStatus review={r} />);
    await waitFor(() => expect(chip()).toHaveTextContent("AI: done"));
    useAiStore
      .getState()
      .applyState(aiRun({ reviewId: r.id, status: "running" }));
    await waitFor(() => expect(chip()).toHaveTextContent("AI: running…"));
  });

  it("toggles the AI panel", async () => {
    render(<AiStatus review={r} />);
    const toggle = await screen.findByRole("button", { name: /Findings/ });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(toggle);
    expect(useLayoutStore.getState().aiPanelOpen).toBe(false);
  });

  it("renders nothing with the AI pass off", async () => {
    run = aiRun({ reviewId: r.id, enabled: false, status: "notStarted" });
    const { container } = render(<AiStatus review={r} />);
    await waitFor(() =>
      expect(useAiStore.getState().runs[r.id]?.enabled).toBe(false),
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe("AiRunIndicator", () => {
  it("shows only while a run is queued or running", () => {
    const { rerender, container } = render(<AiRunIndicator reviewId={r.id} />);
    expect(container).toBeEmptyDOMElement();
    useAiStore.setState({
      runs: { [r.id]: aiRun({ reviewId: r.id, status: "running" }) },
    });
    rerender(<AiRunIndicator reviewId={r.id} />);
    expect(screen.getByRole("img", { name: "AI pass running" })).toBeVisible();
    useAiStore.setState({
      runs: { [r.id]: aiRun({ reviewId: r.id, queued: true }) },
    });
    rerender(<AiRunIndicator reviewId={r.id} />);
    expect(screen.getByRole("img", { name: "AI pass queued" })).toBeVisible();
    useAiStore.setState({ runs: { [r.id]: aiRun({ reviewId: r.id }) } });
    rerender(<AiRunIndicator reviewId={r.id} />);
    expect(container).toBeEmptyDOMElement();
  });
});
