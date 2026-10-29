/**
 * The review language picker: the language the AI pass writes its
 * findings, checklist items and verdict reasons in. A select (Default
 * (<global>) where a default exists, Auto, English, Nederlands, Other…)
 * that applies at once; Other… shows a name field with its own Save, per
 * the settings' save model. Used per repository in Settings → Repositories
 * (with the default) and for the global default in Settings → AI (without).
 */

import { useEffect, useState } from "react";

import type { ReviewLanguage } from "../ipc/types";
import {
  MAX_LANGUAGE_NAME,
  languageLabel,
  languageOfOption,
  optionOf,
  otherLanguage,
  sameLanguage,
  type LanguageOption,
} from "../lib/repositories";
import own from "./ReviewLanguageField.module.css";
import styles from "./SettingsModal.module.css";

export function ReviewLanguageField({
  id,
  label,
  value,
  fallback,
  disabled,
  onChange,
}: {
  /** The select's id (the name field is `${id}-name`). */
  id: string;
  /** The select's accessible name. */
  label: string;
  /** The stored choice; `null` is the default (only with a `fallback`). */
  value: ReviewLanguage | null;
  /** The default it follows, which adds the "Default (…)" option. */
  fallback?: ReviewLanguage;
  disabled?: boolean;
  onChange: (next: ReviewLanguage | null) => void;
}) {
  const stored = optionOf(value);
  // "Other…" picked but not saved yet: the name field shows, the stored
  // choice is unchanged until Save.
  const [option, setOption] = useState<LanguageOption>(stored);
  const storedName = value?.kind === "other" ? value.name : "";
  const [name, setName] = useState(storedName);
  useEffect(() => {
    setOption(stored);
    setName(storedName);
  }, [stored, storedName]);

  const pick = (next: LanguageOption) => {
    setOption(next);
    const language = languageOfOption(next);
    if (language === undefined) return;
    if (!sameLanguage(language, value)) onChange(language);
  };
  const typed = otherLanguage(name);
  const dirty = typed !== null && !sameLanguage(typed, value);

  return (
    <div className={own.field}>
      <select
        id={id}
        aria-label={label}
        value={option}
        disabled={disabled}
        onChange={(e) => pick(e.target.value as LanguageOption)}
      >
        {fallback && (
          <option value="default">Default ({languageLabel(fallback)})</option>
        )}
        <option value="auto">Auto (the merge request&apos;s language)</option>
        <option value="english">English</option>
        <option value="dutch">Nederlands</option>
        <option value="other">Other…</option>
      </select>
      {option === "other" && (
        <form
          className={styles.fieldRow}
          aria-label={`${label}: other language`}
          onSubmit={(e) => {
            e.preventDefault();
            if (typed && dirty) onChange(typed);
          }}
        >
          <input
            id={`${id}-name`}
            type="text"
            value={name}
            maxLength={MAX_LANGUAGE_NAME}
            placeholder="Deutsch"
            aria-label={`${label}: language name`}
            autoComplete="off"
            spellCheck={false}
            disabled={disabled}
            onChange={(e) => setName(e.target.value)}
          />
          <button type="submit" className="primary" disabled={!dirty}>
            Save
          </button>
        </form>
      )}
    </div>
  );
}
