import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
import { addressed, defaultTarget, namedAlone, waitingQuestion, writable } from "../lib/field";
import { api } from "../lib/ipc";
import { bindingFor, FIELD_KEY, FIND_KEY } from "../lib/keybinds";
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

  // Grows with what is written, to the cap the stylesheet sets, the way the
  // composer does: a one-line box scrolled a long request out of sight.
  useEffect(() => {
    const node = input.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
  }, [text]);

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
        // A name and nothing else is a change of recipient, not a message:
        // sent as written it would hand somebody the literal "@Name".
        const named = namedAlone(body, agents);
        if (named) {
          setPicked(named.id);
          setText("");
          return;
        }
        const to = addressed(body, agents) ?? (target && { agent: target, text: body });
        if (!to) {
          setFailure("There is nobody to write to yet. Add an agent first.");
          return;
        }
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
      return;
    }
    // Search with what is written, as the row under the field offers. Stopped
    // here so the window's own shortcut does not open search empty.
    if (bindingFor(event.nativeEvent)?.id === "search" && text.trim() !== "") {
      event.preventDefault();
      event.stopPropagation();
      onSearch(text.trim());
    }
  };

  const placeholder = asking ? `Answer ${asker.name}: ${question.summary}` : "What do you need?";

  return (
    <div className="intake">
      <form
        className="intake__box"
        data-answering={asking ? "" : undefined}
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
        {asking ? (
          <span className="intake__target" data-answering="">
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
              // After the picker has closed and handed focus back to the
              // button that opened it: the next thing typed is the message.
              requestAnimationFrame(() => input.current?.focus());
            }}
            onManageGroups={() => {
              setPicking(false);
              onManageGroups();
            }}
          />
        )}
        <textarea
          ref={input}
          className="intake__input"
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
        <kbd className="intake__key">{FIELD_KEY}</kbd>
        <button
          type="submit"
          className="intake__send"
          aria-label={asking ? "Send answer" : "Send"}
          disabled={busy || text.trim() === ""}
        >
          <Icon name="arrow-up" />
        </button>
      </form>
      <div className="intake__under">
        {question && asker && !answering && (
          <span className="intake__hint">
            <kbd>Tab</kbd> answer {asker.name}
          </span>
        )}
        {asking && (
          <span className="intake__hint">
            <kbd>Esc</kbd> write to someone instead
          </span>
        )}
        {!asking && (
          <span className="intake__hint">
            <kbd>@</kbd> someone
          </span>
        )}
        {text.trim() !== "" && !asking && (
          <button type="button" className="intake__search" onClick={() => onSearch(text.trim())}>
            <kbd>{FIND_KEY}</kbd> Search for “{text.trim()}”
          </button>
        )}
        {receipt && (
          <span className="intake__receipt" role="status">
            {receipt.kind === "sent"
              ? `Sent to ${receipt.agent.name}.`
              : `Answered ${receipt.agent.name}.`}{" "}
            <button
              type="button"
              className="intake__link"
              onClick={() => void select(receipt.agent.id)}
            >
              Open the conversation
            </button>
          </span>
        )}
        {failure && (
          <span className="intake__failure" role="alert">
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
 *
 * Plain buttons in a popup rather than a listbox: a listbox is one focus stop
 * with its own selection model, and this one holds a button that opens the
 * Groups sheet, which no option of a listbox can be. The arrows move between
 * the buttons, and Escape or a press anywhere else closes it.
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
  const box = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const menuId = useId();
  useDismiss(() => onOpen(false), open);

  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      if (event.target instanceof Node && box.current?.contains(event.target)) return;
      onOpen(false);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open, onOpen]);

  const buttons = () => [...(menu.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])];

  // Onto whoever is written to now, so the arrows start from there.
  useEffect(() => {
    if (!open) return;
    const all = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    (all.find((one) => one.getAttribute("aria-current") === "true") ?? all[0])?.focus();
  }, [open]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const all = buttons();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const to =
      event.key === "ArrowDown"
        ? (at + 1) % all.length
        : event.key === "ArrowUp"
          ? (at - 1 + all.length) % all.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? all.length - 1
              : null;
    if (to === null) return;
    event.preventDefault();
    all[to]?.focus();
  };

  const shelves = groups
    .map((group) => ({ group, members: agents.filter((agent) => agent.groupId === group.id) }))
    .filter((shelf) => shelf.members.length > 0);
  const named = shelves.length > 1;
  return (
    <div className="intake__picker" ref={box}>
      <button
        type="button"
        className="intake__target"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
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
          id={menuId}
          ref={menu}
          className="intake__menu"
          role="group"
          aria-label="Write to"
          onKeyDown={onKeyDown}
        >
          {shelves.map(({ group, members }) => (
            <div key={group.id}>
              {named && <p className="intake__group">{group.name}</p>}
              {members.map((agent) => (
                <button
                  key={agent.id}
                  type="button"
                  aria-current={agent.id === target?.id ? "true" : undefined}
                  className="intake__option"
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
          <button type="button" className="intake__option intake__manage" onClick={onManageGroups}>
            Groups and agents
          </button>
        </div>
      )}
    </div>
  );
}
