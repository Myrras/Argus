import { beforeEach, describe, expect, it, vi } from "vitest";

import { fakeEvents, file, reviewFiles } from "../test/fixtures";

const events = fakeEvents();
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) =>
    events.listen(name, cb),
  ),
}));

import { invoke } from "@tauri-apps/api/core";
import { answerBatches } from "../test/batches";

import {
  clearContentsCache,
  loadFileSides,
  prefetchFile,
  useCodeViewStore,
} from "./codeViewStore";

const mockInvoke = answerBatches(vi.mocked(invoke));
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

describe("loading a file's two sides", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((_cmd, args) =>
      Promise.resolve(`${(args as { sha: string }).sha[0]}-text`),
    );
    clearContentsCache();
    useCodeViewStore.setState({ ...initial, byReview: {} }, true);
  });

  const shas = () =>
    mockInvoke.mock.calls
      .filter(([cmd]) => cmd === "file_contents")
      .map(([, a]) => (a as { sha: string; path: string }).sha[0])
      .sort();

  it("reads base and head (one side for an added file)", async () => {
    const files = reviewFiles({ reviewId: ID });
    expect(await loadFileSides(files, file({ path: "src/a.rs" }))).toEqual([
      "b-text",
      "a-text",
    ]);
    expect(
      await loadFileSides(files, file({ path: "src/new.rs", status: "added" })),
    ).toEqual(["", "a-text"]);
  });

  it("prefetches a shown file through the same cache", async () => {
    const files = reviewFiles({
      reviewId: ID,
      files: [file({ path: "src/a.rs" })],
    });
    useCodeViewStore.setState({
      byReview: {
        [ID]: {
          ...initial.byReview[ID],
          since: false,
          files,
          loading: false,
          error: null,
          selected: null,
          order: [],
        },
      },
    });
    await prefetchFile(ID, "src/a.rs");
    await loadFileSides(files, files.files[0]);
    expect(shas()).toEqual(["a", "b"]);
  });

  it("prefetches nothing for a file no editor shows", () => {
    const files = reviewFiles({
      reviewId: ID,
      files: [
        file({ path: "img.png", binary: true }),
        file({ path: "big.json", tooLarge: true }),
        file({ path: "gen.rs", additions: 1400, deletions: 200 }),
      ],
    });
    useCodeViewStore.setState({
      byReview: {
        [ID]: {
          since: false,
          files,
          loading: false,
          error: null,
          selected: null,
          order: [],
        },
      },
    });
    expect(prefetchFile(ID, "img.png")).toBeNull();
    expect(prefetchFile(ID, "big.json")).toBeNull();
    expect(prefetchFile(ID, "gen.rs")).toBeNull();
    expect(prefetchFile(ID, "missing.rs")).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
