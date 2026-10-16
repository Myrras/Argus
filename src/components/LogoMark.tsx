/**
 * Argus brand mark, drawn in `currentColor`: an almond eye — lids, iris ring
 * and pupil (Argus Panoptes was the hundred-eyed watchman who never slept).
 * Same 100×100 grid, opacity steps and soft shadow as the Podium, Selene and
 * Iris marks. The source of truth is `src-tauri/icons/source/argus-mark.svg`
 * (the app icon: `argus.svg`).
 */

import type { SVGProps } from "react";

interface LogoMarkProps extends SVGProps<SVGSVGElement> {
  size?: number;
}

/**
 * The mark's paths, outermost first (shared with tests). Each ring is an outer
 * outline plus a reversed inner one, so the nonzero fill leaves the hole.
 */
export const LOGO_PATHS = [
  "M4 50A47.39 47.39 0 0 1 96 50A47.39 47.39 0 0 1 4 50ZM15 50A36.56 36.56 0 0 0 85 50A36.56 36.56 0 0 0 15 50Z",
  "M28 50A22 22 0 0 1 72 50A22 22 0 0 1 28 50ZM35 50A15 15 0 0 0 65 50A15 15 0 0 0 35 50Z",
  "M39 50A11 11 0 0 1 61 50A11 11 0 0 1 39 50Z",
] as const;

const OPACITY = [0.45, 0.72, 1] as const;

export function LogoMark({ size = 18, ...props }: LogoMarkProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 100 100"
      width={size}
      height={size}
      {...props}
    >
      <defs>
        <filter id="lm-shadow" x="-40%" y="-20%" width="180%" height="140%">
          <feDropShadow
            dx="0"
            dy="2"
            stdDeviation="3"
            floodColor="currentColor"
            floodOpacity="0.18"
          />
        </filter>
      </defs>
      <g fill="currentColor" filter="url(#lm-shadow)">
        {LOGO_PATHS.map((d, i) => (
          <path key={d} d={d} opacity={OPACITY[i]} />
        ))}
      </g>
    </svg>
  );
}
