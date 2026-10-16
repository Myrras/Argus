import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { status } from "../test/fixtures";
import { bannerItems } from "../lib/banner";
import { useLayoutStore } from "../state/layoutStore";
import { ProvidersSection } from "./ProvidersSection";

describe("ProvidersSection", () => {
  it("lists each provider's state and no problems when all work", () => {
    render(
      <ProvidersSection
        status={status()}
        errors={[]}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.getByText("Connected")).toBeInTheDocument();
    expect(screen.getByText("Token set")).toBeInTheDocument();
    expect(screen.getByText("gitlab.example.test")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("collapses to its title, counting the problems", () => {
    useLayoutStore.setState({ providersCollapsed: false });
    render(
      <ProvidersSection
        status={status({
          gitlab: { configured: false, baseUrl: "", tokenPresent: false },
        })}
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

  it("explains gh missing and GitLab not set up, once each", () => {
    const s = status({
      github: {
        enabled: true,
        host: "github.com",
        installed: false,
        loggedIn: false,
      },
      gitlab: { configured: false, baseUrl: "", tokenPresent: false },
    });
    const items = bannerItems(s, [
      {
        provider: "github",
        review: null,
        message: "The GitHub CLI (gh) is missing or not logged in.",
        unavailable: true,
      },
    ]);
    expect(items.map((i) => i.key)).toEqual(["gh-missing", "gl-setup"]);
  });

  it("shows refresh errors without blocking, grouped per provider", () => {
    const onOpenSettings = vi.fn();
    render(
      <ProvidersSection
        status={status({
          github: {
            enabled: true,
            host: "ghe.example.test",
            installed: true,
            loggedIn: false,
          },
        })}
        errors={[
          {
            provider: "gitlab",
            review: null,
            message: "Could not fetch reviews: timed out",
            unavailable: false,
          },
          {
            provider: "gitlab",
            review: "00000000-0000-4000-8000-000000000001",
            message: "Could not fetch reviews: timed out",
            unavailable: false,
          },
          {
            provider: "gitlab",
            review: "00000000-0000-4000-8000-000000000002",
            message: "not found",
            unavailable: false,
          },
        ]}
        onOpenSettings={onOpenSettings}
      />,
    );
    expect(
      screen.getByText(
        "GitHub unavailable: gh is not logged in for ghe.example.test. Run gh auth login.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("GitLab: Could not fetch reviews: timed out"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("GitLab: 2 reviews could not be refreshed."),
    ).toBeInTheDocument();
    expect(screen.getByText("Not logged in")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenCalled();
  });
});
