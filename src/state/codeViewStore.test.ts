import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, reviewFiles } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";

import { useCodeViewStore } from "./codeViewStore";

const mockInvoke = vi.mocked(invoke);
const initial = useCodeViewStore.getState();
const ID = "00000000-0000-4000-8000-0000000000aa";

describe("codeViewStore", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    useCodeViewStore.setState({ ...initial, byReview: {} }, true);
  });

  it("keeps the newest files when an older read answers last", async () => {
    const answers: ((v: unknown) => void)[] = [];
    mockInvoke.mockImplementation(() => new Promise((r) => answers.push(r)));
    const older = useCodeViewStore.getState().load(ID, false);
    const newer = useCodeViewStore.getState().load(ID, false);
    const fresh = reviewFiles({ reviewId: ID, headSha: "c".repeat(40) });
    answers[1](fresh);
    await newer;
    answers[0](reviewFiles({ reviewId: ID, headSha: "a".repeat(40) }));
    await older;
    expect(useCodeViewStore.getState().byReview[ID]).toMatchObject({
      files: fresh,
      loading: false,
      error: null,
    });
  });

  it("drops an older read's failure too", async () => {
    const answers: { ok: (v: unknown) => void; fail: (e: unknown) => void }[] =
      [];
    mockInvoke.mockImplementation(
      () => new Promise((ok, fail) => answers.push({ ok, fail })),
    );
    const older = useCodeViewStore.getState().load(ID, false);
    const newer = useCodeViewStore.getState().load(ID, false);
    answers[1].ok(reviewFiles({ reviewId: ID }));
    await newer;
    answers[0].fail({ kind: "provider", message: "old failure" });
    await older;
    expect(useCodeViewStore.getState().byReview[ID]?.error).toBeNull();
  });
});
