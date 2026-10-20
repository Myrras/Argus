/**
 * Settings → AI → Review skill: the skills the picker offers per repository
 * (`list_review_skills`), read when the section shows and again when a
 * repository's picker opens (a skill may have been installed meanwhile).
 * The choice itself lives in `Settings.reviewSkills`
 * (`backendSettingsStore.save`).
 */

import { create } from "zustand";

import { listReviewSkills, toIpcError } from "../ipc/commands";
import type { ReviewSkillList } from "../ipc/types";

interface ReviewSkillsState {
  /** Per repository (`host/repo`): what the core listed last. */
  lists: Record<string, ReviewSkillList>;
  /** Per repository: a listing under way. */
  loading: Record<string, boolean>;
  /** Per repository: why the last listing failed. */
  errors: Record<string, string>;
  load: (repo: string) => Promise<void>;
}

export const useReviewSkillsStore = create<ReviewSkillsState>((set) => ({
  lists: {},
  loading: {},
  errors: {},
  load: async (repo) => {
    set((s) => ({ loading: { ...s.loading, [repo]: true } }));
    try {
      const list = await listReviewSkills(repo);
      set((s) => {
        const errors = { ...s.errors };
        delete errors[repo];
        return {
          // A malformed answer keeps what was listed before.
          lists:
            list && Array.isArray(list.skills)
              ? { ...s.lists, [repo]: list }
              : s.lists,
          errors,
        };
      });
    } catch (e) {
      set((s) => ({
        errors: { ...s.errors, [repo]: toIpcError(e).message },
      }));
    } finally {
      set((s) => ({ loading: { ...s.loading, [repo]: false } }));
    }
  },
}));
