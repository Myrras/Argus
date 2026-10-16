import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { isMac } from "../lib/platform";
import { CommentComposer } from "./CommentComposer";

const submitKey = {
  key: "Enter",
  code: "Enter",
  metaKey: isMac,
  ctrlKey: !isMac,
};

function setup(body = "Looks **off**") {
  const props = {
    label: "Comment on line 3",
    body,
    onChange: vi.fn(),
    onCancel: vi.fn(),
    primary: { label: "Add to review", onClick: vi.fn() },
    secondary: { label: "Comment now", onClick: vi.fn() },
  };
  render(<CommentComposer {...props} />);
  return props;
}

describe("CommentComposer", () => {
  it("runs the primary action from its button and from Mod+Enter", () => {
    const p = setup();
    fireEvent.click(screen.getByRole("button", { name: "Add to review" }));
    expect(p.primary.onClick).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(screen.getByRole("textbox"), submitKey);
    expect(p.primary.onClick).toHaveBeenCalledTimes(2);
    expect(p.secondary.onClick).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Comment now" }));
    expect(p.secondary.onClick).toHaveBeenCalledTimes(1);
  });

  it("cancels from Escape and the Cancel button", () => {
    const p = setup();
    fireEvent.keyDown(screen.getByRole("textbox"), {
      key: "Escape",
      code: "Escape",
    });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(p.onCancel).toHaveBeenCalledTimes(2);
    expect(p.primary.onClick).not.toHaveBeenCalled();
  });

  it("does nothing on an empty text, nor from other keys", () => {
    const p = setup("   ");
    expect(
      screen.getByRole("button", { name: "Add to review" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Comment now" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("textbox"), submitKey);
    fireEvent.keyDown(screen.getByRole("textbox"), {
      key: "Enter",
      code: "Enter",
    });
    expect(p.primary.onClick).not.toHaveBeenCalled();
    expect(p.secondary.onClick).not.toHaveBeenCalled();
  });

  it("passes typing up and previews the markdown", () => {
    const p = setup();
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "new text" },
    });
    expect(p.onChange).toHaveBeenCalledWith("new text");
    fireEvent.click(screen.getByRole("tab", { name: "Preview" }));
    const preview = screen.getByTestId("composer-preview");
    expect(preview.querySelector("strong")).toHaveTextContent("off");
    expect(screen.queryByRole("textbox")).toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "Write" }));
    expect(screen.getByRole("textbox")).toHaveValue("Looks **off**");
  });

  it("shows the error and locks while busy", () => {
    render(
      <CommentComposer
        label="Reply"
        body="x"
        onChange={vi.fn()}
        onCancel={vi.fn()}
        busy
        error={{ kind: "postRejected", message: "GitHub: refused" }}
        primary={{ label: "Reply", busyLabel: "Replying…", onClick: vi.fn() }}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("GitHub: refused");
    expect(screen.getByRole("button", { name: "Replying…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  });
});
