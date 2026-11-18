import {
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { needsAnswer } from "../lib/decisions";
import { useDismiss } from "../lib/dismiss";
import { doing } from "../lib/doing";
import { useFollowBottom } from "../lib/follow";
import { api } from "../lib/ipc";
import { exchange, headline, liveReply, panelOrder, said, workingCount } from "../lib/menubar";
import { finished, moreLine, newestByAgent, PANEL_LINES } from "../lib/outcomes";
import { groupNamed } from "../lib/presence";
import { useLiveAgents, useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { BEFORE_RELOAD, workspaceOrigin } from "../lib/transport";
import { type AgentCard, type AgentId, errorMessage, type Tokens } from "../lib/types";
import { Brand } from "./Brand";
import { Desk } from "./Desk";
import { FinishedLine } from "./ForYou";
import { Markdown } from "./Markdown";
import { compact, money, priced } from "./Spend";

const RECENT = 3;

export function MenubarPanel({ ready }: { ready: boolean }) {
  const agents = useLiveAgents();
  const activity = useStore((s) => s.activity);
  const lastActive = useStore((s) => s.lastActive);
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const decisions = useStore((s) => s.decisions);
  const activeRun = useStore((s) => s.activeRun);
  const session = useStore((s) => s.sessionSpend);
  const banner = useStore((s) => s.banner);
  const setBanner = useStore((s) => s.setBanner);
  const showFailure = useStore((s) => s.showFailure);
  const [open, setOpen] = useState<AgentId | null>(null);
  const [draftKey] = useState(() => `guaca.panel.drafts:${new URL(workspaceOrigin()).origin}`);
  const [drafts] = useState(() => {
    try {
      const saved: unknown = JSON.parse(sessionStorage.getItem(draftKey) ?? "{}");
      if (saved && typeof saved === "object" && !Array.isArray(saved)) {
        return new Map(
          Object.entries(saved).filter(
            (entry): entry is [string, string] => typeof entry[1] === "string",
          ),
        );
      }
    } catch {}
    return new Map<AgentId, string>();
  });
  const saveDrafts = useCallback(() => {
    sessionStorage.setItem(draftKey, JSON.stringify(Object.fromEntries(drafts)));
  }, [draftKey, drafts]);
  useLayoutEffect(() => {
    const preserve = (event: Event) => {
      try {
        saveDrafts();
      } catch {
        event.preventDefault();
      }
    };
    window.addEventListener(BEFORE_RELOAD, preserve);
    return () => window.removeEventListener(BEFORE_RELOAD, preserve);
  }, [saveDrafts]);

  const root = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  useFit(root, body, content);

  const talking = open ? agents.find((agent) => agent.id === open) : undefined;

  const scroller = useCallback(() => body.current, []);
  const listing = talking === undefined;
  useLayoutEffect(() => {
    if (listing && body.current) body.current.scrollTop = 0;
  }, [listing]);

  useDismiss(() => {
    if (talking) setOpen(null);
    else void api.closeMenubar();
  });

  const waiting = pending.length + stuck.length + decisions.filter(needsAnswer).length;
  const running = Object.values(activeRun).filter((run) => run !== undefined).length;

  const stopAll = () => {
    void api.stopEverything().catch((error) => showFailure(error));
  };

  return (
    <div className="menubar" ref={root}>
      <header className="menubar__head">
        <Brand waiting={waiting > 0} />
        <p className="menubar__headline" data-waiting={waiting > 0 ? "" : undefined}>
          {headline(waiting, workingCount(agents, activity))}
        </p>
      </header>

      <div className="menubar__body" ref={body}>
        <div ref={content}>
          {banner && (
            <div className={banner.tone === "error" ? "banner banner--error" : "banner"}>
              <span>{banner.text}</span>
              <button
                type="button"
                className="btn btn--quiet btn--compact"
                onClick={() => setBanner(null)}
              >
                Dismiss
              </button>
            </div>
          )}
          {!ready ? (
            <p className="menubar__note" role="status">
              Connecting to your workspace…
            </p>
          ) : talking ? (
            <Conversation
              key={talking.id}
              agent={talking}
              scroller={scroller}
              drafts={drafts}
              saveDrafts={saveDrafts}
              onBack={() => setOpen(null)}
            />
          ) : (
            <Overview agents={panelOrder(agents, { activity, lastActive })} onOpen={setOpen} />
          )}
        </div>
      </div>

      <footer className="menubar__foot">
        <span className="menubar__spend">{spent(session)}</span>
        {running > 0 && (
          <button
            type="button"
            className="btn btn--quiet btn--compact"
            title={
              running === 1
                ? "Stop the conversation that is running"
                : `Stop all ${running} conversations that are running`
            }
            onClick={stopAll}
          >
            {running === 1 ? "Stop" : "Stop all"}
          </button>
        )}
        <button type="button" className="btn btn--compact" onClick={() => void api.openWindow(null)}>
          Open Guaca
        </button>
      </footer>
    </div>
  );
}

function spent(session: Tokens): string {
  if (session.calls === 0) return "Nothing spent yet";
  if (priced(session.cost)) return `${money(session.cost)} this session`;
  return `${compact(session.prompt + session.completion)} tokens this session`;
}

function Overview({ agents, onOpen }: { agents: AgentCard[]; onOpen: (id: AgentId) => void }) {
  const decisions = useStore((s) => s.decisions);
  const groups = useStore((s) => s.groups);
  const activity = useStore((s) => s.activity);
  const stuck = useStore((s) => s.stuck);
  const trail = useStore((s) => s.trail);
  const lastActive = useStore((s) => s.lastActive);
  const finishedAt = useStore((s) => s.finishedAt);
  const outcomes = useStore((s) => s.outcomes);
  const seeOutcomes = useStore((s) => s.seeOutcomes);
  const newest = useMemo(() => newestByAgent(outcomes), [outcomes]);
  const now = useNow();

  const open = decisions.filter(needsAnswer).length;
  const news = finished(outcomes);
  const more = moreLine(news.length, PANEL_LINES);
  const crewOf = (agent: AgentCard) => groupNamed(groups, agent.groupId);

  return (
    <>
      {open > 0 && (
        <button
          type="button"
          className="menubar__decisions"
          onClick={() => void api.openWindow({ kind: "forYou" })}
        >
          {open === 1 ? "1 decision needs you" : `${open} decisions need you`}
          <span className="menubar__decisions-go">Open For you</span>
        </button>
      )}

      <Desk onOpenChannel={(id) => void api.openWindow({ kind: "agent", id })} />

      {news.length > 0 && (
        <section className="menubar__finished" aria-labelledby="menubar-finished">
          <h3 id="menubar-finished">Finished</h3>
          {news.slice(0, PANEL_LINES).map((outcome) => (
            <FinishedLine
              key={outcome.runId}
              outcome={outcome}
              agent={agents.find((agent) => agent.id === outcome.agentId)}
              now={now}
              onOpen={() => {
                void seeOutcomes({ runIds: [outcome.runId] });
                void api.openWindow({ kind: "agent", id: outcome.agentId });
              }}
            />
          ))}
          {more && (
            <button
              type="button"
              className="menubar__more"
              onClick={() => void api.openWindow({ kind: "forYou" })}
            >
              {more}
            </button>
          )}
        </section>
      )}

      {agents.length === 0 ? (
        <p className="menubar__note">No agents yet. Open Guaca to hire an agent.</p>
      ) : (
        <nav className="menubar__agents" aria-label="Agents">
          {agents.map((agent) => {
            const label = doing(
              agent.id,
              { activity, stuck, trail, lastActive, finished: newest },
              now,
            );
            const crew = crewOf(agent);
            return (
              <button
                key={agent.id}
                type="button"
                className="agent-row"
                title={label.title}
                data-lifecycle={agent.lifecycle}
                style={{ "--accent": agent.color } as React.CSSProperties}
                onClick={() => onOpen(agent.id)}
              >
                <AgentAvatar
                  avatar={agent.avatar}
                  color={agent.color}
                  activity={activity[agent.id]}
                  lifecycle={agent.lifecycle}
                  work={trail[agent.id]}
                  escalated={stuck.some((one) => one.agentId === agent.id)}
                  finishedAt={finishedAt[agent.id]}
                  seed={agent.id}
                  size="xs"
                />
                <span className="agent-row__title">
                  <span className="agent-row__name">{agent.name}</span>
                  {crew && <span className="menubar__crew">{crew}</span>}
                </span>
                <span className="agent-row__meta" data-state={label.kind}>
                  {label.text}
                </span>
              </button>
            );
          })}
        </nav>
      )}
    </>
  );
}

function Conversation({
  agent,
  scroller,
  drafts,
  saveDrafts,
  onBack,
}: {
  agent: AgentCard;
  scroller: () => HTMLElement | null;
  drafts: Map<AgentId, string>;
  saveDrafts: () => void;
  onBack: () => void;
}) {
  const loadChannel = useStore((s) => s.loadChannel);
  const messages = useStore((s) => s.messages[agent.id]);
  const streams = useStore((s) => s.streams);
  const run = useStore((s) => s.activeRun[agent.id]);
  const activity = useStore((s) => s.activity);
  const stuck = useStore((s) => s.stuck);
  const trail = useStore((s) => s.trail);
  const lastActive = useStore((s) => s.lastActive);
  const finishedAt = useStore((s) => s.finishedAt);
  const outcomes = useStore((s) => s.outcomes);
  const news = useMemo(() => newestByAgent(outcomes), [outcomes]);
  const now = useNow();

  const [draft, setDraftState] = useState(() => drafts.get(agent.id) ?? "");
  const setDraft = (text: string) => {
    setDraftState(text);
    if (text) drafts.set(agent.id, text);
    else drafts.delete(agent.id);
    try {
      saveDrafts();
    } catch {}
  };
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const field = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setError(null);
    loadChannel(agent.id).catch((cause) => setError(errorMessage(cause)));
    field.current?.focus();
  }, [agent.id, loadChannel]);

  useEffect(() => {
    const shown = () => field.current?.focus();
    window.addEventListener("focus", shown);
    return () => window.removeEventListener("focus", shown);
  }, []);

  const recent = exchange(messages, agent.id, RECENT);
  const live = liveReply(streams, agent.id);
  const label = doing(agent.id, { activity, stuck, trail, lastActive, finished: news }, now);

  const { ref: followRef, follow, pin } = useFollowBottom();
  useLayoutEffect(() => {
    const detach = followRef(scroller());
    return typeof detach === "function" ? detach : undefined;
  }, [followRef, scroller]);
  useLayoutEffect(() => pin(), [agent.id, pin]);
  const newest = recent.at(-1)?.id;
  useLayoutEffect(() => follow(), [newest, live, follow]);

  const send = async (event?: FormEvent) => {
    event?.preventDefault();
    const text = draft.trim();
    if (!text || sending) return;
    setSending(true);
    setError(null);
    try {
      await api.sendMessage(agent.id, text);
      setDraft("");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    void send();
  };

  return (
    <section className="menubar__talk" aria-label={`Conversation with ${agent.name}`}>
      <header
        className="menubar__talk-head"
        style={{ "--accent": agent.color } as React.CSSProperties}
      >
        <button type="button" className="btn btn--quiet btn--compact" onClick={onBack}>
          Back
        </button>
        <AgentAvatar
          avatar={agent.avatar}
          color={agent.color}
          size="sm"
          activity={activity[agent.id]}
          lifecycle={agent.lifecycle}
          work={trail[agent.id]}
          escalated={stuck.some((one) => one.agentId === agent.id)}
          finishedAt={finishedAt[agent.id]}
          seed={agent.id}
        />
        <span className="menubar__talk-who">
          <span className="agent-row__name">{agent.name}</span>
          <span className="agent-row__meta" data-state={label.kind}>
            {label.text}
          </span>
        </span>
        {run && (
          <button
            type="button"
            className="btn btn--quiet btn--compact"
            onClick={() => void api.stopRun(run).catch((cause) => setError(errorMessage(cause)))}
          >
            Stop
          </button>
        )}
        <button
          type="button"
          className="btn btn--compact"
          onClick={() => void api.openWindow({ kind: "agent", id: agent.id })}
        >
          Open
        </button>
      </header>

      {recent.length === 0 && !live ? (
        <p className="menubar__note">Nothing said between you yet.</p>
      ) : (
        <div className="menubar__said">
          {recent.map((message) => (
            <article key={message.id} data-from={message.from.kind === "human" ? "you" : "agent"}>
              <span className="menubar__speaker">
                {message.from.kind === "human" ? "You" : agent.name}
              </span>
              <Clamped onMore={() => void api.openWindow({ kind: "agent", id: agent.id })}>
                <Markdown>{said(message)}</Markdown>
              </Clamped>
            </article>
          ))}
          {live && (
            <article data-from="agent" data-live="">
              <span className="menubar__speaker">{agent.name}</span>
              <Clamped onMore={() => void api.openWindow({ kind: "agent", id: agent.id })}>
                <Markdown live>{live}</Markdown>
              </Clamped>
            </article>
          )}
        </div>
      )}

      <form className="menubar__compose" onSubmit={(event) => void send(event)}>
        <textarea
          ref={field}
          className="menubar__field"
          rows={2}
          value={draft}
          placeholder={`Message ${agent.name}`}
          aria-label={`Message ${agent.name}`}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onKeyDown}
        />
        <button
          type="submit"
          className="btn btn--primary btn--compact"
          disabled={sending || draft.trim() === ""}
        >
          Send
        </button>
      </form>
      {error && (
        <p role="alert" className="menubar__error">
          {error}
        </p>
      )}
    </section>
  );
}

function Clamped({ children, onMore }: { children: ReactNode; onMore: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [clipped, setClipped] = useState(false);

  useLayoutEffect(() => {
    const node = box.current;
    if (!node) return;
    const measure = () => setClipped(node.scrollHeight > node.clientHeight + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return () => observer.disconnect();
  }, []);

  return (
    <>
      <div className="menubar__words" ref={box} data-clipped={clipped ? "" : undefined}>
        {children}
      </div>
      {clipped && (
        <button type="button" className="menubar__more" onClick={onMore}>
          Read the rest in Guaca
        </button>
      )}
    </>
  );
}

function useFit(
  root: RefObject<HTMLElement | null>,
  body: RefObject<HTMLElement | null>,
  content: RefObject<HTMLElement | null>,
) {
  const last = useRef(0);
  const measure = useCallback(() => {
    const [whole, scroller, inner] = [root.current, body.current, content.current];
    if (!whole || !scroller || !inner) return;
    const chrome = whole.offsetHeight - scroller.clientHeight;
    const wanted = Math.ceil(chrome + inner.offsetHeight);
    if (wanted === last.current || wanted <= chrome) return;
    last.current = wanted;
    void api
      .fitMenubar(wanted)
      .catch((error) => console.warn(`The panel could not be resized: ${errorMessage(error)}`));
  }, [root, body, content]);

  useLayoutEffect(measure);

  useLayoutEffect(() => {
    const [whole, inner] = [root.current, content.current];
    if (!whole || !inner) return;
    const observer = new ResizeObserver(measure);
    observer.observe(whole);
    observer.observe(inner);
    window.addEventListener("focus", measure);
    return () => {
      observer.disconnect();
      window.removeEventListener("focus", measure);
    };
  }, [root, content, measure]);
}
