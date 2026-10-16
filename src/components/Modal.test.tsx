import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Modal } from "./Modal";

describe("Modal", () => {
  it("renders outside its parent so clipped ancestors can't cut it off", () => {
    const { container } = render(
      <div style={{ overflow: "hidden" }}>
        <Modal open title="Hello" onClose={() => {}}>
          body
        </Modal>
      </div>,
    );
    const dialog = screen.getByRole("dialog", { name: "Hello" });
    expect(container.contains(dialog)).toBe(false);
    expect(document.body.contains(dialog)).toBe(true);
  });

  it("closes on Escape, the close button and a backdrop click", () => {
    const onClose = vi.fn();
    render(
      <Modal open title="Hello" onClose={onClose}>
        body
      </Modal>,
    );

    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Close dialog" }));
    fireEvent.mouseDown(screen.getByRole("presentation"));
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it("ignores clicks inside the card", () => {
    const onClose = vi.fn();
    render(
      <Modal open title="Hello" onClose={onClose}>
        body
      </Modal>,
    );

    fireEvent.mouseDown(screen.getByText("body"));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("renders nothing while closed", () => {
    render(
      <Modal open={false} title="Hello" onClose={() => {}}>
        body
      </Modal>,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
