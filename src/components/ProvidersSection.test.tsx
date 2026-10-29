import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Account, RefreshError } from "../ipc/types";
import { account, accounts } from "../test/fixtures";
import { bannerItems, currentErrors } from "../lib/banner";
import { useLayoutStore } from "../state/layoutStore";
import { ProvidersSection } from "./ProvidersSection";

const GITLAB = "gitlab.example.test";

/** The fixture's GitLab account with `overrides`. */
function gitlab(overrides: Partial<Account>): Account {
  return account({ provider: "gitlab", host: GITLAB, ...overrides });
}

function error(overrides: Partial<RefreshError>): RefreshError {
  return {
    provider: "gitlab",
    host: GITLAB,
    review: null,
    message: "Could not fetch reviews: timed out",
    unavailable: false,
    rateLimitedUntil: null,
    ...overrides,
  };
}

describe("ProvidersSection", () => {
  it("lists each enabled account's state and no problems when all work", () => {
    render(
      <ProvidersSection
        accounts={[
          ...accounts(),
          account({ host: "ghe.example.test", enabled: false }),
        ]}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.getAllByText("Connected")).toHaveLength(2);
    expect(screen.getByText(GITLAB)).toBeInTheDocument();
    // Switched off: not in use, not listed.
    expect(screen.queryByText("ghe.example.test")).not.toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("lists several hosts of one platform each on its own line", () => {
    render(
      <ProvidersSection
        accounts={[
          gitlab({}),
          gitlab({ host: "gitlab.two.test", key: "gitlab:gitlab.two.test" }),
        ]}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.getAllByText("GitLab")).toHaveLength(2);
    expect(screen.getByText("gitlab.two.test")).toBeInTheDocument();
  });

  it("says a CLI is missing, and opens Settings on it", () => {
    const onOpenSettings = vi.fn();
    render(
      <ProvidersSection
        accounts={[
          account(),
          gitlab({
            key: "gitlab:",
            host: "",
            user: null,
            state: { kind: "cliMissing", hint: "Install the GitLab CLI." },
            loginCommand: "glab auth login",
          }),
        ]}
        errors={[]}
        onOpenSettings={onOpenSettings}
      />,
    );
    expect(screen.getByText("glab not found")).toBeInTheDocument();
    expect(
      screen.getByText(
        "GitLab unavailable: the glab CLI was not found. Install it and run glab auth login.",
      ),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenCalledWith("gitlab:");
  });

  it("says glab is not logged in to the host, with the exact command", () => {
    const onOpenSettings = vi.fn();
    render(
      <ProvidersSection
        accounts={[
          gitlab({ user: null, state: { kind: "notLoggedIn", message: null } }),
        ]}
        errors={[
          error({
            message:
              "Not logged in to gitlab.example.test: run `glab auth login --hostname gitlab.example.test`.",
            unavailable: true,
          }),
        ]}
        onOpenSettings={onOpenSettings}
      />,
    );
    expect(screen.getByText("Not logged in")).toBeInTheDocument();
    // The account explains it: the refresh's own error is not repeated.
    expect(screen.queryByText(/\): Not logged in to/)).not.toBeInTheDocument();
    expect(
      screen.getByText(
        "GitLab unavailable: glab is not logged in to gitlab.example.test. Run glab auth login --hostname gitlab.example.test.",
      ),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenCalledWith(`gitlab:${GITLAB}`);
  });

  it("says nothing of a host that is switched off", () => {
    render(
      <ProvidersSection
        accounts={[
          account(),
          gitlab({
            enabled: false,
            state: { kind: "notLoggedIn", message: null },
          }),
        ]}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("says nothing of a host that is off by default or a CLI never set up", () => {
    const off = [
      gitlab({
        host: "gitlab.com",
        key: "gitlab:gitlab.com",
        enabled: false,
        state: { kind: "error", message: "glab could not reach gitlab.com." },
      }),
      gitlab({
        key: "gitlab:",
        host: "",
        enabled: false,
        state: { kind: "cliMissing", hint: "Install glab." },
      }),
    ];
    expect(bannerItems([account(), ...off], [])).toEqual([]);
    render(
      <ProvidersSection
        accounts={[account(), ...off]}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("tells a host it could not check from a missing login", () => {
    render(
      <ProvidersSection
        accounts={[
          gitlab({
            state: {
              kind: "error",
              message: "glab could not reach gitlab.example.test.",
            },
          }),
        ]}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.getByText("Couldn't check")).toBeInTheDocument();
    expect(
      screen.getByText(
        "GitLab (gitlab.example.test): glab could not reach gitlab.example.test.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/not logged in/)).not.toBeInTheDocument();
  });

  it("says how to start without any account", () => {
    const onOpenSettings = vi.fn();
    render(
      <ProvidersSection
        accounts={[]}
        errors={[]}
        onOpenSettings={onOpenSettings}
      />,
    );
    expect(screen.getByText("No accounts in use.")).toBeInTheDocument();
    expect(
      screen.getByText(
        "No accounts yet: log in with gh auth login or glab auth login.",
      ),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenCalledWith(undefined);
  });

  it("collapses to its title, counting the problems", () => {
    useLayoutStore.setState({ providersCollapsed: false });
    render(
      <ProvidersSection
        accounts={[gitlab({ state: { kind: "notLoggedIn", message: null } })]}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    const toggle = screen.getByRole("button", { name: /Providers/ });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(toggle);
    expect(useLayoutStore.getState().providersCollapsed).toBe(true);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(toggle).toHaveTextContent("Providers (1)");
  });

  it("explains a missing gh once, not again for its refresh error", () => {
    const items = bannerItems(
      [
        account({
          key: "github:",
          host: "",
          state: { kind: "cliMissing", hint: "Install gh." },
        }),
        gitlab({}),
      ],
      [
        error({
          provider: "github",
          host: "github.com",
          message: "The GitHub CLI (gh) is missing or not logged in.",
          unavailable: true,
        }),
      ],
    );
    expect(items.map((i) => i.key)).toEqual(["missing:github"]);
  });

  it("shows refresh errors without blocking, grouped per provider", () => {
    render(
      <ProvidersSection
        accounts={accounts()}
        errors={[
          error({}),
          error({ review: "00000000-0000-4000-8000-000000000001" }),
          error({
            review: "00000000-0000-4000-8000-000000000002",
            message: "not found",
          }),
        ]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        "GitLab (gitlab.example.test): Could not fetch reviews: timed out",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("GitLab: 2 reviews could not be refreshed."),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Connected")).toHaveLength(2);
  });

  it("shows a rate limit as 'rate limited until HH:MM', once", () => {
    const until = "2026-10-10T13:05:00Z";
    const d = new Date(until);
    const hhmm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    const message =
      "GitLab is rate limited until 99:99; Argus tries again then";
    render(
      <ProvidersSection
        accounts={accounts()}
        errors={[error({ message, rateLimitedUntil: until })]}
        onOpenSettings={vi.fn()}
      />,
    );
    const item = screen.getByText(
      `GitLab (gitlab.example.test): rate limited until ${hhmm}`,
    );
    expect(item).toBeInTheDocument();
    // The core's own message stays available.
    expect(item.closest("li")).toHaveAttribute("title", message);
    expect(screen.queryByText(/Argus tries again/)).not.toBeInTheDocument();
  });

  it("drops a provider's errors from a refresh that began before its accounts changed", () => {
    const report = {
      changed: [],
      errors: [
        error({ message: "Not logged in to gitlab.example.test." }),
        error({
          provider: "github",
          host: "github.com",
          message: "The GitHub CLI (gh) is missing or not logged in.",
        }),
      ],
      refreshedAt: "2026-10-19T12:12:41.100Z",
    };
    const at = (iso: string) => Date.parse(iso);
    expect(currentErrors(null, {})).toEqual([]);
    expect(currentErrors(report, {})).toEqual(report.errors);
    // GitLab's accounts changed after the refresh began: only GitHub's stays.
    expect(
      currentErrors(report, { gitlab: at("2026-10-19T12:12:41.500Z") }),
    ).toEqual([report.errors[1]]);
    // A refresh that began after the change keeps its errors.
    expect(
      currentErrors(report, { gitlab: at("2026-10-19T12:12:41.000Z") }),
    ).toEqual(report.errors);
  });
});
