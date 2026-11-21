import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
import { inScope, type SearchResult, scopeLabel, searchResults } from "../lib/search";
import { useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { useSearchHits } from "../lib/useSearchHits";
import { Icon, type IconName } from "./Icon";
import { Scrim } from "./Scrim";

const FIND_SCOPES = ["all", "messages", "files"] as const;
type FindScope = (typeof FIND_SCOPES)[number];

export function Search({ onClose }: { onClose: () => void }) {
  const agents = useStore((s) => s.agents);
  const groups = useStore((s) => s.groups);
  const lastActive = useStore((s) => s.lastActive);
  const openMessage = useStore((s) => s.openMessage);
  const showFailure = useStore((s) => s.showFailure);
  const now = useNow(30_000);

  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<FindScope>("all");
  const lookup = useSearchHits(query);
  const hits = lookup?.kind === "ready" ? lookup.hits : null;
  const [cursor, setCursor] = useState(0);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const results = useMemo(
    () =>
      inScope(searchResults({ query, agents, groups, hits, lastActive, now }), scope).filter(
        (result) => result.kind === "messages" || result.kind === "files",
      ),
    [query, agents, groups, hits, lastActive, now, scope],
  );

  const at = Math.min(cursor, Math.max(0, results.length - 1));
  const chosen = results[at];

  const run = useCallback(
    (result: SearchResult) => {
      if (result.action.do !== "openMessage") return;
      void openMessage(result.action.channelId, result.action.messageId).catch(showFailure);
      onClose();
    },
    [onClose, openMessage, showFailure],
  );

  const isTop = useDismiss(onClose);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.isComposing || !isTop()) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setCursor((c) => Math.min(c + 1, Math.max(0, results.length - 1)));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setCursor((c) => Math.max(0, c - 1));
      } else if (event.key === "Tab") {
        event.preventDefault();
        const step = event.shiftKey ? -1 : 1;
        setScope(
          FIND_SCOPES[
            (FIND_SCOPES.indexOf(scope) + step + FIND_SCOPES.length) % FIND_SCOPES.length
          ] ?? "all",
        );
        setCursor(0);
      } else if (event.key === "Enter" && chosen) {
        event.preventDefault();
        run(chosen);
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [chosen, isTop, results.length, run, scope]);

  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>("[data-selected='true']")
      ?.scrollIntoView?.({ block: "nearest" });
  }, [at]);

  return (
    <Scrim>
      <button type="button" className="scrim__close" aria-label="Close search" onClick={onClose} />

      <div className="palette" role="dialog" aria-modal="true" aria-label="Search">
        <div className="palette__query">
          <span aria-hidden="true" className="palette__glass">
            <Icon name="search" />
          </span>
          <input
            ref={inputRef}
            className="palette__input"
            type="text"
            placeholder="Search messages and files"
            value={query}
            spellCheck={false}
            aria-label="Search Guaca"
            onChange={(event) => {
              setQuery(event.target.value);
              setCursor(0);
            }}
          />
          <button type="button" className="mobile-only btn btn--quiet" onClick={onClose}>
            Cancel
          </button>
        </div>

        <div className="palette__tabs" role="tablist" aria-label="What to search">
          {FIND_SCOPES.map((option) => (
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
              {lookup === null
                ? "Searching…"
                : lookup.kind === "failed"
                  ? "Could not search messages and files."
                  : query.trim()
                    ? `Nothing matching “${query.trim()}”.`
                    : "No messages or files yet."}
            </p>
          ) : (
            results.map((result, index) => (
              <Row
                key={result.key}
                result={result}
                selected={index === at}
                onHover={() => setCursor(index)}
                onPick={() => run(result)}
              />
            ))
          )}
        </div>
      </div>
    </Scrim>
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
            <Icon name={GLYPH[result.kind]} size={16} />
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

const GLYPH: Record<SearchResult["kind"], IconName> = {
  messages: "message",
  agents: "agent",
  groups: "group",
  files: "file",
  links: "external",
  routines: "clock",
  artifacts: "artifact",
  actions: "command",
};
