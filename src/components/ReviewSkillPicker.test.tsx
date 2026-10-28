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
  error: vi.fn(() => Promise.resolve()),
  warn: vi.fn(() => Promise.resolve()),
}));

import { invoke } from "@tauri-apps/api/core";

import type { ReviewSkillList, Settings } from "../ipc/types";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useReviewSkillsStore } from "../state/reviewSkillsStore";
import { useReviewStore } from "../state/reviewStore";
import { settings, status } from "../test/fixtures";
import { ReviewSkillsSection } from "./ReviewSkillPicker";

const mockInvoke = vi.mocked(invoke);
const initialBackend = useBackendSettingsStore.getState();
const WIDGETS = "github.com/acme-fixture/widgets";

const skillList: ReviewSkillList = {
  skills: [
    { name: "my-review", description: "My own checklist.", source: "user" },
    {
      name: "fixture-tools:pr-review",
      description: "Reviews pull requests for SQL injection.",
      source: "plugin",
    },
    {
      name: "code-review",
      description: "The project's own review.",
      source: "project",
    },
  ],
  autoPick: "code-review",
  projectListed: true,
};

/** A fake core: `update_settings` stores what it gets. */
function backend(initial: Settings) {
  let stored = initial;
  mockInvoke.mockImplementation(((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve(stored);
      case "update_settings":
        stored = (args as { settings: Settings }).settings;
        return Promise.resolve(stored);
      case "list_review_skills":
        return Promise.resolve(skillList);
      case "provider_status":
        return Promise.resolve(status());
      case "gitlab_token_present":
        return Promise.resolve(false);
      default:
        return Promise.resolve(undefined);
    }
  }) as typeof invoke);
}

const saved = () =>
  mockInvoke.mock.calls
    .filter(([cmd]) => cmd === "update_settings")
    .map(([, args]) => (args as { settings: Settings }).settings.reviewSkills);

async function renderSection(initial: Settings) {
  backend(initial);
  await useBackendSettingsStore.getState().load();
  render(<ReviewSkillsSection />);
  const card = screen.getByRole("group", { name: WIDGETS });
  // Listed when the section shows.
  await waitFor(() =>
    expect(useReviewSkillsStore.getState().lists[WIDGETS]).toBeDefined(),
  );
  return card;
}

const open = (card: HTMLElement) =>
  fireEvent.click(
    within(card).getByRole("button", {
      name: `Choose the review skill of ${WIDGETS}`,
    }),
  );

beforeEach(() => {
  vi.clearAllMocks();
  useBackendSettingsStore.setState({ ...initialBackend }, true);
  useReviewSkillsStore.setState({ lists: {}, loading: {}, errors: {} });
  useReviewStore.setState({ reviews: [] });
});

describe("Review skill picker", () => {
  it("lists Auto and None first, then the skills by source, with descriptions", async () => {
    const card = await renderSection(
      settings({ clonePaths: { [WIDGETS]: "/src/widgets" } }),
    );
    expect(card).toHaveTextContent("Auto (project's own: code-review)");
    expect(mockInvoke).toHaveBeenCalledWith("list_review_skills", {
      repo: WIDGETS,
    });
    open(card);
    const group = screen.getByRole("radiogroup", {
      name: `Review skill for ${WIDGETS}`,
    });
    const radios = within(group).getAllByRole("radio");
    expect(radios.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Auto (project's own: code-review)",
      "None",
      "my-review, yours",
      "fixture-tools:pr-review, plugin",
      "code-review, project",
    ]);
    expect(radios[0]).toHaveAttribute("aria-checked", "true");
    expect(within(group).getByText("Your skills")).toBeInTheDocument();
    expect(within(group).getByText("Plugin skills")).toBeInTheDocument();
    expect(within(group).getByText("This project")).toBeInTheDocument();
    expect(
      within(group).getByText("Reviews pull requests for SQL injection."),
    ).toBeInTheDocument();
    // The filter has the focus; arrows walk the choices.
    const filter = screen.getByLabelText(
      `Filter the review skills of ${WIDGETS}`,
    );
    expect(filter).toHaveFocus();
    fireEvent.keyDown(filter, { key: "ArrowDown" });
    expect(radios[0]).toHaveFocus();
    fireEvent.keyDown(radios[0], { key: "ArrowDown" });
    expect(radios[1]).toHaveFocus();
    fireEvent.keyDown(radios[1], { key: "ArrowUp" });
    fireEvent.keyDown(radios[0], { key: "ArrowUp" });
    expect(filter).toHaveFocus();
  });

  it("filters the skills but keeps Auto and None", async () => {
    const card = await renderSection(
      settings({ clonePaths: { [WIDGETS]: "/src/widgets" } }),
    );
    open(card);
    const filter = screen.getByLabelText(
      `Filter the review skills of ${WIDGETS}`,
    );
    fireEvent.change(filter, { target: { value: "sql" } });
    const group = screen.getByRole("radiogroup");
    expect(
      within(group)
        .getAllByRole("radio")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual([
      "Auto (project's own: code-review)",
      "None",
      "fixture-tools:pr-review, plugin",
    ]);
    fireEvent.change(filter, { target: { value: "nothing like it" } });
    expect(within(group).getByText(/No skill matches/)).toHaveTextContent(
      "No skill matches “nothing like it”.",
    );
    expect(within(group).getAllByRole("radio")).toHaveLength(2);
  });

  it("saves a skill, None and Auto at once, per repository", async () => {
    const card = await renderSection(
      settings({ clonePaths: { [WIDGETS]: "/src/widgets" } }),
    );
    open(card);
    fireEvent.click(
      screen.getByRole("radio", { name: "fixture-tools:pr-review, plugin" }),
    );
    await waitFor(() =>
      expect(saved()).toEqual([
        {
          [WIDGETS]: {
            kind: "skill",
            source: "plugin",
            name: "fixture-tools:pr-review",
          },
        },
      ]),
    );
    // Closed again, the card says what is chosen.
    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(card).toHaveTextContent("fixture-tools:pr-review (plugin)");

    open(card);
    expect(
      screen.getByRole("radio", { name: "fixture-tools:pr-review, plugin" }),
    ).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "None" }));
    await waitFor(() => expect(saved()).toHaveLength(2));
    expect(saved()[1]).toEqual({ [WIDGETS]: { kind: "none" } });
    expect(card).toHaveTextContent("None");

    open(card);
    fireEvent.click(
      screen.getByRole("radio", { name: "Auto (project's own: code-review)" }),
    );
    await waitFor(() => expect(saved()).toHaveLength(3));
    // Auto is the default: no entry is kept.
    expect(saved()[2]).toEqual({});
    // Choosing what is already chosen saves nothing.
    open(card);
    fireEvent.click(
      screen.getByRole("radio", { name: "Auto (project's own: code-review)" }),
    );
    expect(saved()).toHaveLength(3);
  });

  it("marks a chosen skill that is gone", async () => {
    const card = await renderSection(
      settings({
        reviewSkills: {
          [WIDGETS]: {
            kind: "skill",
            source: "user",
            name: "uninstalled-review",
          },
        },
      }),
    );
    expect(card).toHaveTextContent("uninstalled-review (yours)");
    open(card);
    const gone = screen.getByRole("radio", { name: "uninstalled-review" });
    expect(gone).toHaveAttribute("aria-checked", "true");
    expect(gone).toHaveTextContent("Not found any more (yours): runs use Auto");
  });

  it("lists the repositories of reviews too, and says when there are none", async () => {
    backend(settings());
    await useBackendSettingsStore.getState().load();
    const { unmount } = render(<ReviewSkillsSection />);
    expect(screen.getByText(/No repositories yet/)).toBeInTheDocument();
    unmount();
    useReviewStore.setState({
      reviews: [
        { host: "gitlab.example.test", repo: "acme-fixture/api" },
      ] as never,
    });
    render(<ReviewSkillsSection />);
    expect(
      screen.getByRole("group", {
        name: "gitlab.example.test/acme-fixture/api",
      }),
    ).toBeInTheDocument();
  });
});
