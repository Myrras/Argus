/**
 * The review view's first section: the threads without a code reference
 * (comments and discussions on the MR/PR as a whole), the user's first and
 * everyone else's folded, in the same thread style as the inline threads.
 * Under "On other files", anchored threads whose file the file list does not
 * show (so no thread is ever out of reach).
 *
 * Both parts split the way GitLab and GitHub do (`diffThreads.groupThreads`):
 * Unresolved, then Comments (entries nothing can resolve: a GitHub PR
 * conversation entry, a GitLab single comment), then Resolved, folded until
 * clicked; each group keeps the mine-first order. The fold is this card's
 * own state, so it resets per review. A keyboard move onto a thread in a
 * folded Resolved group opens it (`useRevealCard`), so `n`/`p` never land
 * on a thread that is not shown.
 */

import { useId, useState } from "react";

import type { IpcError, ProviderId, StoredThread } from "../ipc/types";
import { CONVERSATION_CARD } from "../lib/focusModel";
import { useFocusStore, useRevealCard } from "../state/focusStore";
import { groupThreads, unresolvedCount } from "../lib/diffThreads";
import { OpenCount } from "./FileStatusMark";
import { CaretIcon, OpenThreadIcon, ResolvedIcon, ThreadsIcon } from "./icons";
import { ListedThread, ThreadBlock } from "./ThreadBlock";
import styles from "./ConversationCard.module.css";

type Part = "conversation" | "elsewhere";

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
  const [resolvedOpen, setResolvedOpen] = useState<Record<Part, boolean>>({
    conversation: false,
    elsewhere: false,
  });
  const showResolved = (part: Part, open: boolean) =>
    setResolvedOpen((s) => ({ ...s, [part]: open }));
  // A keyboard move to one of its threads opens it (M6), and the Resolved
  // group that holds it.
  useRevealCard(CONVERSATION_CARD, () => {
    setCollapsed(false);
    const item = useFocusStore.getState().item;
    if (item?.kind !== "thread") return;
    const held = (list: readonly StoredThread[]) =>
      list.some((t) => t.threadId === item.threadId && t.resolved);
    if (held(conversation)) showResolved("conversation", true);
    if (held(elsewhere)) showResolved("elsewhere", true);
  });
  const all = [...conversation, ...elsewhere];
  const open = unresolvedCount(all);
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
        <OpenCount count={open} />
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
                <div
                  className={styles.threads}
                  data-testid="conversation-threads"
                >
                  <ThreadGroups
                    threads={conversation}
                    resolvedOpen={resolvedOpen.conversation}
                    onResolvedOpen={(o) => showResolved("conversation", o)}
                    render={(t) => (
                      <ThreadBlock
                        key={t.threadId}
                        thread={t}
                        provider={provider}
                        now={now}
                      />
                    )}
                  />
                </div>
              )}
              {elsewhere.length > 0 && (
                <div
                  className={styles.elsewhere}
                  data-testid="elsewhere-threads"
                >
                  <h3 className={styles.subTitle}>
                    On other files{" "}
                    <span className={styles.subCount}>{elsewhere.length}</span>
                  </h3>
                  <ThreadGroups
                    threads={elsewhere}
                    resolvedOpen={resolvedOpen.elsewhere}
                    onResolvedOpen={(o) => showResolved("elsewhere", o)}
                    render={(t) => (
                      <ListedThread
                        key={t.threadId}
                        thread={t}
                        provider={provider}
                        now={now}
                      />
                    )}
                  />
                </div>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

/**
 * One part's threads as Unresolved, Comments and Resolved, each with its
 * header and count; empty groups are left out. Resolved folds.
 */
function ThreadGroups({
  threads,
  resolvedOpen,
  onResolvedOpen,
  render,
}: {
  threads: readonly StoredThread[];
  resolvedOpen: boolean;
  onResolvedOpen: (open: boolean) => void;
  render: (t: StoredThread) => React.ReactNode;
}) {
  const g = groupThreads(threads);
  return (
    <>
      {g.unresolved.length > 0 && (
        <Group kind="unresolved" label="Unresolved" count={g.unresolved.length}>
          {g.unresolved.map(render)}
        </Group>
      )}
      {g.comments.length > 0 && (
        <Group kind="comments" label="Comments" count={g.comments.length}>
          {g.comments.map(render)}
        </Group>
      )}
      {g.resolved.length > 0 && (
        <Group
          kind="resolved"
          label="Resolved"
          count={g.resolved.length}
          fold={{ open: resolvedOpen, onToggle: onResolvedOpen }}
        >
          {resolvedOpen && g.resolved.map(render)}
        </Group>
      )}
    </>
  );
}

const GROUP_ICON = {
  unresolved: OpenThreadIcon,
  comments: ThreadsIcon,
  resolved: ResolvedIcon,
} as const;

function Group({
  kind,
  label,
  count,
  fold,
  children,
}: {
  kind: keyof typeof GROUP_ICON;
  label: string;
  count: number;
  /** A foldable group: its header is the toggle. */
  fold?: { open: boolean; onToggle: (open: boolean) => void };
  children: React.ReactNode;
}) {
  const id = useId();
  const Icon = GROUP_ICON[kind];
  const countLabel = `${count} ${count === 1 ? "thread" : "threads"}`;
  const head = (
    <>
      <Icon size={13} className={styles.groupIcon} />
      <span id={id} className={styles.groupLabel}>
        {label}
      </span>
      <span className={styles.groupCount} aria-label={countLabel}>
        {count}
      </span>
      {/* Trailing, so the group icons line up down the card. */}
      {fold && (
        <CaretIcon size={12} className={styles.caret} data-open={fold.open} />
      )}
    </>
  );
  return (
    <div
      role="group"
      aria-labelledby={id}
      className={styles.group}
      data-group={kind}
    >
      {fold ? (
        <button
          type="button"
          className={`ghost ${styles.groupHead}`}
          aria-expanded={fold.open}
          title={
            fold.open
              ? "Fold the resolved threads"
              : "Show the resolved threads"
          }
          onClick={() => fold.onToggle(!fold.open)}
        >
          {head}
        </button>
      ) : (
        <div className={styles.groupHead}>{head}</div>
      )}
      {children}
    </div>
  );
}
