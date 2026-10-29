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

import type { SubmitScript } from "../ipc/types";
import { newScript } from "../lib/submitScripts";
import { useBackendSettingsStore } from "../state/backendSettingsStore";
import { useReviewStore } from "../state/reviewStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { review, settings } from "../test/fixtures";
import { SubmitReviewModal } from "./SubmitReviewModal";

const mockInvoke = vi.mocked(invoke);
const initialReviews = useReviewStore.getState();
const initialBackend = useBackendSettingsStore.getState();

const r = review({ repo: "acme-fixture/widgets", host: "github.com" });

const tick = newScript("s-tick", {
  name: "Tick the checklist",
  command: "/Users/fixture/bin/tick.sh",
  defaultOn: true,
});
const notify = newScript("s-notify", {
  name: "Notify chat",
  command: "/Users/fixture/bin/notify.sh",
  verdicts: ["approve", "requestChanges"],
});
const elsewhere = newScript("s-elsewhere", {
  name: "Elsewhere",
  command: "/Users/fixture/bin/other.sh",
  repos: ["gitlab.example.test/acme-fixture/api"],
  defaultOn: true,
});
const sameRepo = newScript("s-same", {
  name: "Same repo, any case",
  command: "/Users/fixture/bin/same.sh",
  repos: ["GitHub.com/Acme-Fixture/Widgets/"],
  verdicts: ["comment"],
});

function withScripts(scripts: SubmitScript[]) {
  useBackendSettingsStore.setState({
    settings: settings({ submitScripts: scripts }),
  });
}

function renderDialog(onClose = () => undefined) {
  render(<SubmitReviewModal review={r} open onClose={onClose} />);
  return screen.getByRole("dialog");
}

beforeEach(() => {
  vi.clearAllMocks();
  useReviewStore.setState({ ...initialReviews, reviews: [r] }, true);
  useBackendSettingsStore.setState({ ...initialBackend }, true);
  useSettingsRequestStore.getState().clear();
  mockInvoke.mockImplementation(((cmd: string) =>
    Promise.resolve(
      cmd === "submit_review"
        ? { ...r, status: "approved" }
        : cmd === "list_reviews"
          ? []
          : undefined,
    )) as typeof invoke);
});

describe("SubmitReviewModal — Run after submitting", () => {
  it("lists the scripts for this repository and verdict, by their defaults", () => {
    withScripts([tick, notify, elsewhere, sameRepo]);
    const dialog = renderDialog();
    // No verdict yet: nothing to offer.
    expect(within(dialog).queryByText("Run after submitting")).toBeNull();

    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    expect(
      within(dialog).getByText("Run after submitting"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("checkbox", { name: "Tick the checklist" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("checkbox", { name: "Notify chat" }),
    ).not.toBeChecked();
    expect(within(dialog).queryByText("Elsewhere")).toBeNull();
    expect(within(dialog).queryByText("Same repo, any case")).toBeNull();
    expect(
      within(dialog).getByText(
        /Runs on your machine after the review is submitted/,
      ),
    ).toBeInTheDocument();

    // Another verdict: its scripts; the user's own ticks are kept.
    fireEvent.click(
      within(dialog).getByRole("checkbox", { name: "Notify chat" }),
    );
    fireEvent.click(within(dialog).getByRole("radio", { name: /Comment/ }));
    expect(within(dialog).queryByText("Tick the checklist")).toBeNull();
    expect(
      within(dialog).getByRole("checkbox", { name: "Same repo, any case" }),
    ).not.toBeChecked();
    fireEvent.click(
      within(dialog).getByRole("radio", { name: /Request changes/ }),
    );
    expect(
      within(dialog).getByRole("checkbox", { name: "Notify chat" }),
    ).toBeChecked();
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("sends the ticked ids with the submit, and nothing before it", async () => {
    withScripts([tick, notify, elsewhere]);
    const dialog = renderDialog();
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(
      within(dialog).getByRole("checkbox", { name: "Notify chat" }),
    );
    fireEvent.click(
      within(dialog).getByRole("checkbox", { name: "Tick the checklist" }),
    );
    expect(mockInvoke).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("submit_review", {
        reviewId: r.id,
        headSha: r.headSha,
        verdict: "approve",
        summary: null,
        scriptIds: ["s-notify"],
      }),
    );
  });

  it("offers a quiet way to the Scripts tab without any script", () => {
    withScripts([]);
    const onClose = vi.fn();
    const dialog = renderDialog(onClose);
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    expect(within(dialog).queryByText("Run after submitting")).toBeNull();
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Add a submit script" }),
    );
    expect(onClose).toHaveBeenCalled();
    expect(useSettingsRequestStore.getState().request).toBe("scripts");
  });

  it("links to the Scripts tab from the section", () => {
    withScripts([tick]);
    const onClose = vi.fn();
    const dialog = renderDialog(onClose);
    fireEvent.click(within(dialog).getByRole("radio", { name: /Approve/ }));
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Manage scripts" }),
    );
    expect(useSettingsRequestStore.getState().request).toBe("scripts");
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
