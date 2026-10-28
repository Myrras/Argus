import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { WelcomeScreen } from "./WelcomeScreen";

describe("WelcomeScreen", () => {
  it("greets and points at selecting a review", () => {
    render(<WelcomeScreen />);

    expect(screen.getByText("Welcome to Argus")).toBeInTheDocument();
    expect(
      screen.getByText("Select a review to see its threads."),
    ).toBeInTheDocument();
  });

  it("disables the refresh button without a refresh, or while one runs", () => {
    const { rerender } = render(<WelcomeScreen />);
    expect(screen.getByRole("button", { name: "Refresh now" })).toBeDisabled();
    rerender(<WelcomeScreen onRefresh={vi.fn()} refreshing />);
    expect(screen.getByRole("button", { name: "Refreshing…" })).toBeDisabled();
  });

  it("refreshes on click", () => {
    const onRefresh = vi.fn();
    render(<WelcomeScreen onRefresh={onRefresh} />);

    fireEvent.click(screen.getByRole("button", { name: "Refresh now" }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});
