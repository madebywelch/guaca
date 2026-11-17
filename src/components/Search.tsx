import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
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
import type { AgentCard, Group, SearchHits } from "../lib/types";

interface Props {
  initial?: string;
  onClose: () => void;
  onOpenChannel?: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onEditGroup: (group: Group) => void;
  onNewAgent: () => void;
  onNewGroup: () => void;
  onOpenCafeteria: () => void;
  onOpenSettings: () => void;
}

const DEBOUNCE_MS = 120;

const LIMIT = 25;

type Request =
  | { phase: "inactive" }
  | { phase: "debouncing" | "searching" | "settled"; generation: number };

type RequestEvent =
  | { type: "begin"; generation: number }
  | { type: "start" | "finish" | "cancel"; generation: number };

export function searchRequest(state: Request, event: RequestEvent): Request {
  if (event.type === "begin") return { phase: "debouncing", generation: event.generation };
  if (state.phase === "inactive" || state.generation !== event.generation) return state;
  if (event.type === "cancel") return { phase: "inactive" };
  if (event.type === "start" && state.phase === "debouncing") {
    return { phase: "searching", generation: event.generation };
  }
  if (event.type === "finish" && state.phase === "searching") {
    return { phase: "settled", generation: event.generation };
  }
  return state;
}

export function Search({
  initial = "",
  onClose,
  onOpenChannel,
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
  const showFailure = useStore((s) => s.showFailure);
  const showArtifacts = useStore((s) => s.showArtifacts);
  const now = useNow(30_000);

  const [query, setQuery] = useState(initial);
  const [scope, setScope] = useState<Scope>("all");
  const [hits, setHits] = useState<SearchHits | null>(null);
  const [cursor, setCursor] = useState(0);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const issued = useRef(0);
  const request = useRef<Request>({ phase: "inactive" });
  useEffect(() => {
    const wanted = ++issued.current;
    request.current = searchRequest(request.current, { type: "begin", generation: wanted });
    const timer = window.setTimeout(() => {
      request.current = searchRequest(request.current, { type: "start", generation: wanted });
      void api
        .search(query, LIMIT)
        .then((found) => {
          const next = searchRequest(request.current, { type: "finish", generation: wanted });
          if (next !== request.current) setHits(found);
          request.current = next;
        })
        .catch((error) => {
          const next = searchRequest(request.current, { type: "finish", generation: wanted });
          if (next !== request.current) showFailure(error);
          request.current = next;
        });
    }, DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
      request.current = searchRequest(request.current, { type: "cancel", generation: wanted });
    };
  }, [query, showFailure]);

  const results = useMemo(
    () => inScope(searchResults({ query, agents, groups, hits, lastActive, now }), scope),
    [query, agents, groups, hits, lastActive, now, scope],
  );

  const at = Math.min(cursor, Math.max(0, results.length - 1));
  const chosen = results[at];

  const run = useCallback(
    (action: SearchAction) => {
      switch (action.do) {
        case "openChannel":
          void select(action.agentId);
          onOpenChannel?.();
          break;
        case "openMessage":
          void openMessage(action.channelId, action.messageId);
          onOpenChannel?.();
          break;
        case "openLink":
          void openExternal(action.url);
          break;
        case "openArtifact":
          showArtifacts({ id: action.artifactId });
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
      onOpenChannel,
      openMessage,
      select,
      showArtifacts,
    ],
  );

  const isTop = useDismiss(onClose);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.isComposing || !isTop()) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setCursor((c) => Math.min(c + 1, results.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setCursor((c) => Math.max(0, c - 1));
      } else if (event.key === "Tab") {
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
  }, [chosen, isTop, results.length, run, scope]);

  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>("[data-selected='true']")
      ?.scrollIntoView({ block: "nearest" });
  }, [at]);

  return (
    <div className="scrim scrim--palette">
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
            aria-label="Search Guaca"
            onChange={(event) => {
              setQuery(event.target.value);
              setCursor(0);
            }}
          />
          <button type="button" className="mobile-only btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
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

const GLYPH: Record<SearchResult["kind"], string> = {
  messages: "❝",
  agents: "◍",
  groups: "▤",
  files: "◫",
  links: "↗",
  routines: "◷",
  artifacts: "❏",
  actions: "⌘",
};
