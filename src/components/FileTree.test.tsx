import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { file } from "../test/fixtures";
import { FileTree, type FileTreeProps } from "./FileTree";

const files = [
  file({ path: "src/cache/store.rs" }),
  file({ path: "src/cache/ttl.rs", viewed: true }),
  file({
    path: "src/new_name.rs",
    oldPath: "src/old_name.rs",
    status: "renamed",
  }),
  file({ path: "README.md", status: "added", additions: 3, deletions: 0 }),
];

function props(over: Partial<FileTreeProps> = {}): FileTreeProps {
  return {
    files,
    mode: "tree",
    selected: null,
    counts: new Map([["src/cache/store.rs", 2]]),
    onSelect: vi.fn(),
    onToggleViewed: vi.fn(),
    ...over,
  };
}

const tree = () => screen.getByRole("tree", { name: "Changed files" });

describe("FileTree", () => {
  it("nests files in folders that fold and unfold", () => {
    render(<FileTree {...props()} />);
    const src = within(tree()).getByRole("treeitem", { name: "src" });
    expect(src).toHaveAttribute("aria-expanded", "true");
    expect(within(tree()).getByText("store.rs")).toBeInTheDocument();

    fireEvent.click(within(src).getByRole("button"));
    expect(src).toHaveAttribute("aria-expanded", "false");
    expect(within(tree()).queryByText("store.rs")).toBeNull();
    // Files outside the folder stay.
    expect(within(tree()).getByText("README.md")).toBeInTheDocument();

    fireEvent.click(within(src).getByRole("button"));
    expect(within(tree()).getByText("store.rs")).toBeInTheDocument();
  });

  it("lists full paths in flat mode and names a rename's old path", () => {
    render(<FileTree {...props({ mode: "flat" })} />);
    expect(within(tree()).queryByRole("treeitem", { name: "src" })).toBeNull();
    expect(within(tree()).getByText("src/cache/store.rs")).toBeInTheDocument();
    expect(
      within(tree()).getByTitle("src/old_name.rs → src/new_name.rs"),
    ).toBeInTheDocument();
  });

  it("selects files, toggles viewed and marks the selection", () => {
    const p = props({ selected: "README.md" });
    render(<FileTree {...p} />);
    fireEvent.click(within(tree()).getByText("store.rs"));
    expect(p.onSelect).toHaveBeenCalledWith("src/cache/store.rs");
    const ttl = within(tree()).getByRole("checkbox", {
      name: "Viewed src/cache/ttl.rs",
    });
    expect(ttl).toBeChecked();
    fireEvent.click(ttl);
    expect(p.onToggleViewed).toHaveBeenCalledWith(files[1]);
    const selected = within(tree())
      .getAllByRole("treeitem")
      .filter((i) => i.getAttribute("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0]).toHaveTextContent("README.md");
  });

  it("leads with the Conversation entry", () => {
    const onSelect = vi.fn();
    render(
      <FileTree
        {...props({
          conversation: {
            open: 1,
            total: 3,
            selected: true,
            onSelect,
          },
        })}
      />,
    );
    const first = within(tree()).getAllByRole("treeitem")[0];
    expect(first).toHaveTextContent("Conversation");
    expect(first).toHaveAttribute("aria-selected", "true");
    fireEvent.click(within(first).getByRole("button"));
    expect(onSelect).toHaveBeenCalled();
  });

  it("shows a badge only for files with open threads of yours", () => {
    render(<FileTree {...props()} />);
    const badge = within(tree()).getByLabelText("2 open threads of yours");
    expect(badge).toHaveTextContent("2");
    expect(within(tree()).getAllByLabelText(/threads? of yours/)).toHaveLength(
      1,
    );
  });

  it("shows the Conversation's open count of yours, no total pill", () => {
    render(
      <FileTree
        {...props({
          conversation: {
            open: 2,
            total: 5,
            selected: false,
            onSelect: vi.fn(),
          },
        })}
      />,
    );
    const first = within(tree()).getAllByRole("treeitem")[0];
    expect(first).toHaveTextContent(/^Conversation2 open$/);
    expect(within(first).getByText("2 open")).toHaveAttribute(
      "title",
      "2 open threads of yours, 5 threads in all",
    );
  });

  it("says when nothing matches, unless told not to", () => {
    const { rerender } = render(<FileTree {...props({ files: [] })} />);
    expect(screen.getByText("No files match.")).toBeInTheDocument();
    rerender(<FileTree {...props({ files: [], empty: "No changes." })} />);
    expect(screen.getByText("No changes.")).toBeInTheDocument();
    rerender(<FileTree {...props({ files: [], empty: null })} />);
    expect(screen.queryByText(/No /)).toBeNull();
  });
});
