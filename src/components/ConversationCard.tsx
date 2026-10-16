/**
 * The review view's first section: the threads without a code reference
 * (comments and discussions on the MR/PR as a whole), the user's first and
 * everyone else's folded, in the same thread style as the inline threads.
 * Under "On other files", anchored threads whose file the file list does not
 * show (so no thread is ever out of reach).
 */

import { useState } from "react";

import type { IpcError, ProviderId, StoredThread } from "../ipc/types";
import { CONVERSATION_CARD } from "../lib/focusModel";
import { useRevealCard } from "../state/focusStore";
import { threadCounts } from "../lib/diffThreads";
import { ThreadCount } from "./FileStatusMark";
import { CaretIcon, ThreadsIcon } from "./icons";
import { ListedThread, ThreadBlock } from "./ThreadBlock";
import styles from "./ConversationCard.module.css";

export function ConversationCard({
  conversation,
  elsewhere,
  provider,
  loading,
  error,
  selected,
  now,
  onSelect,
  cardRef,
}: {
  conversation: readonly StoredThread[];
  elsewhere: readonly StoredThread[];
  provider: ProviderId;
  /** The threads are loading for the first time. */
  loading: boolean;
  error: IpcError | null;
  selected: boolean;
  now: number;
  onSelect: () => void;
  cardRef: (el: HTMLElement | null) => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  // A keyboard move to one of its threads opens it (M6).
  useRevealCard(CONVERSATION_CARD, () => setCollapsed(false));
  const count = threadCounts([...conversation, ...elsewhere]);
  const empty = conversation.length === 0 && elsewhere.length === 0;

  return (
    <section
      ref={cardRef}
      className={styles.card}
      data-selected={selected}
      data-file-card="conversation"
      aria-label="Conversation"
    >
      <header className={styles.header}>
        <button
          type="button"
          className={`ghost ${styles.collapse}`}
          aria-expanded={!collapsed}
          aria-label={
            collapsed ? "Expand the conversation" : "Collapse the conversation"
          }
          onClick={() => {
            setCollapsed((c) => !c);
            onSelect();
          }}
        >
          <CaretIcon
            size={12}
            className={styles.caret}
            data-open={!collapsed}
          />
        </button>
        <ThreadsIcon className={styles.icon} />
        <button
          type="button"
          className={`ghost ${styles.title}`}
          data-focus-target
          onClick={onSelect}
        >
          Conversation
        </button>
        <ThreadCount {...count} />
      </header>
      {!collapsed && (
        <div className={styles.body}>
          {error && (
            <p className={styles.error} role="alert">
              {error.message}
            </p>
          )}
          {loading ? (
            <p className={styles.placeholder}>Loading threads…</p>
          ) : empty ? (
            <p className={styles.placeholder}>
              No comments on the review as a whole.
            </p>
          ) : (
            <>
              {conversation.length > 0 && (
                <div className={styles.threads}>
                  {conversation.map((t) => (
                    <ThreadBlock
                      key={t.threadId}
                      thread={t}
                      provider={provider}
                      now={now}
                    />
                  ))}
                </div>
              )}
              {elsewhere.length > 0 && (
                <div className={styles.elsewhere}>
                  <h3 className={styles.subTitle}>
                    On other files{" "}
                    <span className={styles.subCount}>{elsewhere.length}</span>
                  </h3>
                  {elsewhere.map((t) => (
                    <ListedThread
                      key={t.threadId}
                      thread={t}
                      provider={provider}
                      now={now}
                    />
                  ))}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
