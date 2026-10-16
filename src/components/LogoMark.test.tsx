import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import appIcon from "../../src-tauri/icons/source/argus.svg?raw";
import source from "../../src-tauri/icons/source/argus-mark.svg?raw";
import { LOGO_PATHS, LogoMark } from "./LogoMark";

describe("LogoMark", () => {
  it("draws lids, iris and pupil in currentColor at the given size", () => {
    const { container } = render(<LogoMark size={48} aria-hidden />);
    const svg = container.querySelector("svg")!;
    expect(svg).toHaveAttribute("width", "48");
    expect(svg).toHaveAttribute("height", "48");
    expect(svg).toHaveAttribute("viewBox", "0 0 100 100");
    expect(svg.querySelector("g")).toHaveAttribute("fill", "currentColor");
    const paths = [...svg.querySelectorAll("path")];
    expect(paths.map((p) => p.getAttribute("d"))).toEqual([...LOGO_PATHS]);
    expect(paths.map((p) => p.getAttribute("opacity"))).toEqual([
      "0.45",
      "0.72",
      "1",
    ]);
  });

  it("matches the source SVGs the app icon is built from", () => {
    for (const d of LOGO_PATHS) {
      expect(source).toContain(`d="${d}"`);
      expect(appIcon).toContain(`d="${d}"`);
    }
  });
});
