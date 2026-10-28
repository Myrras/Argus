/** Small inline pills: platform, round reasons, thread states. */

import type { ReactNode } from "react";

import type { ProviderId } from "../ipc/types";
import { PROVIDER_LABEL } from "../lib/reviews";
import styles from "./Badge.module.css";

export type BadgeTone =
  "neutral" | "accent" | "warning" | "success" | "danger" | "github" | "gitlab";

export function Badge({
  tone = "neutral",
  title,
  children,
}: {
  tone?: BadgeTone;
  title?: string;
  children: ReactNode;
}) {
  return (
    <span className={styles.badge} data-tone={tone} title={title}>
      {children}
    </span>
  );
}

export function PlatformBadge({ provider }: { provider: ProviderId }) {
  return <Badge tone={provider}>{PROVIDER_LABEL[provider]}</Badge>;
}
