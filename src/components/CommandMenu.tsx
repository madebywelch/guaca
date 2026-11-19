import { Fragment, type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
import { defaultTarget } from "../lib/field";
import { useHeld, useShown } from "../lib/held";
import { api } from "../lib/ipc";
import { formatCombo } from "../lib/keybinds";
import {
  type Asking,
  firstChoice,
  MENU_SECTIONS,
  type MenuAction,
  type MenuRow,
  menuRows,
  nextRecipient,
  type Place,
  requestAt,
} from "../lib/menu";
import { numbered } from "../lib/rail";
import type { Section } from "../lib/sections";
import { useStore } from "../lib/store";
import { type AgentId, errorMessage } from "../lib/types";
import { Icon } from "./Icon";

interface Props {
  onClose: () => void;
  onPlace: (place: Place, query?: string) => void;
  onSection: (section: Section) => void;
}

const ALLOW = formatCombo({ key: "Enter", mod: true });
const DENY = formatCombo({ key: "Backspace", mod: true });
const RETURN = formatCombo({ key: "Enter" });

export function CommandMenu({ onClose, onPlace, onSection }: Props) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const shownNow = useShown();
  const activity = shownNow.activity;
  const lastActive = useStore((s) => s.lastActive);
  const railGroup = useStore((s) => s.railGroup);
  const open = useStore((s) => s.selected);
  const prefs = useStore((s) => s.prefs);
  const allPending = useStore((s) => s.pending);
  const allStuck = useStore((s) => s.stuck);
  const select = useStore((s) => s.select);
  const leaveChannel = useStore((s) => s.leaveChannel);
  const setPrefs = useStore((s) => s.setPrefs);
  const decide = useStore((s) => s.decideApproval);
  const answer = useStore((s) => s.answerQuestion);
  const showFailure = useStore((s) => s.showFailure);
  const heldId = useHeld((s) => s.held?.id);
  const hold = useHeld((s) => s.hold);
  const tell = useHeld((s) => s.tell);

  const ordered = useMemo(() => numbered(agents, groups), [agents, groups]);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<AgentId | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    input.current?.focus();
  }, []);

  const recipient =
    ordered.find((agent) => agent.id === picked) ??
    ordered.find((agent) => agent.id === open) ??
    defaultTarget(ordered, lastActive, railGroup);

  const rows = menuRows({
    query,
    recipient,
    agents: ordered,
    activity,
    pending: allPending.filter((one) => one.id !== heldId),
    stuck: allStuck.filter((one) => one.id !== heldId),
    open,
    prefs,
  });
  const selected = rows.some((row) => row.id === chosen) ? chosen : firstChoice(rows, query);
  const current = requestAt(rows, selected);

  useEffect(() => {
    list.current
      ?.querySelector<HTMLElement>("[data-selected='true']")
      ?.scrollIntoView?.({ block: "nearest" });
  }, [selected]);

  const isTop = useDismiss(onClose);

  const nameOf = (agentId: AgentId) =>
    agents.find((agent) => agent.id === agentId)?.name ?? "A deleted agent";

  const run = (action: MenuAction) => {
    onClose();
    switch (action.do) {
      case "write": {
        const to = nameOf(action.agentId);
        api
          .sendMessage(action.agentId, action.text)
          .then(() => tell({ kind: "sent", agentId: action.agentId, text: `Sent to ${to}.` }))
          .catch((error: unknown) => showFailure(new Error(errorMessage(error))));
        return;
      }
      case "decide":
        hold(
          action.request.id,
          `${action.decision === "deny" ? "Denied" : "Allowed"} what ${nameOf(action.request.agentId)} asked.`,
          () => decide(action.request.id, action.decision),
        );
        return;
      case "answer":
        hold(action.request.id, `Answered ${nameOf(action.request.agentId)}: ${action.text}`, () =>
          answer(action.request.id, action.text),
        );
        return;
      case "open":
        void select(action.agentId);
        return;
      case "place":
        if (action.place === "today") leaveChannel();
        else onPlace(action.place, action.query);
        return;
      case "section":
        onSection(action.section);
        return;
      case "look":
        setPrefs(action.patch);
        return;
    }
  };

  const verdict = (decision: "allow" | "deny") => {
    const row = rows.find(
      (one) =>
        one.request === current && one.action.do === "decide" && one.action.decision === decision,
    );
    if (row) run(row.action);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || !isTop()) return;
    const mod = event.metaKey || event.ctrlKey;
    const at = rows.findIndex((row) => row.id === selected);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const to =
        event.key === "ArrowDown" ? Math.min(rows.length - 1, at + 1) : Math.max(0, at - 1);
      setChosen(rows[to]?.id ?? null);
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      setPicked(nextRecipient(ordered, recipient, event.shiftKey ? -1 : 1)?.id ?? null);
      return;
    }
    if (mod && event.key === "Enter") {
      event.preventDefault();
      verdict("allow");
      return;
    }
    if (mod && event.key === "Backspace") {
      event.preventDefault();
      verdict("deny");
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const row = rows[at];
      if (row) run(row.action);
    }
  };

  const sections: MenuRow["section"][] = [null, ...MENU_SECTIONS];
  return (
    <div className="scrim scrim--palette">
      <button
        type="button"
        className="scrim__close"
        aria-label="Close the command menu"
        onClick={onClose}
      />

      <div
        className="palette palette--menu"
        role="dialog"
        aria-modal="true"
        aria-label="Command menu"
      >
        <div className="palette__query">
          <span className="palette__to" title="Tab changes who you write to">
            {recipient && (
              <AgentAvatar
                avatar={recipient.avatar}
                color={recipient.color}
                size="xs"
                seed={recipient.id}
              />
            )}
            <span>{recipient ? `To ${recipient.name}` : "Nobody to write to yet"}</span>
          </span>
          <input
            ref={input}
            className="palette__input"
            type="text"
            value={query}
            spellCheck={false}
            placeholder="Write, or find an agent, an answer, an action or a setting"
            aria-label="Command menu"
            aria-describedby="command-menu-to"
            onChange={(event) => {
              setQuery(event.target.value);
              setChosen(null);
            }}
            onKeyDown={onKeyDown}
          />
          <span id="command-menu-to" hidden>
            {recipient ? `Writing to ${recipient.name}. Tab changes who.` : ""}
          </span>
          <button type="button" className="mobile-only btn btn--quiet" onClick={onClose}>
            Cancel
          </button>
        </div>

        <div className="palette__results" ref={list}>
          {rows.length === 0 ? (
            <p className="palette__empty">Nothing matches, and there is nobody to write to yet.</p>
          ) : (
            sections.map((section) => {
              const inside = rows.filter((row) => row.section === section);
              if (inside.length === 0) return null;
              return (
                <div key={section ?? "write"}>
                  {section && <p className="palette__section">{section}</p>}
                  {inside.map((row, index) => (
                    <Fragment key={row.id}>
                      {row.asking && row.asking.id !== inside[index - 1]?.asking?.id && (
                        <Heading asking={row.asking} />
                      )}
                      <MenuItem
                        row={row}
                        selected={row.id === selected}
                        keys={
                          row.request !== undefined &&
                          row.request === current &&
                          row.action.do === "decide"
                            ? row.action.decision === "allow"
                              ? ALLOW
                              : row.action.decision === "deny"
                                ? DENY
                                : undefined
                            : row.key
                        }
                        onHover={() => setChosen(row.id)}
                        onPick={() => run(row.action)}
                      />
                    </Fragment>
                  ))}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

function Heading({ asking }: { asking: Asking }) {
  return (
    <p className="palette__asking">
      <span className="palette__face">
        {asking.agent && (
          <AgentAvatar
            avatar={asking.agent.avatar}
            color={asking.agent.color}
            seed={asking.agent.id}
            size="sm"
          />
        )}
      </span>
      <span className="palette__text">
        {asking.text.startsWith(`${asking.who} `) ? (
          <>
            <strong className="palette__who">{asking.who}</strong>
            {asking.text.slice(asking.who.length)}
          </>
        ) : (
          <>
            <strong className="palette__who">{asking.who}:</strong> {asking.text}
          </>
        )}
      </span>
    </p>
  );
}

function MenuItem({
  row,
  selected,
  keys,
  onHover,
  onPick,
}: {
  row: MenuRow;
  selected: boolean;
  keys: string | undefined;
  onHover: () => void;
  onPick: () => void;
}) {
  return (
    <button
      type="button"
      tabIndex={-1}
      className="palette__row"
      data-selected={selected}
      aria-current={selected ? "true" : undefined}
      onMouseMove={onHover}
      onClick={onPick}
    >
      <span className="palette__face">
        {row.asking ? null : row.face ? (
          <AgentAvatar
            avatar={row.face.avatar}
            color={row.face.color}
            seed={row.face.id}
            size="sm"
          />
        ) : (
          <span aria-hidden="true" className="palette__glyph">
            <Icon name={row.icon ?? "command"} size={16} />
          </span>
        )}
      </span>
      <span className="palette__text">
        <span className="palette__title">{row.title}</span>
        {row.detail && <span className="palette__detail">{row.detail}</span>}
      </span>
      <span className="palette__meta">
        {selected && <kbd className="palette__key">{RETURN}</kbd>}
        {keys && <kbd className="palette__key">{keys}</kbd>}
      </span>
    </button>
  );
}
