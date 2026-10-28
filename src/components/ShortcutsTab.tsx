/**
 * Settings → Shortcuts (Iris's `ShortcutsTab`): every action of `lib/keymap`
 * with its binding, in Podium's card look. "Record" waits for a key
 * combination and rebinds the action to it; a fixed combination is refused,
 * one another action already uses is refused too with an offer to swap the
 * two. "Reset" puts a row's default back, "Reset all" every row.
 * Only changed rows are stored (`settingsStore.shortcuts`). A fixed action
 * (Escape cancels a comment) is listed without Record/Reset.
 */

import { useEffect, useState } from "react";

import {
  actionDef,
  activeActions,
  bindingFromEvent,
  bindingProblem,
  findConflict,
  formatBinding,
  GROUPS,
  serializeBinding,
  type ActionId,
} from "../lib/keymap";
import { isMac } from "../lib/platform";
import { useKeymap } from "../lib/useKeymap";
import { useSettingsStore } from "../state/settingsStore";
import styles from "./SettingsModal.module.css";

/** A refused recording: why, and what swapping would do (when it can). */
interface Refusal {
  action: ActionId;
  message: string;
  swap?: { binding: string; with: ActionId };
}

function sameList(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

export function ShortcutsTab() {
  const keymap = useKeymap();
  const overrides = useSettingsStore((s) => s.shortcuts);
  const setShortcut = useSettingsStore((s) => s.setShortcut);
  const resetShortcuts = useSettingsStore((s) => s.resetShortcuts);
  const [recording, setRecording] = useState<ActionId | null>(null);
  const [refusal, setRefusal] = useState<Refusal | null>(null);

  // A list equal to the defaults is not stored, so "Reset" greys out.
  const assign = (action: ActionId, list: string[]) =>
    setShortcut(
      action,
      sameList(list, actionDef(action).defaults) ? null : list,
    );

  useEffect(() => {
    if (!recording) return;
    const action = recording;
    // Capture phase: nothing else (the dialog's Escape, the pane shortcuts)
    // sees the keys while one is being recorded.
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      if (e.key === "Escape") {
        setRecording(null);
        setRefusal(null);
        return;
      }
      const binding = bindingFromEvent(e);
      if (!binding) return; // a modifier on its own: wait for the key
      const shown = formatBinding(binding);
      const problem = bindingProblem(binding);
      if (problem) {
        setRefusal({ action, message: problem });
        return;
      }
      const text = serializeBinding(binding);
      const other = findConflict(keymap, action, binding);
      if (other) {
        setRecording(null);
        setRefusal({
          action,
          message: `${shown} is already used by ${actionDef(other).label}.`,
          swap: { binding: text, with: other },
        });
        return;
      }
      assign(action, [text]);
      setRecording(null);
      setRefusal(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // `assign` only wraps the store action.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording, keymap]);

  const swap = () => {
    if (!refusal?.swap) return;
    const { action } = refusal;
    const { binding, with: other } = refusal.swap;
    const mine = keymap[action];
    assign(action, [binding]);
    assign(other, mine);
    setRefusal(null);
  };

  const actions = activeActions();
  const changed = Object.keys(overrides).length > 0;

  return (
    <>
      <section className={styles.section}>
        <div className={styles.row}>
          <span className={styles.rowLabel}>
            Keyboard shortcuts
            <small className={styles.rowHelp}>
              Click Record, then press the new key or combination. The list keys
              work outside text fields and dialogs.
            </small>
          </span>
          <button type="button" disabled={!changed} onClick={resetShortcuts}>
            Reset all
          </button>
        </div>
      </section>
      {GROUPS.map((group) => {
        const rows = actions.filter((a) => a.group === group);
        if (rows.length === 0) return null;
        return (
          <section key={group} className={styles.section}>
            <h3 className={styles.sectionLabel}>{group}</h3>
            {rows.map((a) => {
              const isRecording = recording === a.id;
              const isDefault = sameList(keymap[a.id], a.defaults);
              const refused = refusal?.action === a.id ? refusal : null;
              return (
                <div
                  key={a.id}
                  className={styles.card}
                  role="group"
                  aria-label={a.label}
                >
                  <div className={styles.cardHeader}>
                    <span className={styles.shortcutName}>
                      {a.label}
                      {a.note && (
                        <small className={styles.rowHelp}>{a.note}</small>
                      )}
                    </span>
                    <span
                      className={`${styles.keys} ${styles.pushRight}`}
                      data-recording={isRecording ? "true" : undefined}
                    >
                      {isRecording ? (
                        <span className={styles.recordingHint}>
                          Press keys…
                        </span>
                      ) : keymap[a.id].length === 0 ? (
                        <span className={styles.rowHelp}>Not bound</span>
                      ) : (
                        keymap[a.id].map((b) => (
                          <kbd key={b} className={styles.kbd}>
                            {formatBinding(b, isMac)}
                          </kbd>
                        ))
                      )}
                    </span>
                    {a.fixed ? (
                      <span className={styles.rowHelp}>Fixed</span>
                    ) : (
                      <div className={styles.cardActions}>
                        <button
                          type="button"
                          aria-label={`${isRecording ? "Cancel recording" : "Record shortcut"} for ${a.label}`}
                          onClick={() => {
                            setRefusal(null);
                            setRecording(isRecording ? null : a.id);
                          }}
                        >
                          {isRecording ? "Cancel" : "Record"}
                        </button>
                        <button
                          type="button"
                          aria-label={`Reset ${a.label}`}
                          disabled={isDefault}
                          onClick={() => {
                            setShortcut(a.id, null);
                            if (isRecording) setRecording(null);
                            setRefusal(null);
                          }}
                        >
                          Reset
                        </button>
                      </div>
                    )}
                  </div>
                  {refused && (
                    <p className={styles.error} role="alert">
                      {refused.message}
                      {refused.swap && (
                        <>
                          {" "}
                          <button
                            type="button"
                            className={styles.inlineButton}
                            onClick={swap}
                          >
                            Swap them
                          </button>
                        </>
                      )}
                    </p>
                  )}
                </div>
              );
            })}
          </section>
        );
      })}
    </>
  );
}
