/** Compact relative times for a dense list ("5m", "3h", "2d", "4w"). */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

/** `iso` relative to `now` (ms), compact; "now" under a minute. */
export function age(iso: string | null, now: number = Date.now()): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const d = Math.max(0, now - t);
  if (d < MINUTE) return "now";
  if (d < HOUR) return `${Math.floor(d / MINUTE)}m`;
  if (d < DAY) return `${Math.floor(d / HOUR)}h`;
  if (d < WEEK) return `${Math.floor(d / DAY)}d`;
  if (d < 52 * WEEK) return `${Math.floor(d / WEEK)}w`;
  return `${Math.floor(d / (365 * DAY))}y`;
}

/** "just now" / "5m ago", for the last-refreshed label. */
export function ago(iso: string | null, now: number = Date.now()): string {
  const a = age(iso, now);
  if (!a) return "never";
  return a === "now" ? "just now" : `${a} ago`;
}

/** Full local date and time, for tooltips. */
export function fullTime(iso: string | null): string {
  if (!iso) return "";
  const t = new Date(iso);
  return Number.isNaN(t.getTime()) ? "" : t.toLocaleString();
}
