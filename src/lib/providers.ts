/** One line per provider for the sidebar's Providers section. */

import type { ProviderStatus } from "../ipc/types";

export interface ProviderLine {
  key: "github" | "gitlab";
  label: string;
  /** The host it talks to, if any. */
  host: string;
  state: string;
  tone: "ok" | "warning" | "off";
  /** The connection is unencrypted (plain http off this machine). */
  insecure: boolean;
}

/** Shown wherever the GitLab URL is plain http off this machine. */
export const INSECURE_NOTE =
  "Plain http: your token travels unencrypted. Only use this on a trusted network, such as a VPN.";

/**
 * Shown when the saved GitLab URL is plain http off this machine and has no
 * token: a token stored for https is never sent over http, so plain http
 * asks for one of its own.
 */
export const HTTP_TOKEN_NOTE =
  "Plain http needs its own token; enter it to send it unencrypted.";

/**
 * Whether a typed GitLab URL is plain http to a host other than this
 * machine (mirrors the core's `is_insecure`; the core's status is the truth
 * once saved, this is for the URL while it is being typed).
 */
export function isInsecureUrl(url: string): boolean {
  try {
    const u = new URL(url.trim());
    if (u.protocol !== "http:") return false;
    return !["localhost", "127.0.0.1", "[::1]"].includes(
      u.hostname.toLowerCase(),
    );
  } catch {
    return false;
  }
}

export function providerLines(status: ProviderStatus): ProviderLine[] {
  const gh = status.github;
  const gl = status.gitlab;
  return [
    {
      key: "github",
      label: "GitHub",
      host: gh.host,
      insecure: false,
      ...(!gh.enabled
        ? { state: "Off", tone: "off" as const }
        : !gh.installed
          ? { state: "gh not found", tone: "warning" as const }
          : !gh.loggedIn
            ? { state: "Not logged in", tone: "warning" as const }
            : { state: "Connected", tone: "ok" as const }),
    },
    {
      key: "gitlab",
      label: "GitLab",
      host: gl.baseUrl.replace(/^https?:\/\//, ""),
      insecure: gl.configured && gl.problem === null && gl.insecure,
      ...(!gl.configured
        ? { state: "Not set up", tone: "off" as const }
        : gl.problem === "invalidUrl"
          ? { state: "Invalid URL", tone: "warning" as const }
          : !gl.tokenPresent
            ? { state: "No token", tone: "warning" as const }
            : { state: "Token set", tone: "ok" as const }),
    },
  ];
}
