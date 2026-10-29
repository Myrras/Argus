import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve("/Users/fixture/bin/picked.sh")),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type {
  GeneratedScript,
  ScriptDraft,
  ScriptGenerationStatus,
  Settings,
  SubmitScript,
} from "../ipc/types";
import { isMac } from "../lib/platform";
import { newScript } from "../lib/submitScripts";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import {
  applyScriptsDone,
  useScriptResultsStore,
} from "../state/scriptResultsStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useSubmitScriptsStore } from "../state/submitScriptsStore";
import { useToastStore } from "../state/toastStore";
import { scriptDraft, settings, accounts } from "../test/fixtures";
import { SettingsModal } from "./SettingsModal";
import { BACKGROUND_NOTE, READ_FIRST_NOTE } from "./ScriptWriter";
import { Toasts } from "./Toasts";

const mockInvoke = vi.mocked(invoke);
const initialBackend = useBackendSettingsStore.getState();
const initialScripts = useSubmitScriptsStore.getState();
const DIR = "/Users/fixture/Library/Argus/scripts";

const GENERATED: GeneratedScript = {
  name: "Tick the checklist",
  verdicts: ["approve"],
  script: "#!/bin/sh\nset -eu\necho ticked\n",
  explanation: "Ticks every task in the description.",
};

interface Backend {
  generation?: ScriptGenerationStatus;
  save?: (args: Record<string, unknown>) => string;
}

/**
 * The fake core's drafts: start/send put a request under way, `finish`
 * plays the agent's answer as `script-draft:state` does.
 */
let drafts: ScriptDraft[] = [];
let revision = 0;

function putDraft(d: ScriptDraft): ScriptDraft {
  revision += 1;
  const next = { ...d, revision, updatedAt: new Date().toISOString() };
  drafts = drafts.some((x) => x.id === d.id)
    ? drafts.map((x) => (x.id === d.id ? next : x))
    : [...drafts, next];
  return next;
}

const draftOf = (id: unknown) => drafts.find((d) => d.id === id)!;

function asking(d: ScriptDraft, prompt: string): ScriptDraft {
  return {
    ...d,
    status: "running",
    error: null,
    turns: [
      ...d.turns,
      {
        prompt,
        at: new Date().toISOString(),
        result: null,
        error: null,
        attachments: [],
        notes: [],
      },
    ],
  };
}

/** The agent answered the draft's last request (the event). */
function finish(id: string, result: GeneratedScript = GENERATED) {
  const d = draftOf(id);
  const turns = d.turns.map((t, i) =>
    i === d.turns.length - 1 ? { ...t, result } : t,
  );
  const next = putDraft({ ...d, turns, status: "idle", title: result.name });
  act(() => useSubmitScriptsStore.getState().applyDraft(next));
}

/** The agent failed on the draft's last request (the event). */
function fail(id: string, error: string) {
  const d = draftOf(id);
  const turns = d.turns.map((t, i) =>
    i === d.turns.length - 1 ? { ...t, error } : t,
  );
  const next = putDraft({ ...d, turns, status: "failed", error });
  act(() => useSubmitScriptsStore.getState().applyDraft(next));
}

/** A fake core: `update_settings` stores what it gets. */
function backend(initial: Settings, opts: Backend = {}) {
  let stored = initial;
  mockInvoke.mockImplementation(((cmd: string, args?: unknown) => {
    const a = (args ?? {}) as Record<string, unknown>;
    try {
      switch (cmd) {
        case "get_settings":
          return Promise.resolve(stored);
        case "update_settings":
          stored = (a as { settings: Settings }).settings;
          return Promise.resolve(stored);
        case "list_accounts":
          return Promise.resolve(accounts());
        case "submit_scripts_dir":
          return Promise.resolve(DIR);
        case "script_generation_status":
          return Promise.resolve(
            opts.generation ?? {
              available: true,
              adapter: "claude",
              reason: null,
            },
          );
        case "start_script_draft": {
          const prompt = a.description as string;
          const scriptId = (a.scriptId as string | null) ?? null;
          const saved = stored.submitScripts?.find((x) => x.id === scriptId);
          const base = saved
            ? {
                name: saved.name,
                verdicts: saved.verdicts,
                script: "#!/bin/sh\necho old\n",
                explanation: "",
              }
            : null;
          const d = scriptDraft({
            scriptId,
            base,
            title: saved?.name ?? prompt,
            turns: [],
          });
          return Promise.resolve(putDraft(asking(d, prompt)));
        }
        case "send_script_draft":
          return Promise.resolve(
            putDraft(asking(draftOf(a.draftId), a.message as string)),
          );
        case "cancel_script_draft": {
          const d = draftOf(a.draftId);
          const turns = d.turns.map((t, i) =>
            i === d.turns.length - 1 ? { ...t, error: "Stopped." } : t,
          );
          return Promise.resolve(
            putDraft({ ...d, turns, status: "cancelled" }),
          );
        }
        case "retry_script_draft": {
          const d = draftOf(a.draftId);
          const turns = d.turns.map((t, i) =>
            i === d.turns.length - 1 ? { ...t, error: null } : t,
          );
          return Promise.resolve(
            putDraft({ ...d, turns, status: "running", error: null }),
          );
        }
        case "list_script_drafts":
          return Promise.resolve(drafts);
        case "discard_script_draft":
          drafts = drafts.filter((d) => d.id !== a.draftId);
          return Promise.resolve(null);
        case "link_script_draft":
          return Promise.resolve(
            putDraft({
              ...draftOf(a.draftId),
              scriptId: a.scriptId as string,
            }),
          );
        case "save_submit_script":
          return Promise.resolve(
            (opts.save ?? (() => `${DIR}/tick-the-checklist.sh`))(a),
          );
        case "read_submit_script":
          return Promise.resolve("#!/bin/sh\necho old\n");
        default:
          return Promise.resolve(undefined);
      }
    } catch (e) {
      return Promise.reject(e);
    }
  }) as typeof invoke);
}

const calls = (cmd: string) =>
  mockInvoke.mock.calls
    .filter(([c]) => c === cmd)
    .map(([, args]) => args as Record<string, unknown>);
const updates = () =>
  calls("update_settings").map((a) => (a as { settings: Settings }).settings);
const lastScripts = (): SubmitScript[] =>
  updates().slice(-1)[0]?.submitScripts ?? [];

async function renderScripts() {
  render(
    <>
      <SettingsModal open onClose={() => undefined} />
      <Toasts />
    </>,
  );
  fireEvent.click(screen.getByRole("tab", { name: "Scripts" }));
  await waitFor(() =>
    expect(useBackendSettingsStore.getState().settings).not.toBeNull(),
  );
  await waitFor(() =>
    expect(useSubmitScriptsStore.getState().generation).not.toBeNull(),
  );
}

const tick = newScript("s-tick", {
  name: "Tick the checklist",
  command: "/Users/fixture/bin/tick.sh",
});

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  useSettingsRequestStore.getState().clear();
  useBackendSettingsStore.setState({ ...initialBackend }, true);
  useSubmitScriptsStore.setState({ ...initialScripts }, true);
  useScriptResultsStore.setState({ last: {} });
  useToastStore.setState({ toasts: [] });
  drafts = [];
  backend(settings({ submitScripts: [] }));
});

describe("Settings → Scripts", () => {
  it("sits between Editors and Shortcuts and opens on request", () => {
    useSettingsRequestStore.getState().open("scripts");
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "General",
      "Accounts",
      "AI",
      "Repositories",
      "Editors",
      "Scripts",
      "Shortcuts",
    ]);
    expect(screen.getByRole("tab", { name: "Scripts" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("explains when scripts run and what they get", async () => {
    await renderScripts();
    expect(
      screen.getByText(/only when you tick it in the submit dialog/),
    ).toBeInTheDocument();
    for (const v of [
      "ARGUS_PROVIDER",
      "ARGUS_HOST",
      "ARGUS_REPO",
      "ARGUS_NUMBER",
      "ARGUS_URL",
      "ARGUS_TITLE",
      "ARGUS_VERDICT",
      "ARGUS_HEAD_SHA",
      "ARGUS_CLONE",
    ]) {
      expect(screen.getByRole("cell", { name: v })).toBeInTheDocument();
    }
    expect(screen.getByText(/one JSON object on stdin/)).toBeInTheDocument();
    expect(screen.getByText(/empty temporary folder/)).toBeInTheDocument();
    expect(screen.getByText(/chmod \+x your-script\.sh/)).toBeInTheDocument();
    expect(
      screen.getByText(/Argus never passes its token/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Argus can't check what a script does. Only add scripts you trust.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Example: tick every checklist item (GitLab)"),
    ).toBeInTheDocument();
    expect(screen.getByText(DIR)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show in Finder" }));
    await waitFor(() =>
      expect(calls("reveal_submit_scripts_dir")).toHaveLength(1),
    );
  });

  it("adds, edits and removes a script through the settings", async () => {
    await renderScripts();
    expect(screen.getByText("No scripts yet.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add script" }));
    const card = await screen.findByRole("group", {
      name: "Script New script",
    });
    expect(lastScripts()).toMatchObject([
      {
        name: "New script",
        command: "",
        args: [],
        verdicts: ["approve"],
        repos: [],
        defaultOn: false,
        timeoutSecs: 60,
      },
    ]);

    // Text fields wait for Save.
    fireEvent.change(within(card).getByLabelText("Name"), {
      target: { value: "Tick the checklist" },
    });
    fireEvent.change(within(card).getByLabelText("Command"), {
      target: { value: "/Users/fixture/bin/tick.sh" },
    });
    fireEvent.change(within(card).getByLabelText("Arguments (optional)"), {
      target: { value: "--quiet\n" },
    });
    expect(updates()).toHaveLength(1);
    fireEvent.click(within(card).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(updates()).toHaveLength(2));
    expect(lastScripts()[0]).toMatchObject({
      name: "Tick the checklist",
      command: "/Users/fixture/bin/tick.sh",
      args: ["--quiet"],
    });

    // Checkboxes, switches and steppers apply at once.
    const saved = await screen.findByRole("group", {
      name: "Script Tick the checklist",
    });
    fireEvent.click(
      within(saved).getByRole("checkbox", { name: "Request changes" }),
    );
    await waitFor(() =>
      expect(lastScripts()[0]!.verdicts).toEqual(["approve", "requestChanges"]),
    );
    fireEvent.click(
      within(saved).getByRole("switch", { name: "Ticked by default" }),
    );
    await waitFor(() => expect(lastScripts()[0]!.defaultOn).toBe(true));
    fireEvent.click(
      within(saved).getByRole("button", {
        name: "Increase timeout of tick the checklist",
      }),
    );
    await waitFor(() => expect(lastScripts()[0]!.timeoutSecs).toBe(65));
    fireEvent.change(within(saved).getByLabelText("Repositories"), {
      target: { value: "gitlab.example.test/acme-fixture/api/" },
    });
    fireEvent.click(
      within(saved).getByRole("button", {
        name: "Add the repository to Tick the checklist",
      }),
    );
    await waitFor(() =>
      expect(lastScripts()[0]!.repos).toEqual([
        "gitlab.example.test/acme-fixture/api",
      ]),
    );

    // No verdict at all: never offered, and it says so.
    fireEvent.click(within(saved).getByRole("checkbox", { name: "Approve" }));
    fireEvent.click(
      within(saved).getByRole("checkbox", { name: "Request changes" }),
    );
    expect(
      await within(saved).findByText(
        "No verdict ticked: this script is never offered.",
      ),
    ).toBeInTheDocument();

    fireEvent.click(
      within(saved).getByRole("button", { name: "Remove Tick the checklist" }),
    );
    await waitFor(() => expect(lastScripts()).toEqual([]));
    // Nothing here runs anything.
    expect(calls("submit_review")).toEqual([]);
  });

  it("shows a script's last run from scripts:done", async () => {
    backend(settings({ submitScripts: [tick] }));
    await renderScripts();
    const card = screen.getByRole("group", {
      name: "Script Tick the checklist",
    });
    expect(within(card).getByText("Not run yet.")).toBeInTheDocument();
    act(() =>
      applyScriptsDone({
        reviewId: "r1",
        results: [
          {
            scriptId: "s-tick",
            name: "Tick the checklist",
            status: "timedOut",
            exitCode: null,
            stdout: "half way\n",
            stderr: "",
            durationMs: 60000,
            message: "Took longer than 60s and was stopped.",
            finishedAt: new Date().toISOString(),
          },
        ],
      }),
    );
    expect(within(card).getByText("Timed out")).toBeInTheDocument();
    expect(within(card).getByText(/half way/)).toBeInTheDocument();
    expect(
      await screen.findByText('Script "Tick the checklist": timed out'),
    ).toBeInTheDocument();
  });

  it("turns Write with AI off with the reason", async () => {
    backend(settings({ submitScripts: [] }), {
      generation: {
        available: false,
        adapter: "claude",
        reason: "The AI pass is off: turn it on in Settings → AI.",
      },
    });
    await renderScripts();
    const button = screen.getByRole("button", { name: "Write with AI…" });
    expect(button).toBeDisabled();
    expect(button.parentElement).toHaveAttribute(
      "title",
      "The AI pass is off: turn it on in Settings → AI.",
    );
    expect(
      screen.getByText(/Write with AI is off: The AI pass is off/),
    ).toBeInTheDocument();
  });
});

describe("Write with AI", () => {
  async function openWriter() {
    await renderScripts();
    fireEvent.click(screen.getByRole("button", { name: "Write with AI…" }));
    return screen.findByRole("dialog", { name: "Write a script with AI" });
  }

  /** Open the writer and send the first request; the draft's id. */
  async function startDraft(text = "Tick every checklist item when I approve") {
    const dialog = await openWriter();
    fireEvent.change(
      within(dialog).getByLabelText("Describe what the script should do"),
      { target: { value: text } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "Write it" }));
    await waitFor(() => expect(calls("start_script_draft")).toHaveLength(1));
    await within(dialog).findByText("Writing…");
    return { dialog, id: drafts[0]!.id };
  }

  const writeCalls = () =>
    [
      "save_submit_script",
      "update_settings",
      "submit_review",
      "link_script_draft",
    ].flatMap((c) => calls(c));

  it("writes in the background: close the dialog, a toast opens the result", async () => {
    const { dialog, id } = await startDraft();
    expect(calls("start_script_draft")).toEqual([
      {
        description: "Tick every checklist item when I approve",
        scriptId: null,
      },
    ]);
    expect(
      within(dialog).getByText(BACKGROUND_NOTE, { exact: false }),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Save script" }),
    ).toBeDisabled();
    // The tab shows that a draft is running.
    expect(
      screen.getByRole("tab", { name: "Scripts" }).querySelector(".spinner"),
    ).not.toBeNull();

    // Closing never stops it.
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: /Write/ })).toBeNull(),
    );
    expect(calls("cancel_script_draft")).toEqual([]);
    const row = screen.getByRole("listitem", {
      name: "Draft Tick every checklist item when I approve",
    });
    expect(within(row).getByText(/Writing… · 1 request/)).toBeInTheDocument();

    // It finishes: a toast with Open.
    finish(id);
    expect(
      await screen.findByText("Script ready: Tick the checklist"),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByRole("listitem", { name: "Draft Tick the checklist" }),
      ).getByText(/Ready/),
    ).toBeInTheDocument();
    expect(writeCalls()).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    const reopened = await screen.findByRole("dialog", {
      name: "Tick the checklist",
    });
    expect(within(reopened).getByText(READ_FIRST_NOTE)).toBeInTheDocument();
    expect(
      within(reopened).getByText("Ticks every task in the description."),
    ).toBeInTheDocument();
    expect(reopened.textContent).toContain("echo ticked");
    expect(within(reopened).getByLabelText("Name")).toHaveValue(
      "Tick the checklist",
    );

    // Only Save writes: the file, the entry, then the link.
    fireEvent.click(
      within(reopened).getByRole("button", { name: "Save script" }),
    );
    await waitFor(() => expect(calls("link_script_draft")).toHaveLength(1));
    expect(calls("save_submit_script")).toEqual([
      {
        name: "Tick the checklist",
        content: GENERATED.script,
        overwrite: false,
      },
    ]);
    const added = lastScripts()[0]!;
    expect(added).toMatchObject({
      name: "Tick the checklist",
      command: `${DIR}/tick-the-checklist.sh`,
      verdicts: ["approve"],
    });
    expect(calls("link_script_draft")).toEqual([
      { draftId: id, scriptId: added.id },
    ]);
    // Its card offers the draft.
    const card = await screen.findByRole("group", {
      name: "Script Tick the checklist",
    });
    expect(within(card).getByText("Draft with changes")).toBeInTheDocument();
    expect(calls("submit_review")).toEqual([]);
  });

  it("refines with follow-ups and shows the conversation", async () => {
    const { dialog, id } = await startDraft();
    finish(id);
    // Its own dialog is open: no toast.
    expect(screen.queryByText(/Script ready/)).toBeNull();
    const ask = await within(dialog).findByLabelText("Ask for changes…");
    fireEvent.change(ask, { target: { value: "Also on request changes" } });
    fireEvent.keyDown(ask, {
      key: "Enter",
      code: "Enter",
      metaKey: isMac,
      ctrlKey: !isMac,
    });
    await waitFor(() => expect(calls("send_script_draft")).toHaveLength(1));
    expect(calls("send_script_draft")[0]).toEqual({
      draftId: id,
      message: "Also on request changes",
    });
    expect(await within(dialog).findByText("Writing…")).toBeInTheDocument();
    // Sending again waits for the answer.
    fireEvent.change(ask, { target: { value: "More" } });
    expect(within(dialog).getByRole("button", { name: "Send" })).toBeDisabled();

    finish(id, {
      ...GENERATED,
      name: "Tick on both",
      verdicts: ["approve", "requestChanges"],
      script: "#!/bin/sh\nset -eu\necho both\n",
      explanation: "Now on both verdicts.",
    });
    const requests = within(dialog).getAllByLabelText("Your request");
    expect(requests.map((r) => r.textContent)).toEqual([
      "Tick every checklist item when I approve",
      "Also on request changes",
    ]);
    // The older version folded, the latest open.
    expect(
      within(dialog).getByText("Version 1: Tick the checklist"),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("Now on both verdicts.")).toBeVisible();
    expect(within(dialog).getByLabelText("Name")).toHaveValue("Tick on both");
    expect(
      within(dialog).getByRole("checkbox", { name: "Request changes" }),
    ).toBeChecked();
    expect(writeCalls()).toEqual([]);
  });

  it("streams the thinking on the running turn, then folds it", async () => {
    const { dialog, id } = await startDraft();
    const progress = (thinking: string, stage: "thinking" | "writing") => ({
      draftId: id,
      progress: {
        turn: 0,
        stage,
        startedAt: new Date(Date.now() - 65_000).toISOString(),
        thinking,
        thinkingCut: false,
        thinkingTokens: 300,
        answerChars: stage === "writing" ? 1200 : 0,
        notice: null,
      },
    });
    act(() =>
      useSubmitScriptsStore
        .getState()
        .applyProgress(progress("Reading the template.", "thinking")),
    );
    const status = await within(dialog).findByText("Thinking…");
    const line = status.closest("[role=status]") as HTMLElement;
    expect(within(line).getByLabelText("Elapsed").textContent).toMatch(
      /^1m 0[5-6]s$/,
    );
    // Stop sits on the running turn.
    expect(within(line).getByRole("button", { name: "Stop" })).toBeVisible();
    const thinking = within(dialog).getByLabelText("The agent's thinking");
    expect(thinking.textContent).toBe("Reading the template.");
    expect(thinking.closest("details")).toHaveAttribute("open");

    act(() =>
      useSubmitScriptsStore
        .getState()
        .applyProgress(
          progress("Reading the template.\nFour items to tick.", "writing"),
        ),
    );
    expect(
      await within(dialog).findByText("Writing the script…"),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("1,200 characters written")).toBeVisible();
    expect(
      within(dialog).getByLabelText("The agent's thinking").textContent,
    ).toContain("Four items to tick.");

    // Done: the thinking stays in this window, folded.
    finish(id);
    const summary = await within(dialog).findByText(/^Thought for /);
    expect(summary.closest("details")).not.toHaveAttribute("open");
    expect(within(dialog).queryByRole("status")).toBeNull();
    // A late report for an ended turn is dropped.
    act(() =>
      useSubmitScriptsStore
        .getState()
        .applyProgress(progress("late", "thinking")),
    );
    expect(within(dialog).queryByText("late")).toBeNull();
    expect(writeCalls()).toEqual([]);
  });

  it("shows what was attached and retries a failed request", async () => {
    const { dialog, id } = await startDraft("See the MR and the template");
    const d = draftOf(id);
    act(() =>
      useSubmitScriptsStore.getState().applyDraft(
        putDraft({
          ...d,
          turns: d.turns.map((t) => ({
            ...t,
            attachments: ["MR !4242", "Code_review.md"],
            notes: ["Not read: /etc/hosts (outside your local clones)"],
          })),
        }),
      ),
    );
    expect(
      await within(dialog).findByText("Attached: MR !4242 · Code_review.md"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        "Not read: /etc/hosts (outside your local clones)",
      ),
    ).toBeInTheDocument();

    fail(id, "Couldn't write the script: claude exited with status 1: boom.");
    expect(
      await within(dialog).findByText(
        "Couldn't write the script: claude exited with status 1: boom.",
      ),
    ).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(calls("retry_script_draft")).toEqual([{ draftId: id }]),
    );
    expect(await within(dialog).findByText("Writing…")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Retry" })).toBeNull();
    // The same request, not a new one.
    expect(within(dialog).getAllByLabelText("Your request")).toHaveLength(1);
    expect(writeCalls()).toEqual([]);
  });

  it("stops a draft from the Drafts section", async () => {
    const { dialog, id } = await startDraft("Post to chat");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    const row = await screen.findByRole("listitem", {
      name: "Draft Post to chat",
    });
    fireEvent.click(
      within(row).getByRole("button", { name: "Stop Post to chat" }),
    );
    await waitFor(() =>
      expect(calls("cancel_script_draft")).toEqual([{ draftId: id }]),
    );
    expect(
      await within(row).findByText(/Stopped · 1 request/),
    ).toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: /^Stop/ })).toBeNull();
    // Stopped by the user: no toast.
    expect(screen.queryByRole("alert")).toBeNull();

    // Discard forgets it.
    fireEvent.click(
      within(row).getByRole("button", { name: "Discard Post to chat" }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Drafts" })).toBeNull(),
    );
    expect(calls("discard_script_draft")).toEqual([{ draftId: id }]);
    expect(writeCalls()).toEqual([]);
  });

  it("toasts a failure that ended while the dialog was closed", async () => {
    const { dialog, id } = await startDraft("Post to chat");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    fail(id, "Couldn't write the script: the agent took longer than 180s.");
    expect(
      await screen.findByText("Couldn't write Post to chat"),
    ).toBeInTheDocument();
    const row = screen.getByRole("listitem", { name: "Draft Post to chat" });
    expect(
      within(row).getByText(/Failed: Couldn't write the script/),
    ).toBeInTheDocument();
  });

  it("asks before it replaces a script of the same name", async () => {
    let first = true;
    backend(settings({ submitScripts: [] }), {
      save: (a) => {
        if (first && !a.overwrite) {
          first = false;
          throw {
            kind: "conflict",
            message: "A script named tick-the-checklist.sh already exists.",
          };
        }
        return `${DIR}/tick-the-checklist.sh`;
      },
    });
    const { dialog, id } = await startDraft("Tick");
    finish(id);
    fireEvent.click(
      await within(dialog).findByRole("button", { name: "Save script" }),
    );
    expect(
      await within(dialog).findByText(/already exists\. Replace it\?/),
    ).toBeInTheDocument();
    expect(updates()).toEqual([]);
    fireEvent.click(within(dialog).getByRole("button", { name: "Replace it" }));
    await waitFor(() => expect(updates()).toHaveLength(1));
    expect(calls("save_submit_script").map((a) => a.overwrite)).toEqual([
      false,
      true,
    ]);
  });

  it("changes a saved script from its card, starting from its text", async () => {
    const saved = newScript("s-saved", {
      name: "Tick the checklist",
      command: `${DIR}/tick-the-checklist.sh`,
      verdicts: ["approve"],
    });
    backend(settings({ submitScripts: [saved, tick] }));
    await renderScripts();
    // Only a script Argus saved can be changed with AI.
    expect(
      screen.getAllByRole("button", {
        name: "Edit Tick the checklist with AI",
      }),
    ).toHaveLength(1);
    fireEvent.click(
      screen.getByRole("button", { name: "Edit Tick the checklist with AI" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Edit Tick the checklist with AI",
    });
    await waitFor(() => expect(dialog.textContent).toContain("echo old"));
    expect(
      within(dialog).getByText("The saved script you start from"),
    ).toBeInTheDocument();
    expect(calls("read_submit_script")).toEqual([{ scriptId: "s-saved" }]);
    fireEvent.change(
      within(dialog).getByLabelText("Describe what the script should do"),
      { target: { value: "Print what you ticked" } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "Write it" }));
    await waitFor(() => expect(calls("start_script_draft")).toHaveLength(1));
    expect(calls("start_script_draft")[0]).toEqual({
      description: "Print what you ticked",
      scriptId: "s-saved",
    });
    finish(drafts[0]!.id);
    fireEvent.click(
      await within(dialog).findByRole("button", { name: "Save script" }),
    );
    await waitFor(() => expect(updates()).toHaveLength(1));
    // Its own file is replaced, its entry updated (not a second one).
    expect(calls("save_submit_script")[0]!.overwrite).toBe(true);
    expect(lastScripts().map((s) => s.id)).toEqual(["s-saved", "s-tick"]);
    // Already linked from the start.
    expect(calls("link_script_draft")).toEqual([]);

    // Edit with AI again opens the same draft.
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: /Edit Tick the checklist/ }),
      ).toBeNull(),
    );
    expect(screen.getByText("Draft with changes")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Edit Tick the checklist with AI" }),
    );
    const again = await screen.findByRole("dialog", {
      name: "Edit Tick the checklist with AI",
    });
    expect(
      within(again)
        .getAllByLabelText("Your request")
        .map((r) => r.textContent),
    ).toEqual(["Print what you ticked"]);
  });
});
