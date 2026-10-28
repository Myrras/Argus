import { act, fireEvent, render, screen } from "@testing-library/react";
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
  it("keeps Tab inside and gives the focus back on close", async () => {
    const opener = document.createElement("button");
    opener.textContent = "open it";
    document.body.appendChild(opener);
    opener.focus();
    const view = (open: boolean) => (
      <Modal
        open={open}
        title="Hello"
        onClose={() => {}}
        footer={<button type="button">Last</button>}
      >
        <input aria-label="Field" />
      </Modal>
    );
    const { rerender } = render(view(true));
    const close = screen.getByRole("button", { name: "Close dialog" });
    const last = screen.getByRole("button", { name: "Last" });
    // From the last control Tab wraps to the first, Shift+Tab back.
    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(close).toHaveFocus();
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(last).toHaveFocus();
    // The card itself (focused on open) wraps to the last on Shift+Tab.
    screen.getByRole("dialog").focus();
    fireEvent.keyDown(screen.getByRole("dialog"), {
      key: "Tab",
      shiftKey: true,
    });
    expect(last).toHaveFocus();
    rerender(view(false));
    await act(async () => {});
    expect(opener).toHaveFocus();
    opener.remove();
  });
});
