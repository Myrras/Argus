import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { status } from "../test/fixtures";
import { bannerItems } from "../lib/banner";
import { ProviderBanner } from "./ProviderBanner";

describe("ProviderBanner", () => {
  it("is absent when every provider works", () => {
    const { container } = render(
      <ProviderBanner status={status()} errors={[]} onOpenSettings={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
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
      <ProviderBanner
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
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(onOpenSettings).toHaveBeenCalled();
  });
});
