import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { addressed, defaultTarget, waitingQuestion, writable } from "../lib/field";
import { api } from "../lib/ipc";
import { FIELD_KEY, FIND_KEY } from "../lib/keybinds";
import { useStore } from "../lib/store";
import { type AgentCard, type AgentId, errorMessage } from "../lib/types";
import { Icon } from "./Icon";

interface Props {
  /** Opens search with what was typed, for the row offered under the field. */
  onSearch: (query: string) => void;
  /** The Groups sheet, which the picker's last row opens. */
  onManageGroups: () => void;
}

/** What the last send did, said once under the field. */
type Receipt = { kind: "sent"; agent: AgentCard } | { kind: "answered"; agent: AgentCard };

/**
 * The first thing in the column: one line that writes to an agent, or answers
 * the question that is waiting.
 *
 * A field rather than the channel's composer. It sends one message and stays
 * where it is, which is the whole of the most common errand ("tell someone,
 * then leave"); a draft, a file or a thread is a conversation, and the
 * conversation is one click away with the composer it needs. The picker names
 * every group, so the group an agent belongs to is said where the agent is
 * chosen.
 */
export function Field({ onSearch, onManageGroups }: Props) {
  const agents = useStore((state) => state.agents);
  const groups = useStore((state) => state.groups);
  const pending = useStore((state) => state.pending);
  const lastActive = useStore((state) => state.lastActive);
  const railGroup = useStore((state) => state.railGroup);
  const answer = useStore((state) => state.answerQuestion);
  const select = useStore((state) => state.select);
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<AgentId | null>(null);
  const [answering, setAnswering] = useState(false);
  const [picking, setPicking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);

  const live = writable(agents);
  const question = waitingQuestion(pending);
  const asker = question && live.find((agent) => agent.id === question.agentId);
  const target =
    live.find((agent) => agent.id === picked) ?? defaultTarget(agents, lastActive, railGroup);
  const asking = answering && question && asker;

  // A question answered elsewhere takes the field out of answering it, rather
  // than leaving a sentence pointed at a request that is gone.
  useEffect(() => {
    if (!question) setAnswering(false);
  }, [question]);

  const send = async () => {
    const body = text.trim();
    if (!body || busy) return;
    setBusy(true);
    setFailure(null);
    try {
      if (asking) {
        await answer(question.id, body);
        setReceipt({ kind: "answered", agent: asker });
        setAnswering(false);
      } else {
        const to = addressed(body, agents) ?? (target && { agent: target, text: body });
        if (!to) return;
        await api.sendMessage(to.agent.id, to.text);
        setReceipt({ kind: "sent", agent: to.agent });
      }
      setText("");
    } catch (error) {
      setFailure(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
      return;
    }
    // Tab answers the waiting question, once, from an empty field. With text
    // in it Tab is the browser's, and leaving the field is what it should do.
    if (event.key === "Tab" && !event.shiftKey && !answering && question && asker && !text) {
      event.preventDefault();
      setAnswering(true);
      return;
    }
    if (event.key === "Escape" && answering) {
      event.preventDefault();
      setAnswering(false);
    }
  };

  const placeholder = asking ? `Answer ${asker.name}: ${question.summary}` : "What do you need?";

  return (
    <div className="ask">
      <form
        className="ask__box"
        data-answering={asking ? "" : undefined}
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
        {asking ? (
          <span className="ask__target" data-answering="">
            <AgentAvatar avatar={asker.avatar} color={asker.color} size="xs" seed={asker.id} />
            Answer
          </span>
        ) : (
          <TargetPicker
            target={target}
            open={picking}
            onOpen={setPicking}
            agents={live}
            groups={groups}
            onPick={(id) => {
              setPicked(id);
              setPicking(false);
              input.current?.focus();
            }}
            onManageGroups={() => {
              setPicking(false);
              onManageGroups();
            }}
          />
        )}
        <textarea
          ref={input}
          className="ask__input"
          data-field=""
          rows={1}
          value={text}
          placeholder={placeholder}
          aria-label={asking ? `Answer ${asker.name}` : "What do you need?"}
          disabled={busy}
          onChange={(event) => {
            setText(event.target.value);
            setReceipt(null);
          }}
          onKeyDown={onKeyDown}
        />
        <kbd className="ask__key">{FIELD_KEY}</kbd>
        <button
          type="submit"
          className="ask__send"
          aria-label={asking ? "Send answer" : "Send"}
          disabled={busy || text.trim() === ""}
        >
          <Icon name="arrow-up" />
        </button>
      </form>
      <div className="ask__under">
        {question && asker && !answering && (
          <span className="ask__hint">
            <kbd>Tab</kbd> answer {asker.name}
          </span>
        )}
        {asking && (
          <span className="ask__hint">
            <kbd>Esc</kbd> write to someone instead
          </span>
        )}
        {!asking && (
          <span className="ask__hint">
            <kbd>@</kbd> someone
          </span>
        )}
        {text.trim() !== "" && !asking && (
          <button type="button" className="ask__search" onClick={() => onSearch(text.trim())}>
            <kbd>{FIND_KEY}</kbd> Search for “{text.trim()}”
          </button>
        )}
        {receipt && (
          <span className="ask__receipt" role="status">
            {receipt.kind === "sent"
              ? `Sent to ${receipt.agent.name}.`
              : `Answered ${receipt.agent.name}.`}{" "}
            <button
              type="button"
              className="ask__link"
              onClick={() => void select(receipt.agent.id)}
            >
              Open the conversation
            </button>
          </span>
        )}
        {failure && (
          <span className="ask__failure" role="alert">
            {failure}
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * Who the field writes to, and the one place group names are read now that the
 * top of the window is a timeline: each group is a heading over its agents.
 */
function TargetPicker({
  target,
  open,
  onOpen,
  agents,
  groups,
  onPick,
  onManageGroups,
}: {
  target: AgentCard | undefined;
  open: boolean;
  onOpen: (open: boolean) => void;
  agents: AgentCard[];
  groups: { id: string; name: string }[];
  onPick: (id: AgentId) => void;
  onManageGroups: () => void;
}) {
  const shelves = groups
    .map((group) => ({ group, members: agents.filter((agent) => agent.groupId === group.id) }))
    .filter((shelf) => shelf.members.length > 0);
  const named = shelves.length > 1;
  return (
    <div className="ask__picker">
      <button
        type="button"
        className="ask__target"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={target ? `Writing to ${target.name}. Change who` : "Choose who to write to"}
        onClick={() => onOpen(!open)}
      >
        {target && (
          <AgentAvatar avatar={target.avatar} color={target.color} size="xs" seed={target.id} />
        )}
        <span>{target?.name ?? "Nobody yet"}</span>
        <Icon name="chevron-right" />
      </button>
      {open && (
        <div
          className="ask__menu"
          role="listbox"
          aria-label="Write to"
          onKeyDown={(event) => {
            if (event.key === "Escape") onOpen(false);
          }}
        >
          {shelves.map(({ group, members }) => (
            <div key={group.id}>
              {named && <p className="ask__group">{group.name}</p>}
              {members.map((agent) => (
                <button
                  key={agent.id}
                  type="button"
                  role="option"
                  aria-selected={agent.id === target?.id}
                  className="ask__option"
                  onClick={() => onPick(agent.id)}
                >
                  <AgentAvatar
                    avatar={agent.avatar}
                    color={agent.color}
                    size="xs"
                    seed={agent.id}
                  />
                  {agent.name}
                </button>
              ))}
            </div>
          ))}
          <button type="button" className="ask__option ask__manage" onClick={onManageGroups}>
            Groups and agents
          </button>
        </div>
      )}
    </div>
  );
}
