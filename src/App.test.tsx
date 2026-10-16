import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import App from "./App";

describe("App", () => {
  it("renders the Argus placeholder screen", () => {
    render(<App />);

    expect(screen.getByRole("heading", { name: "Argus" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "A review cockpit for GitLab merge requests and GitHub pull requests.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Toggle theme")).toBeInTheDocument();
  });
});
