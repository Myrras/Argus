import { describe, expect, it } from "vitest";

import { file } from "../test/fixtures";
import { buildTree, filterFiles, orderFiles } from "./fileTree";

const files = [
  file({ path: "src/lib/b.ts" }),
  file({ path: "README.md" }),
  file({ path: "src/lib/a.ts" }),
  file({ path: "src/app/deep/x.ts" }),
  file({ path: "src/new.ts", oldPath: "src/old.ts", status: "renamed" }),
];

describe("fileTree", () => {
  it("builds folders first with single-child chains compacted", () => {
    const tree = buildTree(files);
    expect(tree.map((n) => n.name)).toEqual(["src", "README.md"]);
    const src = tree[0];
    if (src.kind !== "dir") throw new Error("src is a folder");
    expect(src.children.map((n) => n.name)).toEqual([
      "app/deep",
      "lib",
      "new.ts",
    ]);
    expect(orderFiles(files, "tree").map((f) => f.path)).toEqual([
      "src/app/deep/x.ts",
      "src/lib/a.ts",
      "src/lib/b.ts",
      "src/new.ts",
      "README.md",
    ]);
    expect(orderFiles(files, "flat").map((f) => f.path)).toEqual([
      "README.md",
      "src/app/deep/x.ts",
      "src/lib/a.ts",
      "src/lib/b.ts",
      "src/new.ts",
    ]);
  });

  it("filters on the path or the path before a rename", () => {
    expect(filterFiles(files, "LIB/").map((f) => f.path)).toEqual([
      "src/lib/b.ts",
      "src/lib/a.ts",
    ]);
    expect(filterFiles(files, "old").map((f) => f.path)).toEqual([
      "src/new.ts",
    ]);
    expect(filterFiles(files, "  ")).toHaveLength(5);
  });
});
