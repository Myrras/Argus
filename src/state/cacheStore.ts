/**
 * The core's disk cache of file contents and file lists, for Settings →
 * General: what it holds (`cache_stats`) and Clear (`clear_cache`). Its
 * size limit is a setting (`Settings.cacheLimitMb`, `backendSettingsStore`).
 * Failures are error toasts.
 */

import { create } from "zustand";

import { cacheStats, clearCache, toIpcError } from "../ipc/commands";
import type { CacheStats } from "../ipc/types";
import { toastError } from "./toastStore";

/** The limits the stepper offers, in MB. */
export const CACHE_LIMITS_MB = [128, 256, 512, 1024, 2048, 4096, 8192];
/** The core's default (`cache::DEFAULT_LIMIT_MB`). */
export const DEFAULT_CACHE_LIMIT_MB = 512;

interface CacheState {
  /** `null` until read. */
  stats: CacheStats | null;
  clearing: boolean;
  load: () => Promise<void>;
  clear: () => Promise<void>;
}

export const useCacheStore = create<CacheState>((set) => ({
  stats: null,
  clearing: false,
  load: async () => {
    try {
      set({ stats: await cacheStats() });
    } catch (e) {
      toastError("Could not read the cache", toIpcError(e).message);
    }
  },
  clear: async () => {
    set({ clearing: true });
    try {
      set({ stats: await clearCache() });
    } catch (e) {
      toastError("Could not clear the cache", toIpcError(e).message);
    } finally {
      set({ clearing: false });
    }
  },
}));

/** `bytes` as "12.3 MB" / "1.2 GB". */
export function formatBytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  if (mb >= 10) return `${Math.round(mb)} MB`;
  return `${mb.toFixed(1)} MB`;
}

/** A limit in MB as the stepper shows it. */
export function formatLimit(mb: number): string {
  return mb >= 1024 ? `${mb / 1024} GB` : `${mb} MB`;
}
