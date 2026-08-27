import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { useFollowBottom } from "../lib/follow";
import { api } from "../lib/ipc";
import { thoughtNow } from "../lib/reasoning";
import { ACTIVITY_CHANNEL, type ChannelKey, useAgentLookup, useStore } from "../lib/store";
import { elapsed, useNow } from "../lib/time";
import {
  callInFlight,
  type LiveCall,
  type Step,
  tallyLabel,
  tallyTrail,
  trailStep,
} from "../lib/trail";
import { rowStandsFor, toPeer, transcriptRows } from "../lib/transcript";
import {
  type Activity,
  type AgentCard,
  type AgentId,
  type Envelope,
  errorMessage,
  plainText,
} from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";
import { CodingPanel } from "./CodingPanel";
import { Composer } from "./Composer";
import { Answering } from "./HtmlArtifact";
import { MessageItem, StreamingMessage, WhenRow } from "./MessageItem";
import { PairThread } from "./PairThread";
import { TrailRow } from "./Trail";
import { PeerBurstRow, RefusedRow, WritingRow } from "./WireRow";

interface Props {
  channel: ChannelKey;
  /** Where the operator asked for an agent's actions, and on whom. */
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
}

/** How long a message arrived at from search stays marked. */
const FLASH_MS = 1800;

/**
 * How long a tool call runs before the line above the composer names it.
 *
 * A `directory` lookup answers in milliseconds, and a line that flashed
 * "Checking who is available" for each of them would put back the flicker the
 * sentence rule takes out. What this line is for is the wait an operator
 * notices, so a call has to become one before it earns the line.
 */
const WAITED_MS = 1000;

export function ChannelView({ channel, onOpenMenu }: Props) {
  const lookups = useAgentLookup();
  const messages = useStore((s) => s.messages[channel]);
  const activity = useStore((s) => s.activity);
  const setBanner = useStore((s) => s.setBanner);
  const focused = useStore((s) => s.focused);
  const clearFocus = useStore((s) => s.clearFocus);

  /** The peer whose thread is open over this channel, if any. */
  const [reading, setReading] = useState<AgentId | null>(null);

  // Only ever moves the transcript for somebody who is already watching its
  // newest line. Yanking the view while they are reading back through a
  // cascade is worse than a scrollbar that does not move: `lib/follow.ts`.
  const { ref: scrollRef, node: transcript, follow, pin } = useFollowBottom();

  const isActivity = channel === ACTIVITY_CHANNEL;
  const agent = isActivity ? undefined : lookups.byId(channel);
  // Held steady across renders: it is the value of a context read by every page
  // in the transcript, and a fresh object each time would redraw all of them
  // for every token that lands anywhere.
  const answeringId = agent?.id;
  const answeringName = agent?.name;
  const answering = useMemo(
    () => (answeringId && answeringName ? { id: answeringId, name: answeringName } : null),
    [answeringId, answeringName],
  );

  useLayoutEffect(follow, [follow, messages]);

  // A channel switch abandons any thread opened off the old one: a conversation
  // between two other agents is not what you asked for by clicking a third.
  useLayoutEffect(() => {
    setReading(null);
  }, [channel]);

  // The newest message, whether the transcript is being opened or coming back
  // from a thread. Coming back to where you were is not on offer: the
  // transcript was unmounted, so there is no scroll position to come back to,
  // and the top of the history is the one place it must not land.
  useLayoutEffect(pin, [pin, channel, reading]);

  // Built once per set of messages rather than per render: it walks every
  // message, and it is what decides which of them are drawn at all.
  const rows = useMemo(() => transcriptRows(messages ?? [], lookups), [messages, lookups]);

  // A message somebody arrived at from search. Run after the transcript is on
  // screen, because the row cannot be scrolled to before it exists, and the
  // read that widens the window to reach it lands a render later than the
  // channel switch does. The mark is cleared once it has been shown, so
  // reopening the channel later does not jump again.
  //
  // Matched against a list rather than a single id, because a row does not
  // stand for exactly one message: a burst stands for every message it
  // collapsed, and a message found inside one is read by opening the thread
  // behind the row rather than in the channel.
  useEffect(() => {
    if (!focused || messages === undefined) return;
    const row = transcript()?.querySelector<HTMLElement>(`[data-message~="${focused}"]`);
    if (!row) return;
    // Nothing here says to stop following. Coming up the transcript to reach
    // the row is a scroll like any other, and releases the bottom exactly as
    // the operator's own would; a row that was already at the end moves
    // nothing, and following the end is what keeps it in view.
    row.scrollIntoView({ block: "center" });
    const timer = window.setTimeout(clearFocus, FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [focused, messages, clearFocus, transcript]);

  const paused = agent?.lifecycle === "paused";

  if (reading && agent) {
    return (
      <PairThread
        self={agent.id}
        peer={reading}
        lookups={lookups}
        onClose={() => setReading(null)}
      />
    );
  }

  return (
    <section className="pane">
      <header className="pane__header">
        {isActivity ? (
          <>
            <span
              aria-hidden="true"
              style={{ color: "var(--muted)", fontFamily: "var(--font-mono)" }}
            >
              #
            </span>
            <h1 className="pane__title">activity</h1>
            <p className="pane__subtitle">
              Who spoke to whom, in order. Click any arrow to read the message.
            </p>
          </>
        ) : agent ? (
          <>
            <AgentAvatar
              avatar={agent.avatar}
              color={agent.color}
              activity={activity[agent.id]}
              lifecycle={agent.lifecycle}
              seed={agent.id}
              size="sm"
            />
            <h1 className="pane__title">{agent.name}</h1>
            {/* The one thing about an agent that changes what this pane does.
                Everything else it is set up with is edited rarely and read
                behind the menu, rather than sitting over every message. */}
            {paused && <span className="pane__flag">Paused</span>}
            <button
              type="button"
              className="pane__more"
              aria-label={`Actions for ${agent.name}`}
              title={`Actions for ${agent.name}`}
              onClick={(event) => {
                // Under the button it came from. The menu measures itself and
                // slides back inside the window if it does not fit there.
                const box = event.currentTarget.getBoundingClientRect();
                onOpenMenu(agent, { x: box.left, y: box.bottom + 4 });
              }}
            >
              ⋯
            </button>
          </>
        ) : (
          <h1 className="pane__title">Channel unavailable</h1>
        )}
      </header>

      {isActivity ? (
        // A list cannot show who spoke to whom, so the activity view is a
        // board rather than a transcript.
        <ActivityFlow messages={messages ?? []} byId={lookups.byId} />
      ) : (
        // A log rather than a plain box, so a screen reader offers it as the
        // transcript it is. `aria-live` is off deliberately: the role would
        // otherwise announce politely, and opening a channel replaces three
        // hundred rows at once, which is the whole history read aloud for the
        // crime of clicking a name. What is worth hearing is announced by
        // `Arrivals` below, one message at a time, as it lands.
        <div
          className="pane__scroll"
          ref={scrollRef}
          role="log"
          aria-live="off"
          aria-label={`Conversation with ${agent?.name ?? "this agent"}`}
        >
          {/* Who a page in this transcript may answer. Only here: the operator
              is one of the two participants in a channel, so a value handed
              back has an obvious next message and an obvious recipient. */}
          <Answering.Provider value={answering}>
            {messages === undefined ? (
              <p className="hint" style={{ padding: "1rem 1.15rem" }}>
                Loading…
              </p>
            ) : messages.length === 0 ? (
              <div className="empty">
                <p className="empty__body">No messages with {agent?.name ?? "this agent"} yet.</p>
              </div>
            ) : (
              rows.map((row) => {
                const stands = rowStandsFor(row);
                return (
                  // Wrapped rather than marked on the entry itself: a row renders
                  // as several different shapes depending on who sent what to
                  // whom, and one of them is several rows.
                  <div
                    key={row.key}
                    data-message={stands.join(" ")}
                    data-found={focused && stands.includes(focused) ? "true" : undefined}
                  >
                    {row.kind === "peers" ? (
                      <PeerBurstRow peers={row.peers} onOpen={setReading} />
                    ) : row.kind === "refused" ? (
                      <RefusedRow peer={row.peer} at={row.at} body={row.body} reason={row.reason} />
                    ) : row.kind === "when" ? (
                      <WhenRow at={row.at} />
                    ) : (
                      // Two participants, one of them named at the top of the
                      // pane and the other reading this. Nothing here needs
                      // telling whose words it is looking at.
                      <MessageItem
                        message={row.message}
                        lookups={lookups}
                        continued={row.continued}
                        named={false}
                      />
                    )}
                  </div>
                );
              })
            )}

            <LiveStreams channel={channel} lookups={lookups} follow={follow} />
          </Answering.Provider>
        </div>
      )}

      {isActivity ? null : (
        <>
          {/* Outside the log on purpose. Inside it the sentence is a second
              copy of the newest message for anybody reading the transcript
              itself, which is the cost of announcing it to everybody else. */}
          <Arrivals channel={channel} messages={messages} lookups={lookups} />
          {/* Above the line about the turn, because a coding job outlives the
              turn that started it: the turn footer is empty and the agent is
              idle while this is running. Keyed by agent so switching channels
              does not carry one job's tail into another's panel. */}
          {agent && <CodingPanel key={`coding-${agent.id}`} agent={agent.id} />}
          {/* Keyed by agent, so the disclosure below does not arrive already
              open on a channel the operator has just switched to. */}
          {agent && <TurnFooter key={agent.id} agent={agent} state={activity[agent.id]} />}
          <Composer
            placeholder={`Message ${agent?.name ?? "agent"}`}
            group={agent?.groupId ?? null}
            disabled={!agent || agent.lifecycle === "terminated"}
            disabledReason="This agent has been deleted."
            onSend={async (text, files) => {
              if (!agent) return;
              // Typing into the box is a decision to be at the end of the
              // transcript, so this is the one thing besides opening a channel
              // that overrides where the operator was reading. Their own
              // message landing off screen, with nothing following it, is the
              // same bug in the other direction.
              pin();
              try {
                await api.sendMessage(agent.id, text, files);
              } catch (error) {
                setBanner({ tone: "error", text: errorMessage(error) });
                throw error;
              }
            }}
          />
        </>
      )}
    </section>
  );
}

/**
 * Everything about the turn that is still going, between the transcript and the
 * box you would type into.
 *
 * **One line, and one slot above it.** The line says what the turn is doing
 * now, what it has done and what it is spending, and it is the same height from
 * the first token to the last. Behind it are two things worth a longer look —
 * the working the model has published, and the calls it has already made — and
 * they share one place to appear, so opening either moves the transcript by the
 * same amount and opening the second does not stack on the first.
 *
 * It was three layers deep instead, and the middle one was the whole record of
 * the turn drawn as it accumulated: a browsing turn's seven kinds of work
 * wrapped across four rows of gray monospace, reflowing every time a call came
 * back, with the composer moving underneath. That record is not lost by this
 * and was never only here — the transcript draws every chip of it the moment
 * the turn ends, from the same rules over the same values.
 *
 * Still three components rather than one, because they change at wildly
 * different rates. The line moves sixty times a second and the chips a few
 * times a minute, and a token arriving must not re-render a chip for the same
 * reason it must not re-render the transcript.
 *
 * This one holds only which of the two is open, which is the whole reason it
 * exists: both buttons are on the line and the panel is above it.
 */
function TurnFooter({ agent, state }: { agent: AgentCard; state: Activity | undefined }) {
  const [showing, setShowing] = useState<Showing | null>(null);
  const panel = useId();

  // Queued counts: the agent has work it has not read yet, and to the operator
  // that is the same thing as working. Awaiting approval does not: it is
  // waiting on a person, and the request itself is in this channel saying so.
  const working = state?.state === "thinking" || state?.state === "queued";

  // A turn ending closes it. Asking to watch one turn's working is not a
  // standing decision to watch every turn's, and a panel that opened by itself
  // on the next one is the app deciding where the operator looks.
  useEffect(() => {
    if (!working) setShowing(null);
  }, [working]);

  if (!working) return null;

  return (
    <div className="turn">
      {showing === "working" && <ThoughtPanel id={panel} agent={agent.id} />}
      {showing === "steps" && <StepsPanel id={panel} agent={agent.id} />}
      <WorkingNote
        agent={agent}
        panel={panel}
        showing={showing}
        onToggle={(which) => setShowing((held) => (held === which ? null : which))}
      />
    </div>
  );
}

/** Which of the line's two disclosures is in the slot above it, if either. */
type Showing = "working" | "steps";

/**
 * The whole of what the model has published this turn, when it is asked for.
 *
 * The line below is a peek, and a peek is enough right up until the wait runs
 * to ten minutes. Then the question stops being "is it alive" and becomes "is
 * it doing something sensible", which cannot be answered one sentence at a
 * time. So the line opens, and what it opens into is everything the model has
 * said about this turn.
 *
 * Nothing about it is kept any longer than the line is: it is the same slice,
 * dropped by the same event, and the panel is unmounted with the turn.
 *
 * Drawn as the model wrote it, marks and all. This is the one surface in the
 * app that is not something anybody said, and formatting it would make it look
 * like something somebody did. It is also why it is one text node in a `pre`:
 * a document re-parsed as markdown sixty times a second is exactly the cost the
 * reasoning slice was split out of the stream buffer to avoid.
 */
function ThoughtPanel({ id, agent }: { id: string; agent: AgentId }) {
  const held = useStore((s) => s.reasoning[agent]);
  // Follows the end for whoever is at the end, exactly as the transcript does
  // and for the same reason: an operator reading back through a minute of
  // working must not be thrown to the floor by the next token.
  const { ref, follow } = useFollowBottom();

  useLayoutEffect(follow, [follow, held]);

  return (
    <div className="thought" id={id} ref={ref}>
      {held ? (
        <pre className="thought__text">{held}</pre>
      ) : (
        // Reachable: a model call that fails is retried under a new placeholder
        // and the thinking of the attempt that broke goes with it, which can
        // happen while this is open.
        <p className="thought__empty">Nothing published yet on this turn.</p>
      )}
    </div>
  );
}

/**
 * A turn's finished calls, as the row draws them.
 *
 * A call still in flight is not one of them: it has no outcome to draw a chip
 * from, and it is what the line above the composer is for.
 */
function liveSteps(calls: LiveCall[] | undefined): Step[] {
  return (calls ?? []).flatMap((call, index) =>
    call.done ? [trailStep(call.done, `${call.callId}-${index}`)] : [],
  );
}

/**
 * What this turn has already done, when the operator asks for it.
 *
 * The same chips the transcript will draw once the turn ends, from the same
 * rules, built from the calls the runtime reports as it makes them. Until this
 * existed, a turn's tool calls were invisible for as long as the turn ran: a
 * browsing turn spends most of its twenty-four rounds in the browser, and the
 * operator watching it had one line of prose and no way to tell work from a
 * hang. Prose is what a model says it is doing. This is what it did.
 *
 * Behind a click rather than in front of one, which is the one thing that
 * changed: sitting open it is the whole of a long turn's record wrapped across
 * four rows and reflowing under the composer, and what it answers — "is this
 * doing something sensible" — is a question asked a few times in ten minutes
 * rather than continuously. The count that says there is something to ask about
 * is on the line, and it never wraps.
 */
function StepsPanel({ id, agent }: { id: string; agent: AgentId }) {
  const calls = useStore((s) => s.trail[agent]);
  const steps = liveSteps(calls);
  // Follows the end exactly as the working does and for the same reason: an
  // operator reading back through what a turn has done must not be thrown to
  // the floor by the next call coming back.
  const { ref, follow } = useFollowBottom();

  useLayoutEffect(follow, [follow, steps.length]);

  return (
    <div className="steps" id={id} ref={ref}>
      <TrailRow steps={steps} />
    </div>
  );
}

/**
 * That the agent is still going, above the box you would type into, and what it
 * is doing while it goes.
 *
 * The sidebar already says "typing" beside the name, but the operator watching
 * a channel is looking at the bottom of it, waiting. A silent gap between
 * sending and the first token is the moment the app looks broken, and it is a
 * long one: a turn can spend several model calls on tool results before a word
 * is written for anybody to read. A pulse says the turn is alive; it does not
 * say what it is doing, and those are different questions when the wait is ten
 * minutes.
 *
 * So the line says the most specific true thing available, in this order:
 *
 * - **A call that has not come back.** While a command runs or a page loads,
 *   the model is not thinking and its last thought is frozen and stale, which
 *   is the state that reads as a hang. Naming the call and counting the seconds
 *   is the difference between waiting and wondering.
 * - **What it is working through**, as the model's own section heading and the
 *   last sentence it finished under it. `lib/reasoning.ts` decides both, and
 *   the whole of the working is one click away.
 * - **That it is working**, which is all there is to say where the provider
 *   publishes nothing. Revealed on hover, because whose channel this is has
 *   been established four times over by the time you reach the bottom of it. It
 *   stays in the accessibility tree either way, which is why that is opacity
 *   rather than a mount.
 *
 * At the end of it are the three things that are not the sentence: what the
 * turn has spent, how much it has done, and the one control that stops it. The
 * count is the whole of the trail while the turn runs and opens into the rest
 * of it; the credentials are named rather than counted, because a name is the
 * operator's audit trail for their own tokens and is not a thing to put behind
 * a click; and Stop is always drawn, because it is the one thing on screen that
 * costs money to leave alone.
 *
 * Subscribed here rather than in the parent for the same reason `LiveStreams`
 * is: what it draws changes every sixteen milliseconds and everything around it
 * does not.
 */
function WorkingNote({
  agent,
  panel,
  showing,
  onToggle,
}: {
  agent: AgentCard;
  /** The one slot this line's two buttons open into. */
  panel: string;
  /** Which of them is open, if either. */
  showing: Showing | null;
  onToggle: (which: Showing) => void;
}) {
  const held = useStore((s) => s.reasoning[agent.id]);
  const calls = useStore((s) => s.trail[agent.id]);
  // One string, so this re-renders when the run changes and not when a token
  // arrives. Subscribing to `streams` here to find it would undo the whole
  // reason this component exists.
  const run = useStore((s) => s.activeRun[agent.id]);
  const setBanner = useStore((s) => s.setBanner);
  const [stopping, setStopping] = useState(false);
  const now = useNow(1000);

  const thought = thoughtNow(held);
  // Held against the trail rather than against the render: this component
  // re-renders on every token of the model's working, and a tally rebuilt
  // sixty times a second walks two dozen calls to arrive at the same number.
  const tally = useMemo(() => tallyTrail(liveSteps(calls)), [calls]);
  // At most one: a turn makes its calls one at a time and waits for each.
  const waiting = calls?.find((call) => call.done === null && now - call.startedAt >= WAITED_MS);
  const saying = Boolean(waiting) || Boolean(thought.heading || thought.line);

  const line = waiting ? (
    <>
      <span className="working__now">{callInFlight(waiting.name, waiting.arguments)}</span>
      <span className="working__for">{elapsed(now, waiting.startedAt)}</span>
    </>
  ) : saying ? (
    <>
      {thought.heading && <span className="working__topic">{thought.heading}</span>}
      {thought.line && <span className="working__thought">{thought.line}</span>}
    </>
  ) : (
    `${agent.name} is working`
  );

  return (
    // A sentence that appears once is a status worth announcing; a line that
    // moves under it is not. So this is a live region for exactly as long as it
    // holds the sentence, which is from the moment the turn starts until the
    // model publishes its first thought or reaches for its first tool.
    <div
      className="working"
      role={saying ? undefined : "status"}
      data-thinking={saying || undefined}
    >
      <AgentAvatar
        avatar={agent.avatar}
        color={agent.color}
        size="sm"
        seed={agent.id}
        activity={{ state: "thinking" }}
        title={`${agent.name} is working`}
      />
      {held ? (
        // Only where there is something to open. A control that does nothing is
        // one the operator learns to distrust, and a model that publishes no
        // working is a turn with nothing behind this line.
        <button
          type="button"
          className="working__label working__label--open"
          aria-expanded={showing === "working"}
          aria-controls={showing === "working" ? panel : undefined}
          aria-label={`${showing === "working" ? "Hide" : "Show"} what ${agent.name} is working through`}
          onClick={() => onToggle("working")}
        >
          {line}
        </button>
      ) : (
        <span className="working__label">{line}</span>
      )}
      <div className="working__end">
        {/* Named, never counted, and never behind the click beside it. The
            value is not here and there is no field it could arrive in. */}
        {tally.spent.map((credential) => (
          <span className="trail__spent" key={credential}>
            {credential}
          </span>
        ))}
        {tally.done > 0 && (
          // Drawn as one of the chips it opens, so the control and its contents
          // read as one thing. Its own text is its name: "12 steps, 1 failed"
          // is what somebody would say out loud about the button.
          <button
            type="button"
            className="working__steps"
            data-failed={tally.failed > 0 || undefined}
            aria-expanded={showing === "steps"}
            aria-controls={showing === "steps" ? panel : undefined}
            title={`What ${agent.name} has done on this turn`}
            onClick={() => onToggle("steps")}
          >
            <span className="working__caret" aria-hidden="true">
              {showing === "steps" ? "▾" : "▸"}
            </span>
            {tallyLabel(tally)}
          </button>
        )}
        {run && (
          // Stops the conversation, not the agent. A message that reached four
          // agents is one run, and stopping only the one on screen would leave
          // the other three working on the operator's bill: hence the wording,
          // which is the same distinction the button actually makes.
          <button
            type="button"
            className="btn btn--ghost btn--small"
            disabled={stopping}
            title="Stop this conversation and everything it set off"
            onClick={async () => {
              setStopping(true);
              try {
                // False means it had already finished, which needs no telling:
                // the answer is on screen.
                await api.stopRun(run);
              } catch (error) {
                setBanner({ tone: "error", text: errorMessage(error) });
              } finally {
                setStopping(false);
              }
            }}
          >
            Stop
          </button>
        )}
      </div>
    </div>
  );
}

/**
 * The bubbles that are still being written, and nothing else.
 *
 * Its own component because it is the only thing on screen that changes while
 * text arrives. Subscribing here rather than in the parent means a token
 * re-renders two lines instead of the whole transcript above them: with thirty
 * messages loaded that was six thousand renders for one reply, and the window
 * froze with five agents working at once. It also means a token written in
 * another agent's channel costs this one nothing.
 */
function LiveStreams({
  channel,
  lookups,
  follow,
}: {
  channel: ChannelKey;
  lookups: ReturnType<typeof useAgentLookup>;
  follow: () => void;
}) {
  const streams = useStore((s) => s.streams);
  const live = Object.entries(streams).filter(([, buffer]) => buffer?.channelId === channel);

  // Keeps the newest text in view without the parent re-rendering to notice.
  useLayoutEffect(follow);

  return (
    // Never announced. A live region reading a bubble as it is typed says the
    // same sentence a dozen times before it is finished; the finished message
    // is announced once, by `Arrivals`, and "is working" by `WorkingNote`.
    <div aria-live="off">
      {live.map(([id, buffer]) => {
        if (!buffer) return null;

        // A message bound for a peer will settle into a collapsed row, so it is
        // announced rather than streamed. Only text meant for the operator is
        // worth watching arrive. The writer is the peer: this stream is filed
        // in the recipient's channel, which is the one being looked at.
        if (buffer.to.kind === "agent") {
          return (
            <WritingRow key={id} peer={toPeer(lookups.byId(buffer.agentId), buffer.agentId)} />
          );
        }
        return buffer.text.length > 0 ? (
          <StreamingMessage key={id} agent={lookups.byId(buffer.agentId)} text={buffer.text} />
        ) : null;
      })}
    </div>
  );
}

/**
 * The one thing a transcript says out loud.
 *
 * Waiting for a reply is the whole shape of using this app, and a reader who
 * cannot see the column arrive has no way of knowing one did. So a message
 * addressed to the operator is announced once, when it settles, and nothing
 * else is: what is announced is exactly what is drawn as a full bubble.
 *
 * That symmetry is the rule, and it is the same one the channel is arranged by.
 * Peer traffic and tool trails are collapsed on screen precisely because
 * reading them line by line buries the conversation; read aloud they would bury
 * it far more thoroughly, since there is no glancing past a sentence being
 * spoken.
 *
 * Its own component so a message landing in another agent's channel costs this
 * one nothing, and so the sentence is built where it is announced rather than
 * threaded down from the pane.
 */
function Arrivals({
  channel,
  messages,
  lookups,
}: {
  channel: ChannelKey;
  messages: Envelope[] | undefined;
  lookups: ReturnType<typeof useAgentLookup>;
}) {
  const [said, setSaid] = useState("");
  /** The newest message already accounted for. Null until the channel is read. */
  const seen = useRef<string | null>(null);

  // Declared before the one below so it runs first on the render that changes
  // both: a channel switch has to forget what it had seen before that render
  // decides whether the newest message is news.
  useEffect(() => {
    seen.current = null;
    setSaid("");
  }, [channel]);

  useEffect(() => {
    const newest = messages?.[messages.length - 1];
    if (!newest) return;
    // Opening a channel is not an arrival. Everything in it was already there.
    if (seen.current === null || seen.current === newest.id) {
      seen.current = newest.id;
      return;
    }
    seen.current = newest.id;
    setSaid(announcement(newest, lookups.byId));
  }, [messages, lookups]);

  return (
    // Explicit rather than left to the role: the transcript above declares
    // `aria-live="off"`, live-region settings are inherited, and an implicit
    // value from `role="status"` is a weaker claim than a stated one.
    <p className="offscreen" role="status" aria-live="polite" aria-atomic="true">
      {said}
    </p>
  );
}

/** What an arriving message is worth saying, or nothing. */
function announcement(message: Envelope, byId: ReturnType<typeof useAgentLookup>["byId"]): string {
  if (message.from.kind !== "agent") return "";
  const who = byId(message.from.id)?.name ?? "An agent";

  // Ahead of the text, as everywhere else: this is the one thing in a
  // transcript the operator is expected to act on rather than read.
  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) return `${who} is asking you: ${asking.summary}`;

  // Addressed to a peer, so it is a collapsed row here rather than a bubble.
  if (message.to.kind !== "human") return "";

  const body = plainText(message);
  if (!body) return "";
  // Long enough to be the message, short enough that a reader can interrupt it
  // and go and read the column instead.
  return body.length > 300 ? `${who}: ${body.slice(0, 300)}…` : `${who}: ${body}`;
}
