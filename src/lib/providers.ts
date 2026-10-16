/** One line per provider for the sidebar's Providers section. */

import type { ProviderStatus } from "../ipc/types";

export interface ProviderLine {
  key: "github" | "gitlab";
  label: string;
  /** The host it talks to, if any. */
  host: string;
  state: string;
  tone: "ok" | "warning" | "off";
}

export function providerLines(status: ProviderStatus): ProviderLine[] {
  const gh = status.github;
  const gl = status.gitlab;
  return [
    {
      key: "github",
      label: "GitHub",
      host: gh.host,
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
      ...(!gl.configured
        ? { state: "Not set up", tone: "off" as const }
        : !gl.tokenPresent
          ? { state: "No token", tone: "warning" as const }
          : { state: "Token set", tone: "ok" as const }),
    },
  ];
}
