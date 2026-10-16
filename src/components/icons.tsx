/**
 * Central icon set for Argus (same wrapper and defaults as Iris and Podium).
 *
 * The rest of the app imports semantic names from here and never references
 * `lucide-react` directly, so re-theming a glyph is a one-line change in this
 * file.
 *
 * Defaults: 14px at stroke-width 1.75 reads crisp and light at our small UI
 * sizes (lucide's own defaults are 24 / 2, too heavy here). Icons are purely
 * decorative — the buttons and rows that hold them already carry the
 * `aria-label`/`title` — so they default to `aria-hidden`. Every prop stays
 * overridable per use (size, strokeWidth, color, className…); colour follows
 * `currentColor`, so icons inherit the active theme with no per-theme work.
 */

import {
  Check,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  Copy,
  ExternalLink,
  FolderTree,
  GitPullRequest,
  Info,
  MessageSquare,
  Minus,
  PanelLeftClose,
  PanelLeftOpen,
  Plug,
  Plus,
  RotateCw,
  Settings,
  Square,
  X,
  type LucideIcon,
  type LucideProps,
} from "lucide-react";

/** Wrap a lucide glyph with our shared defaults; `defaults` then `props` win. */
function icon(Glyph: LucideIcon, defaults?: Partial<LucideProps>) {
  function Icon(props: LucideProps) {
    return (
      <Glyph
        size={14}
        strokeWidth={1.75}
        aria-hidden={true}
        {...defaults}
        {...props}
      />
    );
  }
  Icon.displayName = `Icon(${Glyph.displayName ?? "lucide"})`;
  return Icon;
}

// Entities: reviews, their threads and changed files, and the providers.
export const ReviewsIcon = icon(GitPullRequest);
export const ThreadsIcon = icon(MessageSquare);
export const FileTreeIcon = icon(FolderTree);
export const ProvidersIcon = icon(Plug);

// Actions / affordances.
export const AddIcon = icon(Plus, { size: 15 });
export const MinusIcon = icon(Minus, { size: 15 });
export const CloseIcon = icon(X);
export const CheckIcon = icon(Check);
export const RefreshIcon = icon(RotateCw);
export const ExternalLinkIcon = icon(ExternalLink, { size: 13 });
/** Collapse / expand the review view's file tree. */
export const PanelCloseIcon = icon(PanelLeftClose);
export const PanelOpenIcon = icon(PanelLeftOpen);

// The caret rotates 90° on expand (CSS).
export const CaretIcon = icon(ChevronRight);

// App chrome (Windows-only title bar). Window controls use a single line
// (minimize), a square (maximize), two stacked squares (restore), and the
// shared X (close) to match the platform convention.
export const SettingsIcon = icon(Settings);
export const WindowMinimizeIcon = icon(Minus, { size: 16 });
export const WindowMaximizeIcon = icon(Square, { size: 12 });
export const WindowRestoreIcon = icon(Copy, { size: 12 });

// Toast severity.
export const ErrorIcon = icon(CircleAlert, { size: 16 });
export const SuccessIcon = icon(CircleCheck, { size: 16 });
export const InfoIcon = icon(Info, { size: 16 });
