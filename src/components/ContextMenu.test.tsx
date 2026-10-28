import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { ContextMenu } from "./ContextMenu";

function Harness({ onPick }: { onPick: (label: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        row
      </button>
      <ContextMenu
        open={open}
        x={10}
        y={10}
        onClose={() => setOpen(false)}
        items={[
          { label: "Open", onSelect: () => onPick("Open") },
          { label: "Disabled", onSelect: () => {}, disabled: true },
          { label: "Copy link", onSelect: () => onPick("Copy link") },
          { label: "Mark reviewed", onSelect: () => onPick("Mark") },
        ]}
      />
    </>
  );
}

describe("ContextMenu", () => {
  it("works from the keyboard and gives the focus back", async () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick} />);
    const row = screen.getByRole("button", { name: "row" });
    row.focus();
    fireEvent.click(row);
    const item = (name: string) => screen.getByRole("menuitem", { name });
    expect(item("Open")).toHaveFocus();
    const menu = screen.getByRole("menu");
    // Disabled items are skipped; the ends wrap.
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(item("Copy link")).toHaveFocus();
    fireEvent.keyDown(menu, { key: "End" });
    expect(item("Mark reviewed")).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(item("Open")).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(item("Mark reviewed")).toHaveFocus();
    fireEvent.keyDown(menu, { key: "Home" });
    expect(item("Open")).toHaveFocus();
    fireEvent.keyDown(window, { key: "Escape" });
    await act(async () => {});
    expect(row).toHaveFocus();
    expect(onPick).not.toHaveBeenCalled();
  });
});
