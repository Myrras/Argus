import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, review, settings } from "../test/fixtures";
import { aiRun, checklistItem, finding, verdict } from "../test/aiFixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  warn: vi.fn(() => Promise.resolve()),
  error: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { ChecklistItem, Finding, FindingPostResult } from "../ipc/types";
import { subscribeAiEvents, useAiStore } from "./aiStore";
import { useBackendSettingsStore } from "./backendSettingsStore";
import { useReviewStore } from "./reviewStore";
import { useToastStore } from "./toastStore";

const mockInvoke = vi.mocked(invoke);
const initial = useAiStore.getState();
const initialReviews = useReviewStore.getState();
const initialSettings = useBackendSettingsStore.getState();

const r = review();
const ID = r.id;
const uuidThread = "00000000-0000-4000-a000-000000009001";

/** Every command that changes something (the platform or Argus' store). */
const WRITES = [
  "post_findings",
  "run_ai",
  "cancel_ai",
  "dismiss_finding",
  "set_checklist_checked",
  "set_verdict_override",
  "show_finding_anyway",
  "show_checklist_item_anyway",
];

const writes = () =>
  mockInvoke.mock.calls.filter(([cmd]) => WRITES.includes(cmd));

let findings = [
  finding({ reviewId: ID, severity: "low", title: "Low one" }),
  finding({ reviewId: ID, severity: "high", title: "High one" }),
];
let postResults: FindingPostResult[] = [];
let skipped: Finding[] = [];
let skippedItems: ChecklistItem[] = [];

function backend(cmd: string, args?: unknown) {
  const a = (args ?? {}) as Record<string, unknown>;
  switch (cmd) {
    case "ai_state":
      return Promise.resolve(aiRun({ reviewId: String(a.reviewId) }));
    case "list_findings":
      return Promise.resolve(findings);
    case "list_skipped_findings":
      return Promise.resolve(skipped);
    case "show_finding_anyway":
      return Promise.resolve(skipped[0]);
    case "list_skipped_checklist":
      return Promise.resolve(skippedItems);
    case "show_checklist_item_anyway":
      return Promise.resolve(skippedItems[0]);
    case "list_checklist":
      return Promise.resolve([checklistItem({ reviewId: ID })]);
    case "list_verdicts":
      return Promise.resolve([verdict({ reviewId: ID })]);
    case "post_findings":
      return Promise.resolve(postResults);
    case "dismiss_finding":
      return Promise.resolve({ ...findings[0], dismissed: a.dismissed });
    default:
      return Promise.resolve(null);
  }
}

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(backend as typeof invoke);
  useAiStore.setState({ ...initial, runs: {}, byReview: {}, ui: {} }, true);
  useReviewStore.setState({ ...initialReviews, reviews: [] }, true);
  useBackendSettingsStore.setState({ ...initialSettings }, true);
  useToastStore.setState({ toasts: [] });
  findings = [
    finding({ reviewId: ID, severity: "low", title: "Low one" }),
    finding({ reviewId: ID, severity: "high", title: "High one" }),
  ];
  skipped = [
    finding({
      reviewId: ID,
      title: "Already said",
      repeatOf: { reason: "similar", threadId: uuidThread, findingId: null },
    }),
  ];
  skippedItems = [
    checklistItem({
      reviewId: ID,
      text: "Said already?",
      repeatOf: { reason: "similar", threadId: uuidThread, findingId: null },
    }),
  ];
});

describe("aiStore", () => {
  it("loads the run and the output when a review opens", async () => {
    await useAiStore.getState().load(ID);
    expect(mockInvoke).toHaveBeenCalledWith("ai_state", { reviewId: ID });
    expect(mockInvoke).toHaveBeenCalledWith("list_findings", { reviewId: ID });
    expect(mockInvoke).toHaveBeenCalledWith("list_checklist", {
      reviewId: ID,
    });
    expect(mockInvoke).toHaveBeenCalledWith("list_verdicts", { reviewId: ID });
    const s = useAiStore.getState();
    expect(s.runs[ID].status).toBe("done");
    expect(s.byReview[ID].findings).toHaveLength(2);
    expect(s.byReview[ID].checklist).toHaveLength(1);
    expect(s.byReview[ID].verdicts).toHaveLength(1);
  });

  it("re-reads the output when ai:state reports a finished run", async () => {
    const off = await subscribeAiEvents();
    await useAiStore.getState().load(ID);
    events.emit("ai:state", aiRun({ reviewId: ID, status: "running" }));
    expect(useAiStore.getState().runs[ID].status).toBe("running");
    mockInvoke.mockClear();
    // Running → no re-read yet.
    events.emit("ai:state", aiRun({ reviewId: ID, status: "running" }));
    expect(mockInvoke).not.toHaveBeenCalledWith("list_findings", {
      reviewId: ID,
    });
    findings = [finding({ reviewId: ID, title: "New one" })];
    events.emit(
      "ai:state",
      aiRun({ reviewId: ID, finishedAt: "2026-10-16T11:00:00Z" }),
    );
    await vi.waitFor(() =>
      expect(useAiStore.getState().byReview[ID].findings[0].title).toBe(
        "New one",
      ),
    );
    off();
  });

  it("re-reads a loaded review on review:changed, and only that one", async () => {
    const off = await subscribeAiEvents();
    await useAiStore.getState().load(ID);
    mockInvoke.mockClear();
    events.emit("review:changed", { reviewId: "someone-else" });
    expect(mockInvoke).not.toHaveBeenCalled();
    events.emit("review:changed", { reviewId: ID });
    await vi.waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("list_findings", {
        reviewId: ID,
      }),
    );
    off();
  });

  it("reads the run of every listed review, once", async () => {
    const off = await subscribeAiEvents();
    const other = review();
    useReviewStore.setState({ reviews: [r, other] });
    useReviewStore.setState({ reviews: [r, other] });
    await vi.waitFor(() =>
      expect(Object.keys(useAiStore.getState().runs)).toHaveLength(2),
    );
    expect(
      mockInvoke.mock.calls.filter(([cmd]) => cmd === "ai_state"),
    ).toHaveLength(2);
    off();
  });

  it("re-reads everything when the AI pass is switched in Settings", async () => {
    const off = await subscribeAiEvents();
    useBackendSettingsStore.setState({ settings: settings() });
    await useAiStore.getState().load(ID);
    mockInvoke.mockClear();
    useBackendSettingsStore.setState({
      settings: settings({
        ai: { enabled: false, adapter: "claude", args: [], binary: null },
      }),
    });
    expect(mockInvoke).toHaveBeenCalledWith("ai_state", { reviewId: ID });
    expect(mockInvoke).toHaveBeenCalledWith("list_findings", { reviewId: ID });
    off();
  });

  it("re-reads the runs when the agent is switched in Settings", async () => {
    const off = await subscribeAiEvents();
    useBackendSettingsStore.setState({ settings: settings() });
    await useAiStore.getState().load(ID);
    mockInvoke.mockClear();
    useBackendSettingsStore.setState({
      settings: settings({
        ai: { enabled: true, adapter: "auggie", args: [], binary: null },
      }),
    });
    expect(mockInvoke).toHaveBeenCalledWith("ai_state", { reviewId: ID });
    off();
  });

  it("writes nothing on its own: loads and events are reads only", async () => {
    const off = await subscribeAiEvents();
    useReviewStore.setState({ reviews: [r] });
    await useAiStore.getState().load(ID);
    useAiStore.getState().setSelected(findings[0].id, true);
    events.emit("ai:state", aiRun({ reviewId: ID, finishedAt: "later" }));
    events.emit("review:changed", { reviewId: ID });
    await new Promise((res) => setTimeout(res, 0));
    expect(writes()).toEqual([]);
    off();
  });

  it("posts the selected findings with their edits; a failure keeps its selection and reason", async () => {
    await useAiStore.getState().load(ID);
    const [low, high] = findings;
    const s = useAiStore.getState();
    s.setSelected(low.id, true);
    s.setSelected(high.id, true);
    s.openEditor(low);
    s.setEditorText(low.id, "My own words");
    s.saveEditor(low);
    postResults = [
      {
        findingId: high.id,
        status: "posted",
        postedUrl: "https://github.com/acme-fixture/widgets/pull/1#c1",
        postedAt: "2026-10-16T12:00:00Z",
      },
      {
        findingId: low.id,
        status: "failed",
        kind: "lineNotInDiff",
        message: "The line is not in the diff any more.",
      },
    ];
    await useAiStore.getState().postSelected(ID);
    // High first (the list's order); the edit goes with the low one only.
    expect(mockInvoke).toHaveBeenCalledWith("post_findings", {
      reviewId: ID,
      posts: [
        { findingId: high.id, body: null },
        { findingId: low.id, body: "My own words" },
      ],
    });
    const ui = useAiStore.getState().ui;
    expect(ui[high.id]?.selected).toBe(false);
    expect(ui[low.id]).toMatchObject({
      selected: true,
      edit: "My own words",
      error: { kind: "lineNotInDiff" },
    });
    const toasts = useToastStore.getState().toasts.map((t) => t.message);
    expect(toasts).toContain("Posted 1 finding");
    expect(toasts).toContain("1 of 2 findings not posted");
    // Re-read after the post.
    expect(
      mockInvoke.mock.calls.filter(([cmd]) => cmd === "list_findings"),
    ).toHaveLength(2);
  });

  it("keeps everything when nothing could be tried", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    useAiStore.getState().setSelected(low.id, true);
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "post_findings"
        ? Promise.reject({ kind: "githubUnavailable", message: "gh is gone" })
        : backend(cmd, args)) as typeof invoke);
    await useAiStore.getState().postSelected(ID);
    expect(useAiStore.getState().ui[low.id]?.selected).toBe(true);
    expect(useAiStore.getState().posting[ID]).toBeUndefined();
    expect(useToastStore.getState().toasts[0].message).toBe(
      "Could not post the findings",
    );
  });

  it("posts nothing when nothing is selected", async () => {
    await useAiStore.getState().load(ID);
    await useAiStore.getState().postSelected(ID);
    expect(writes()).toEqual([]);
  });

  it("reads the findings set aside as already raised apart", async () => {
    await useAiStore.getState().load(ID);
    expect(mockInvoke).toHaveBeenCalledWith("list_skipped_findings", {
      reviewId: ID,
    });
    const out = useAiStore.getState().byReview[ID];
    expect(out.findings.map((f) => f.title)).toEqual(["Low one", "High one"]);
    expect(out.skipped?.map((f) => f.title)).toEqual(["Already said"]);
    expect(mockInvoke).toHaveBeenCalledWith("list_skipped_checklist", {
      reviewId: ID,
    });
    expect(out.skippedChecklist?.map((i) => i.text)).toEqual(["Said already?"]);
  });

  it("shows a set-aside checklist item anyway (local) and re-reads", async () => {
    await useAiStore.getState().load(ID);
    mockInvoke.mockClear();
    await useAiStore.getState().showItemAnyway(skippedItems[0]);
    expect(writes()).toEqual([
      ["show_checklist_item_anyway", { checklistId: skippedItems[0].id }],
    ]);
    expect(mockInvoke).toHaveBeenCalledWith("list_checklist", { reviewId: ID });
    expect(mockInvoke).toHaveBeenCalledWith("list_skipped_checklist", {
      reviewId: ID,
    });
  });

  it("shows a set-aside finding anyway (local) and re-reads; never posts it", async () => {
    await useAiStore.getState().load(ID);
    mockInvoke.mockClear();
    await useAiStore.getState().showAnyway(skipped[0]);
    expect(writes()).toEqual([
      ["show_finding_anyway", { findingId: skipped[0].id }],
    ]);
    expect(mockInvoke).toHaveBeenCalledWith("list_findings", { reviewId: ID });
    expect(mockInvoke).toHaveBeenCalledWith("list_skipped_findings", {
      reviewId: ID,
    });
    // Selecting it is impossible while set aside: Post selected sends nothing.
    useAiStore.getState().setSelected(skipped[0].id, true);
    await useAiStore.getState().postSelected(ID);
    expect(
      mockInvoke.mock.calls.filter(([cmd]) => cmd === "post_findings"),
    ).toEqual([]);
  });

  it("dismisses (deselecting) and re-reads", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    useAiStore.getState().setSelected(low.id, true);
    mockInvoke.mockClear();
    await useAiStore.getState().dismiss(low, true);
    expect(mockInvoke).toHaveBeenCalledWith("dismiss_finding", {
      findingId: low.id,
      dismissed: true,
    });
    expect(mockInvoke).toHaveBeenCalledWith("list_findings", { reviewId: ID });
    expect(useAiStore.getState().ui[low.id]?.selected).toBe(false);
  });

  it("toggles the focused finding's selection, not a posted one", async () => {
    findings = [
      finding({ reviewId: ID }),
      finding({ reviewId: ID, postedAt: "2026-10-16T12:00:00Z" }),
    ];
    await useAiStore.getState().load(ID);
    const [open, posted] = findings;
    const s = useAiStore.getState();
    expect(s.toggleFocused(ID)).toBe(false);
    s.focus(open.id);
    expect(useAiStore.getState().toggleFocused(ID)).toBe(true);
    expect(useAiStore.getState().ui[open.id]?.selected).toBe(true);
    useAiStore.getState().focus(posted.id);
    expect(useAiStore.getState().toggleFocused(ID)).toBe(false);
  });

  it("saving the unchanged default is no edit", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    const s = useAiStore.getState();
    s.openEditor(low);
    s.saveEditor(low);
    expect(useAiStore.getState().ui[low.id]?.edit).toBeNull();
  });
  it("Post selected posts what an open editor shows", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    const s = useAiStore.getState();
    s.setSelected(low.id, true);
    s.openEditor(low);
    s.setEditorText(low.id, "My careful rewrite");
    postResults = [
      {
        findingId: low.id,
        status: "posted",
        postedUrl: "https://github.com/acme-fixture/widgets/pull/1#c2",
        postedAt: "2026-10-16T12:00:00Z",
      },
    ];
    await useAiStore.getState().postSelected(ID);
    expect(mockInvoke).toHaveBeenCalledWith("post_findings", {
      reviewId: ID,
      posts: [{ findingId: low.id, body: "My careful rewrite" }],
    });
    expect(useAiStore.getState().ui[low.id]?.editor).toBeNull();
  });

  it("Post selected leaves out an emptied editor and keeps one opened meanwhile", async () => {
    await useAiStore.getState().load(ID);
    const [low, high] = findings;
    const s = useAiStore.getState();
    s.setSelected(low.id, true);
    s.setSelected(high.id, true);
    s.openEditor(low);
    s.setEditorText(low.id, "  ");
    let finish: (v: FindingPostResult[]) => void = () => {};
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "post_findings"
        ? new Promise((r) => (finish = r as typeof finish))
        : backend(cmd, args)) as typeof invoke);
    const p = useAiStore.getState().postSelected(ID);
    expect(mockInvoke).toHaveBeenCalledWith("post_findings", {
      reviewId: ID,
      posts: [{ findingId: high.id, body: null }],
    });
    // Opened while the post runs: not what was posted, so it stays.
    useAiStore.getState().openEditor(high);
    useAiStore.getState().setEditorText(high.id, "Second thoughts");
    finish([
      {
        findingId: high.id,
        status: "posted",
        postedUrl: "https://github.com/acme-fixture/widgets/pull/1#c3",
        postedAt: "2026-10-16T12:00:00Z",
      },
    ]);
    await p;
    const ui = useAiStore.getState().ui;
    expect(ui[high.id]).toMatchObject({
      selected: false,
      editor: "Second thoughts",
    });
    expect(ui[low.id]).toMatchObject({ selected: true, editor: "  " });
  });
  it("Save edit waits while Post selected runs; the newer text stays", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    useAiStore.getState().setSelected(low.id, true);
    let finish: (v: FindingPostResult[]) => void = () => {};
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "post_findings"
        ? new Promise((r) => (finish = r as typeof finish))
        : backend(cmd, args)) as typeof invoke);
    const p = useAiStore.getState().postSelected(ID);
    useAiStore.getState().openEditor(low);
    useAiStore.getState().setEditorText(low.id, "Second thoughts");
    useAiStore.getState().saveEditor(low);
    // Refused while posting: the editor keeps the text.
    expect(useAiStore.getState().ui[low.id]).toMatchObject({
      edit: null,
      editor: "Second thoughts",
    });
    expect(
      useToastStore
        .getState()
        .toasts.some((t) => t.message.includes("post to finish")),
    ).toBe(true);
    finish([
      {
        findingId: low.id,
        status: "posted",
        postedUrl: "https://github.com/acme-fixture/widgets/pull/1#c4",
        postedAt: "2026-10-16T12:00:00Z",
      },
    ]);
    await p;
    expect(useAiStore.getState().ui[low.id]?.editor).toBe("Second thoughts");
  });

  it("a saved edit that changed during Post selected stays in its editor", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    useAiStore.getState().setSelected(low.id, true);
    let finish: (v: FindingPostResult[]) => void = () => {};
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "post_findings"
        ? new Promise((r) => (finish = r as typeof finish))
        : backend(cmd, args)) as typeof invoke);
    const p = useAiStore.getState().postSelected(ID);
    // Changed by a path other than Save edit (belt and braces).
    useAiStore.setState((s) => ({
      ui: { ...s.ui, [low.id]: { ...s.ui[low.id], edit: "Newer edit" } },
    }));
    finish([
      {
        findingId: low.id,
        status: "posted",
        postedUrl: "https://github.com/acme-fixture/widgets/pull/1#c5",
        postedAt: "2026-10-16T12:00:00Z",
      },
    ]);
    await p;
    expect(useAiStore.getState().ui[low.id]).toMatchObject({
      selected: false,
      editor: "Newer edit",
    });
    expect(
      useToastStore
        .getState()
        .toasts.some((t) => t.message.includes("stays in its editor")),
    ).toBe(true);
  });

  it("Run again: an ai:state that came in meanwhile wins over the answer", async () => {
    const off = await subscribeAiEvents();
    await useAiStore.getState().load(ID);
    let answer: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "run_ai"
        ? new Promise<unknown>((r) => (answer = r))
        : backend(cmd, args)) as typeof invoke);
    const started = useAiStore.getState().run(ID);
    // The run starts and fails fast; its events beat the command's answer.
    events.emit("ai:state", aiRun({ reviewId: ID, queued: true }));
    events.emit(
      "ai:state",
      aiRun({ reviewId: ID, status: "running", finishedAt: null }),
    );
    const failed = aiRun({
      reviewId: ID,
      status: "failed",
      error: "`claude` was not found",
      finishedAt: "2026-10-16T11:00:00Z",
    });
    events.emit("ai:state", failed);
    answer(aiRun({ reviewId: ID, status: "running", finishedAt: null }));
    await started;
    expect(useAiStore.getState().runs[ID]).toEqual(failed);
    off();
  });

  it("Run again: the answer shows the run when no event came first", async () => {
    await useAiStore.getState().load(ID);
    const running = aiRun({
      reviewId: ID,
      status: "running",
      finishedAt: null,
    });
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "run_ai"
        ? Promise.resolve(running)
        : backend(cmd, args)) as typeof invoke);
    await useAiStore.getState().run(ID);
    expect(useAiStore.getState().runs[ID]).toEqual(running);
  });

  it("Cancel: an ai:state that came in meanwhile wins over the answer", async () => {
    const off = await subscribeAiEvents();
    await useAiStore.getState().load(ID);
    let answer: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "cancel_ai"
        ? new Promise<unknown>((r) => (answer = r))
        : backend(cmd, args)) as typeof invoke);
    const cancelled = aiRun({
      reviewId: ID,
      status: "failed",
      error: "cancelled",
      finishedAt: "2026-10-16T11:00:00Z",
    });
    const stopping = useAiStore.getState().cancel(ID);
    events.emit("ai:state", cancelled);
    // Read before the run had stopped.
    answer(aiRun({ reviewId: ID, status: "running", finishedAt: null }));
    await stopping;
    expect(useAiStore.getState().runs[ID]).toEqual(cancelled);
    off();
  });

  it("keeps the newest output when an older read answers last", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(((cmd: string, args?: unknown) =>
      cmd === "list_findings"
        ? new Promise<unknown>((r) => answers.push(r))
        : backend(cmd, args)) as typeof invoke);
    const older = useAiStore.getState().reloadOutput(ID);
    const newer = useAiStore.getState().reloadOutput(ID);
    const [fresh, stale] = findings;
    answers[1]([fresh]);
    await newer;
    answers[0]([stale]);
    await older;
    expect(useAiStore.getState().byReview[ID]?.findings).toEqual([fresh]);
  });
  it("closing an editor with changed text keeps it one Undo away", async () => {
    await useAiStore.getState().load(ID);
    const [low] = findings;
    const s = useAiStore.getState();
    s.openEditor(low);
    s.setEditorText(low.id, "Half rewritten");
    useAiStore.getState().closeEditor(low.id);
    expect(useAiStore.getState().ui[low.id]?.editor).toBeNull();
    const toast = useToastStore
      .getState()
      .toasts.find((t) => t.message === "Edit discarded");
    void toast?.action?.onClick();
    expect(useAiStore.getState().ui[low.id]?.editor).toBe("Half rewritten");
  });
});
