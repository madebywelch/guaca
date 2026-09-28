import {
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { needsAnswer } from "../lib/decisions";
import { doing } from "../lib/doing";
import { useFollowBottom } from "../lib/follow";
import { api } from "../lib/ipc";
import { exchange, headline, liveReply, panelOrder, said, workingCount } from "../lib/menubar";
import { useLiveAgents, useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { type AgentCard, type AgentId, errorMessage, type Tokens } from "../lib/types";
import { Brand } from "./Brand";
import { Desk } from "./Desk";
import { Markdown } from "./Markdown";
import { compact, money, priced } from "./Spend";

/**
 * How many of the last things said a conversation opens on.
 *
 * Enough to pick a conversation back up, which is what the panel is for. The
 * window has the rest, one click away, and a panel that scrolled back through a
 * transcript would be the window drawn smaller.
 */
const RECENT = 3;

/**
 * Guaca under the menu bar icon.
 *
 * What a glance at the corner of the screen is for, and nothing past it: what
 * is waiting on the operator, answerable where it is seen; who is doing what;
 * and a line to any one of them without bringing the window back. Everything
 * else is the window's, and every surface here has a way into it.
 *
 * A second client of the host the window is attached to, with a store of its
 * own, rather than a view the window feeds. Answering a request here goes
 * through the same store action the window's desk does, so both copies of a
 * request settle together, and nothing here raises a notification or reports
 * to the icon: those are the window's, and two pages doing them is every
 * interruption twice.
 */
export function MenubarPanel({ ready }: { ready: boolean }) {
  const agents = useLiveAgents();
  const activity = useStore((s) => s.activity);
  const building = useStore((s) => s.building);
  const lastActive = useStore((s) => s.lastActive);
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const decisions = useStore((s) => s.decisions);
  const activeRun = useStore((s) => s.activeRun);
  const session = useStore((s) => s.sessionSpend);
  const banner = useStore((s) => s.banner);
  const setBanner = useStore((s) => s.setBanner);
  const [open, setOpen] = useState<AgentId | null>(null);
  // What was being typed to each agent, for as long as the panel lives. Going
  // back to the list is not deciding against a message, and the window keeps a
  // draft across every change of view for the same reason.
  const drafts = useRef(new Map<AgentId, string>());

  const root = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  useFit(root, body, content);

  // An agent deleted while its conversation was open takes the conversation
  // with it, and the list is what is left.
  const talking = open ? agents.find((agent) => agent.id === open) : undefined;

  // A conversation follows its end and the list is read from the top, and the
  // two share one scroller, so coming back to the list is coming back to its
  // top rather than to wherever the conversation was left.
  const scroller = useCallback(() => body.current, []);
  const listing = talking === undefined;
  useLayoutEffect(() => {
    if (listing && body.current) body.current.scrollTop = 0;
  }, [listing]);

  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      if (talking) setOpen(null);
      else void api.closeMenubar();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [talking]);

  const waiting = pending.length + stuck.length + decisions.filter(needsAnswer).length;
  const running = Object.values(activeRun).filter((run) => run !== undefined).length;

  const stopAll = () => {
    void api
      .stopEverything()
      .catch((error) => setBanner({ tone: "error", text: errorMessage(error) }));
  };

  return (
    <div className="menubar" ref={root}>
      <header className="menubar__head">
        <Brand waiting={waiting > 0} />
        <p className="menubar__headline" data-waiting={waiting > 0 ? "" : undefined}>
          {headline(waiting, workingCount(agents, activity, building))}
        </p>
      </header>

      <div className="menubar__body" ref={body}>
        <div ref={content}>
          {banner && (
            <div className={banner.tone === "error" ? "banner banner--error" : "banner"}>
              <span>{banner.text}</span>
              <button
                type="button"
                className="btn btn--ghost btn--small"
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
              drafts={drafts.current}
              onBack={() => setOpen(null)}
            />
          ) : (
            <Overview
              agents={panelOrder(agents, { activity, building, lastActive })}
              onOpen={setOpen}
            />
          )}
        </div>
      </div>

      <footer className="menubar__foot">
        <span className="menubar__spend">{spent(session)}</span>
        {running > 0 && (
          <button
            type="button"
            className="btn btn--ghost btn--small"
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
        <button type="button" className="btn btn--small" onClick={() => void api.openWindow(null)}>
          Open Guaca
        </button>
      </footer>
    </div>
  );
}

/**
 * What this session has spent, in the width a footer has.
 *
 * The price when there is one worth drawing and the tokens when there is not,
 * under the same floor the window's meters use: a local server or a plan
 * prices nothing, and `$0.0000` is seven characters saying so.
 */
function spent(session: Tokens): string {
  if (session.calls === 0) return "Nothing spent yet";
  if (priced(session.cost)) return `${money(session.cost)} this session`;
  return `${compact(session.prompt + session.completion)} tokens this session`;
}

/** The list: what is waiting, then everybody. */
function Overview({ agents, onOpen }: { agents: AgentCard[]; onOpen: (id: AgentId) => void }) {
  const decisions = useStore((s) => s.decisions);
  const groups = useStore((s) => s.groups);
  const activity = useStore((s) => s.activity);
  const stuck = useStore((s) => s.stuck);
  const building = useStore((s) => s.building);
  const trail = useStore((s) => s.trail);
  const lastActive = useStore((s) => s.lastActive);
  const finishedAt = useStore((s) => s.finishedAt);
  const now = useNow();

  const open = decisions.filter(needsAnswer).length;
  // A crew is named only when there is another to tell it from, which is the
  // rule the crews' column and the icon's tooltip are drawn by.
  const crewOf = (agent: AgentCard) =>
    groups.length > 1 ? groups.find((group) => group.id === agent.groupId)?.name : undefined;

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

      {agents.length === 0 ? (
        <p className="menubar__note">No agents yet. Open Guaca to hire a crew.</p>
      ) : (
        <nav className="menubar__agents" aria-label="Agents">
          {agents.map((agent) => {
            const label = doing(agent.id, { activity, stuck, building, trail, lastActive }, now);
            const crew = crewOf(agent);
            return (
              <button
                key={agent.id}
                type="button"
                className="agent-row"
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

/** One agent: the last few things said, what it is writing now, and a line back. */
function Conversation({
  agent,
  scroller,
  drafts,
  onBack,
}: {
  agent: AgentCard;
  /** The panel's scrolling body, which this follows to its end while it is open. */
  scroller: () => HTMLElement | null;
  /** Unsent words, by agent, kept by the panel so leaving is not discarding. */
  drafts: Map<AgentId, string>;
  onBack: () => void;
}) {
  const loadChannel = useStore((s) => s.loadChannel);
  const messages = useStore((s) => s.messages[agent.id]);
  const streams = useStore((s) => s.streams);
  const run = useStore((s) => s.activeRun[agent.id]);
  const activity = useStore((s) => s.activity);
  const stuck = useStore((s) => s.stuck);
  const building = useStore((s) => s.building);
  const trail = useStore((s) => s.trail);
  const lastActive = useStore((s) => s.lastActive);
  const finishedAt = useStore((s) => s.finishedAt);
  const now = useNow();

  const [draft, setDraftState] = useState(() => drafts.get(agent.id) ?? "");
  const setDraft = (text: string) => {
    setDraftState(text);
    if (text) drafts.set(agent.id, text);
    else drafts.delete(agent.id);
  };
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const field = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setError(null);
    loadChannel(agent.id).catch((cause) => setError(errorMessage(cause)));
    field.current?.focus();
  }, [agent.id, loadChannel]);

  // The panel is hidden rather than closed, so the field it was hidden with is
  // the field it comes back with, and the keyboard should be in it.
  useEffect(() => {
    const shown = () => field.current?.focus();
    window.addEventListener("focus", shown);
    return () => window.removeEventListener("focus", shown);
  }, []);

  const recent = exchange(messages, agent.id, RECENT);
  const live = liveReply(streams, agent.id);
  const label = doing(agent.id, { activity, stuck, building, trail, lastActive }, now);

  // The newest line in view, by the transcript's own rule: followed for
  // whoever is at the end, and left alone for whoever scrolled up to read.
  // Attached to the panel's body only while a conversation is open, because
  // the list under it is read from the top.
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
      // The draft is kept. A message that failed to send and took the words
      // with it is a message the operator has to write twice.
      setError(errorMessage(cause));
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends and Shift+Enter is a new line, as in the window's composer.
    // Not while an input method is composing: that Enter picks a character.
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
        <button type="button" className="btn btn--ghost btn--small" onClick={onBack}>
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
            className="btn btn--ghost btn--small"
            onClick={() => void api.stopRun(run).catch((cause) => setError(errorMessage(cause)))}
          >
            Stop
          </button>
        )}
        <button
          type="button"
          className="btn btn--small"
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
          className="btn btn--primary btn--small"
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

/**
 * A message cut to the height a panel can spare, and a way to the rest of it.
 *
 * Measured rather than assumed: a short reply is drawn whole with no fade and
 * no link, because a "read the rest" under a message that has no rest is a
 * promise the window cannot keep.
 */
function Clamped({ children, onMore }: { children: ReactNode; onMore: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [clipped, setClipped] = useState(false);

  // Once, rather than on every render: a reply being written re-renders on
  // every token, and the body it is written into is one element whose size is
  // what changes. Observing that element is what catches it.
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

/**
 * Keeps the window exactly as tall as what it is showing.
 *
 * The panel's height is its content's, up to what the screen under the icon
 * can hold: a quiet workspace is a short panel rather than a tall one with a
 * sentence at the top. What the content wants is the chrome around the body
 * plus the body's own content, which is measured inside the scrolling box so
 * that a panel already at its ceiling still knows how much taller it would be.
 * Rust clamps it and keeps the top edge under the icon.
 *
 * Measured on every commit as well as on a resize, and that is not
 * belt-and-braces. The panel spends most of its life hidden, and a hidden
 * window draws nothing, so nothing observes its size change and the height it
 * would open at is whatever was measured before it was hidden. A commit still
 * lays the page out when it is asked to, so the height is current by the time
 * the icon is clicked.
 */
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
