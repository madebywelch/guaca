import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { api, openExternal } from "../lib/ipc";
import {
  inScope,
  SCOPES,
  type Scope,
  type SearchAction,
  type SearchResult,
  scopeLabel,
  searchResults,
} from "../lib/search";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { type AgentCard, errorMessage, type Group, type SearchHits } from "../lib/types";

interface Props {
  onClose: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onEditGroup: (group: Group) => void;
  onNewAgent: () => void;
  onNewGroup: () => void;
  onOpenCafeteria: () => void;
  onOpenSettings: () => void;
}

/**
 * How long after a keystroke the store is asked about it.
 *
 * Short enough that the list feels live, long enough that holding a key down is
 * one query rather than eight. Agents, groups and actions do not wait for it:
 * they are matched from state already in hand and redraw on the keystroke
 * itself, so the palette always answers immediately for the things an operator
 * is most often looking for and fills in the transcript a moment later.
 */
const DEBOUNCE_MS = 120;

/** How many of each kind the store is asked for. */
const LIMIT = 25;

/**
 * The workspace search.
 *
 * One ranked list over seven kinds of thing, opened from the rail or with the
 * platform's find shortcut. Everything it can do is a jump or a settings pane.
 * Nothing here deletes, pauses or sends: a control you drive by typing and
 * pressing Enter is the wrong place to keep an irreversible action.
 */
export function Search({
  onClose,
  onEditAgent,
  onEditGroup,
  onNewAgent,
  onNewGroup,
  onOpenCafeteria,
  onOpenSettings,
}: Props) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const lastActive = useStore((s) => s.lastActive);
  const openMessage = useStore((s) => s.openMessage);
  const select = useStore((s) => s.select);
  const setBanner = useStore((s) => s.setBanner);
  const now = useNow(30_000);

  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<Scope>("all");
  const [hits, setHits] = useState<SearchHits | null>(null);
  const [cursor, setCursor] = useState(0);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // The half that lives in SQLite. Debounced, and stamped: the replies to two
  // keystrokes can arrive out of order, and the older one landing last leaves
  // the list showing results for a query the operator has already moved past.
  const issued = useRef(0);
  useEffect(() => {
    const wanted = ++issued.current;
    const timer = window.setTimeout(() => {
      void api
        .search(query, LIMIT)
        .then((found) => {
          if (issued.current === wanted) setHits(found);
        })
        .catch((error) => setBanner({ tone: "error", text: errorMessage(error) }));
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query, setBanner]);

  const results = useMemo(
    () => inScope(searchResults({ query, agents, groups, hits, lastActive, now }), scope),
    [query, agents, groups, hits, lastActive, now, scope],
  );

  // The list reorders under the cursor as answers arrive, so it is clamped
  // rather than trusted: row twelve of the previous result set is not row
  // twelve of this one, and acting on a stale index opens something nobody
  // pointed at.
  const at = Math.min(cursor, Math.max(0, results.length - 1));
  const chosen = results[at];

  const run = useCallback(
    (action: SearchAction) => {
      switch (action.do) {
        case "openChannel":
          void select(action.agentId);
          break;
        case "openMessage":
          void openMessage(action.channelId, action.messageId);
          break;
        case "openLink":
          void openExternal(action.url);
          break;
        case "editAgent": {
          const agent = agents.find((a) => a.id === action.agentId);
          if (agent) onEditAgent(agent);
          break;
        }
        case "editGroup": {
          const group = groups.find((g) => g.id === action.groupId);
          if (group) onEditGroup(group);
          break;
        }
        case "openSettings":
          onOpenSettings();
          break;
        case "newAgent":
          onNewAgent();
          break;
        case "newGroup":
          onNewGroup();
          break;
        case "openCafeteria":
          onOpenCafeteria();
          break;
      }
      onClose();
    },
    [
      agents,
      groups,
      onClose,
      onEditAgent,
      onEditGroup,
      onNewAgent,
      onNewGroup,
      onOpenCafeteria,
      onOpenSettings,
      openMessage,
      select,
    ],
  );

  // Bound to the window rather than to the panel. The palette is modal, and a
  // handler on the panel stops working the moment focus lands on a tab button,
  // which is exactly when somebody reaches for Escape.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        setCursor((c) => Math.min(c + 1, results.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setCursor((c) => Math.max(0, c - 1));
      } else if (event.key === "Tab") {
        // Tab cycles the filters. The alternative is taking a hand off the
        // keyboard to narrow a list you are already typing into.
        event.preventDefault();
        const step = event.shiftKey ? -1 : 1;
        setScope(SCOPES[(SCOPES.indexOf(scope) + step + SCOPES.length) % SCOPES.length]!);
        setCursor(0);
      } else if (event.key === "Enter" && chosen) {
        event.preventDefault();
        run(chosen.action);
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [chosen, onClose, results.length, run, scope]);

  // Keeps the selected row in view when the cursor is driven from the keyboard.
  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>("[data-selected='true']")
      ?.scrollIntoView({ block: "nearest" });
  }, [at]);

  return (
    <div className="scrim scrim--palette">
      {/* A real button, so dismissing by clicking away is reachable from the
          keyboard and announced, rather than being an invisible div handler. */}
      <button type="button" className="scrim__close" aria-label="Close search" onClick={onClose} />

      <div className="palette" role="dialog" aria-modal="true" aria-label="Search">
        <div className="palette__query">
          <span aria-hidden="true" className="palette__glass">
            ⌕
          </span>
          <input
            ref={inputRef}
            className="palette__input"
            type="text"
            placeholder="Search"
            value={query}
            spellCheck={false}
            aria-label="Search the workspace"
            onChange={(event) => {
              setQuery(event.target.value);
              setCursor(0);
            }}
          />
        </div>

        <div className="palette__tabs" role="tablist" aria-label="What to search">
          {SCOPES.map((option) => (
            <button
              key={option}
              type="button"
              role="tab"
              className="palette__tab"
              aria-selected={scope === option}
              onClick={() => {
                setScope(option);
                setCursor(0);
                inputRef.current?.focus();
              }}
            >
              {scopeLabel(option)}
            </button>
          ))}
        </div>

        <div className="palette__results" ref={listRef}>
          {results.length === 0 ? (
            <p className="palette__empty">
              {query.trim()
                ? `Nothing matching “${query.trim()}”.`
                : "Nothing here yet. Add an agent to start."}
            </p>
          ) : (
            results.map((result, index) => (
              <Row
                key={result.key}
                result={result}
                selected={index === at}
                onHover={() => setCursor(index)}
                onPick={() => run(result.action)}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function Row({
  result,
  selected,
  onHover,
  onPick,
}: {
  result: SearchResult;
  selected: boolean;
  onHover: () => void;
  onPick: () => void;
}) {
  return (
    <button
      type="button"
      className="palette__row"
      data-selected={selected}
      data-kind={result.kind}
      // Moving the pointer, not entering: a list that reorders under a still
      // mouse would otherwise take the cursor away from the keyboard.
      onMouseMove={onHover}
      onClick={onPick}
    >
      <span className="palette__face">
        {result.face ? (
          <AgentAvatar
            avatar={result.face.avatar}
            color={result.face.color}
            seed={result.face.seed}
            size="sm"
          />
        ) : (
          <span aria-hidden="true" className="palette__glyph">
            {GLYPH[result.kind]}
          </span>
        )}
      </span>
      <span className="palette__text">
        <span className="palette__title">{result.title}</span>
        {result.detail && <span className="palette__detail">{result.detail}</span>}
      </span>
      <span className="palette__meta">{result.meta}</span>
    </button>
  );
}

/** A mark for the rows with no face of their own. */
const GLYPH: Record<SearchResult["kind"], string> = {
  messages: "❝",
  agents: "◍",
  groups: "▤",
  files: "◫",
  links: "↗",
  routines: "◷",
  actions: "⌘",
};
