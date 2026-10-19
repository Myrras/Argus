import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { ChecklistItem, Finding, FindingPostResult } from "../ipc/types";
import { useAiStore } from "../state/aiStore";
import { useCodeViewStore } from "../state/codeViewStore";
import { useLayoutStore } from "../state/layoutStore";
import { useReviewStore } from "../state/reviewStore";
import { aiRun, checklistItem, finding } from "../test/aiFixtures";
import { review } from "../test/fixtures";
import { AiPanel } from "./AiPanel";

const mockInvoke = vi.mocked(invoke);
const initial = useAiStore.getState();
const initialReviews = useReviewStore.getState();
const r = review({ round: 2 });
const ID = r.id;

const URL = "https://github.com/acme-fixture/widgets/pull/2#discussion_r1";

let findings: Finding[];
let checklist: ChecklistItem[];
let postResults: FindingPostResult[];

function seed(enabled = true) {
  useAiStore.setState(
    {
      ...initial,
      runs: {
        [ID]: aiRun({ reviewId: ID, enabled, round: 2, lastDoneRound: 2 }),
      },
      byReview: {
        [ID]: {
          findings,
          checklist,
          verdicts: [],
          loading: false,
          error: null,
        },
      },
      ui: {},
      posting: {},
    },
    true,
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
  vi.mocked(openUrl).mockClear();
  findings = [
    finding({
      reviewId: ID,
      round: 2,
      severity: "low",
      path: "src/a.rs",
      line: 3,
      title: "Low on a",
    }),
    finding({
      reviewId: ID,
      round: 2,
      severity: "high",
      path: "src/b.rs",
      line: 9,
      title: "High on b9",
    }),
    finding({
      reviewId: ID,
      round: 1,
      severity: "high",
      path: "src/a.rs",
      line: 40,
      title: "Old high on a",
    }),
    finding({
      reviewId: ID,
      round: 2,
      severity: "medium",
      path: "src/a.rs",
      line: 1,
      title: "Medium on a",
    }),
    finding({
      reviewId: ID,
      round: 2,
      severity: "medium",
      path: "src/c.rs",
      line: 1,
      title: "Dismissed one",
      dismissed: true,
    }),
    finding({
      reviewId: ID,
      round: 1,
      severity: "low",
      path: "src/c.rs",
      line: 7,
      title: "Posted one",
      postedAt: "2026-10-16T09:00:00Z",
      postedUrl: URL,
    }),
  ];
  checklist = [
    checklistItem({
      reviewId: ID,
      round: 2,
      text: "Is the cache meant to be per user?",
    }),
    checklistItem({
      reviewId: ID,
      round: 2,
      text: "Check the migration on a copy",
      checked: true,
    }),
    checklistItem({ reviewId: ID, round: 1, text: "An earlier question" }),
  ];
  postResults = [];
  mockInvoke.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "list_findings":
        return Promise.resolve(findings);
      case "list_checklist":
        return Promise.resolve(checklist);
      case "list_verdicts":
        return Promise.resolve([]);
      case "post_findings":
        return Promise.resolve(postResults);
      default:
        return Promise.resolve(null);
    }
  }) as typeof invoke);
  useReviewStore.setState(
    { ...initialReviews, reviews: [r], selectedId: ID },
    true,
  );
  useLayoutStore.setState({ aiPanelOpen: true });
  seed();
});

const panel = () => screen.getByRole("complementary", { name: "AI review" });
const rows = () =>
  within(panel())
    .getAllByRole("listitem")
    .filter((li) => li.getAttribute("aria-label")?.startsWith("Finding: "))
    .map((li) => li.getAttribute("aria-label")!.slice("Finding: ".length));

describe("AiPanel", () => {
  it("groups the findings by severity, then path and line; hides dismissed", () => {
    render(<AiPanel />);
    expect(rows()).toEqual([
      "Old high on a",
      "High on b9",
      "Medium on a",
      "Low on a",
      "Posted one",
    ]);
    const old = screen.getByRole("listitem", {
      name: "Finding: Old high on a",
    });
    expect(within(old).getByText("from round 1")).toBeInTheDocument();
    // A posted earlier one is not "from round 1": it shows its link.
    const posted = screen.getByRole("listitem", {
      name: "Finding: Posted one",
    });
    expect(within(posted).queryByText("from round 1")).toBeNull();
    expect(within(posted).queryByRole("checkbox")).toBeNull();
    fireEvent.click(within(posted).getByRole("button", { name: "Posted" }));
    expect(openUrl).toHaveBeenCalledWith(URL);

    fireEvent.click(screen.getByRole("button", { name: "Show dismissed (1)" }));
    expect(rows()).toContain("Dismissed one");
    const dismissed = screen.getByRole("listitem", {
      name: "Finding: Dismissed one",
    });
    fireEvent.click(
      within(dismissed).getByRole("button", { name: "Undismiss" }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("dismiss_finding", {
      findingId: findings[4].id,
      dismissed: false,
    });
  });

  it("posts only on Post selected, with a per-finding result", async () => {
    render(<AiPanel />);
    const post = screen.getByRole("button", { name: "Post selected (0)" });
    expect(post).toBeDisabled();
    const [low, highB, oldHigh] = findings;
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: `Select "${highB.title}" for posting`,
      }),
    );
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: `Select "${low.title}" for posting`,
      }),
    );
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: `Select "${oldHigh.title}" for posting`,
      }),
    );
    // An edit made in the code view goes with it.
    useAiStore.setState((s) => ({
      ui: {
        ...s.ui,
        [low.id]: { ...s.ui[low.id]!, edit: "**Low**\n\nReworded." },
      },
    }));
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "post_findings",
      expect.anything(),
    );

    postResults = [
      {
        findingId: oldHigh.id,
        status: "posted",
        postedUrl: URL,
        postedAt: "2026-10-16T12:00:00Z",
      },
      {
        findingId: highB.id,
        status: "failed",
        kind: "postRejected",
        message: "GitHub refused: line outside the diff.",
      },
      {
        findingId: low.id,
        status: "posted",
        postedUrl: `${URL}2`,
        postedAt: "2026-10-16T12:00:01Z",
      },
    ];
    findings = findings.map((f) =>
      f.id === oldHigh.id || f.id === low.id
        ? {
            ...f,
            postedAt: "2026-10-16T12:00:00Z",
            postedUrl: f.id === low.id ? `${URL}2` : URL,
          }
        : f,
    );
    fireEvent.click(screen.getByRole("button", { name: "Post selected (3)" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("post_findings", {
        reviewId: ID,
        posts: [
          { findingId: oldHigh.id, body: null },
          { findingId: highB.id, body: null },
          { findingId: low.id, body: "**Low**\n\nReworded." },
        ],
      }),
    );
    // The failed one stays selected with its reason; the posted ones link.
    const failed = await screen.findByRole("listitem", {
      name: `Finding: ${highB.title}`,
    });
    expect(within(failed).getByRole("alert")).toHaveTextContent(
      "Not posted: GitHub refused: line outside the diff.",
    );
    expect(within(failed).getByRole("checkbox")).toBeChecked();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Post selected (1)" }),
      ).toBeEnabled(),
    );
    const done = screen.getByRole("listitem", {
      name: `Finding: ${low.title}`,
    });
    expect(
      within(done).getByRole("button", { name: "Posted" }),
    ).toHaveAttribute("title", `${URL}2`);
  });

  it("scrolls to a finding: its file is selected", () => {
    render(<AiPanel />);
    fireEvent.click(screen.getByRole("button", { name: /Medium on a/ }));
    expect(useCodeViewStore.getState().byReview[ID]?.selected).toBe("src/a.rs");
    expect(useAiStore.getState().focused).toBe(findings[3].id);
    expect(useAiStore.getState().reveal?.path).toBe("src/a.rs");
  });

  it("ticks the checklist of the current round; earlier rounds stay folded", async () => {
    render(<AiPanel />);
    const list = screen.getByRole("region", { name: "Look here yourself" });
    expect(within(list).getByText("1 of 2 open")).toBeInTheDocument();
    expect(within(list).queryByText("An earlier question")).toBeNull();
    fireEvent.click(
      within(list).getByRole("checkbox", {
        name: "Is the cache meant to be per user?",
      }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("set_checklist_checked", {
      checklistId: checklist[0].id,
      checked: true,
    });
    fireEvent.click(
      within(list).getByRole("button", { name: "Earlier rounds (1)" }),
    );
    expect(within(list).getByText("An earlier question")).toBeInTheDocument();
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("list_checklist", {
        reviewId: ID,
      }),
    );
  });

  it("closes into the layout store", () => {
    render(<AiPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Close the AI panel" }));
    expect(useLayoutStore.getState().aiPanelOpen).toBe(false);
  });

  it("warns when the agent cannot be held to reading only (auggie)", () => {
    const warning =
      "Auggie can still use MCP tools from your own Auggie setup and read local secret files in the clone (.env files, keys); Argus cannot guarantee this run only reads.";
    const { unmount } = render(<AiPanel />);
    expect(screen.queryByText(warning)).toBeNull();
    unmount();
    useAiStore.setState({
      runs: {
        [ID]: aiRun({
          reviewId: ID,
          adapter: "auggie",
          readOnlyGuaranteed: false,
        }),
      },
    });
    render(<AiPanel />);
    expect(screen.getByText(warning)).toBeInTheDocument();
  });

  it("counts the findings skipped as already raised in the review", () => {
    const { unmount } = render(<AiPanel />);
    expect(screen.queryByText(/already raised in the review/)).toBeNull();
    unmount();
    useAiStore.setState({
      runs: { [ID]: aiRun({ reviewId: ID, droppedDuplicates: 3 }) },
    });
    render(<AiPanel />);
    expect(
      screen.getByText("3 skipped: already raised in the review."),
    ).toBeInTheDocument();
  });

  it("is absent with the AI pass off", () => {
    seed(false);
    const { container } = render(<AiPanel />);
    expect(container).toBeEmptyDOMElement();
  });

  it("says why there is nothing yet", () => {
    findings = [];
    checklist = [];
    seed();
    useAiStore.setState({
      runs: {
        [ID]: aiRun({
          reviewId: ID,
          status: "failed",
          error: "agent timed out",
        }),
      },
    });
    render(<AiPanel />);
    expect(
      screen.getByText("The last AI pass failed: agent timed out."),
    ).toBeInTheDocument();
    expect(screen.getByText("No findings.")).toBeInTheDocument();
  });
});
