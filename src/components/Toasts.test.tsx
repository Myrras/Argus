import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useToastStore } from "../state/toastStore";
import { Toasts } from "./Toasts";

const initialState = useToastStore.getState();

describe("Toasts", () => {
  beforeEach(() => {
    useToastStore.setState(initialState, true);
  });

  it("renders an action button and invokes it on click", () => {
    const onClick = vi.fn();
    useToastStore.getState().push({
      kind: "error",
      message: "Could not open the folder",
      detail: "permission denied",
      sticky: true,
      action: { label: "Retry", onClick },
    });

    render(<Toasts />);
    fireEvent.click(screen.getByText("Retry"));

    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("omits the action button when a toast has none", () => {
    useToastStore.getState().push({ kind: "success", message: "Saved" });

    render(<Toasts />);

    expect(screen.queryByText("Retry")).not.toBeInTheDocument();
  });
});
