/**
 * The shortcut help (?, M6): every active shortcut with its current
 * bindings, by where it works (the keymap's scopes) — the same data as
 * Settings → Shortcuts (`activeActions`, `useKeymap`), read-only, with a way
 * there to change them.
 */

import {
  activeActions,
  formatBinding,
  type ActionDef,
  type ActionScope,
} from "../lib/keymap";
import { isMac } from "../lib/platform";
import { useKeymap } from "../lib/useKeymap";
import { useOverlayStore } from "../state/overlayStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { Modal } from "./Modal";
import styles from "./ShortcutHelp.module.css";

/** The scopes in reading order, with where their keys work. */
const SCOPES: { scope: ActionScope; title: string; help: string }[] = [
  {
    scope: "files",
    title: "In a review",
    help: "Outside text fields and dialogs, with a review open.",
  },
  {
    scope: "list",
    title: "Anywhere",
    help: "Outside text fields and dialogs.",
  },
  {
    scope: "composer",
    title: "In a comment",
    help: "While writing a comment, reply or finding edit.",
  },
  {
    scope: "find",
    title: "Find in review",
    help: "While the find bar is open; Enter and Shift+Enter in its field too.",
  },
  { scope: "app", title: "Everywhere", help: "Also in text fields." },
];

function Rows({ actions }: { actions: ActionDef[] }) {
  const keymap = useKeymap();
  return (
    <dl className={styles.rows}>
      {actions.map((a) => (
        <div key={a.id} className={styles.row}>
          <dt className={styles.label}>
            {a.label}
            {a.note && <small className={styles.note}>{a.note}</small>}
          </dt>
          <dd className={styles.keys}>
            {keymap[a.id].length === 0 ? (
              <span className={styles.note}>Not bound</span>
            ) : (
              keymap[a.id].map((b) => (
                <kbd key={b} className={styles.kbd}>
                  {formatBinding(b)}
                </kbd>
              ))
            )}
          </dd>
        </div>
      ))}
    </dl>
  );
}

export function ShortcutHelp() {
  const open = useOverlayStore((s) => s.open === "help");
  const close = useOverlayStore((s) => s.close);
  const actions = activeActions();
  const change = () => {
    close();
    useSettingsRequestStore.getState().open("shortcuts");
  };
  return (
    <Modal
      open={open}
      title="Keyboard shortcuts"
      onClose={close}
      width="min(680px, 94vw)"
      footer={
        <>
          <button type="button" onClick={change}>
            Change shortcuts…
          </button>
          <button type="button" className="primary" onClick={close}>
            Close
          </button>
        </>
      }
    >
      <div className={styles.help}>
        {SCOPES.map(({ scope, title, help }) => {
          const rows = actions.filter((a) => a.scope === scope);
          if (rows.length === 0 && !(scope === "app" && isMac)) return null;
          return (
            <section key={scope} className={styles.section} aria-label={title}>
              <h3 className={styles.title}>
                {title}
                <small className={styles.note}>{help}</small>
              </h3>
              <Rows actions={rows} />
              {scope === "app" && isMac && (
                <dl className={styles.rows}>
                  <div className={styles.row}>
                    <dt className={styles.label}>Open Settings</dt>
                    <dd className={styles.keys}>
                      <kbd className={styles.kbd}>⌘,</kbd>
                    </dd>
                  </div>
                </dl>
              )}
            </section>
          );
        })}
      </div>
    </Modal>
  );
}
