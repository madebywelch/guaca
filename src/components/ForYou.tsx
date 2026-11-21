import { useEffect, useRef, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { ABSENT_ACCENT } from "../avatars/catalog";
import { decisionTime, needsAnswer, orderedDecisions } from "../lib/decisions";
import { useDismiss } from "../lib/dismiss";
import { useShown } from "../lib/held";
import { api } from "../lib/ipc";
import {
  drawnThrough,
  EARLIER_LIMIT,
  EARLIER_MS,
  earlier,
  finished,
  forYouSummary,
  type Shelf,
  shelves,
  statusWord,
} from "../lib/outcomes";
import { askers, DELETED, questions } from "../lib/questions";
import { together } from "../lib/running";
import { useStore } from "../lib/store";
import { relativeTime, useNow } from "../lib/time";
import { type AgentCard, errorMessage, type Outcome, type WorkDecision } from "../lib/types";
import { Desk } from "./Desk";
import { Scrim } from "./Scrim";

export function ForYou({ onClose }: { onClose: () => void }) {
  const decisions = useStore((state) => state.decisions);
  const refresh = useStore((state) => state.refreshDecisions);
  const error = useStore((state) => state.decisionError);
  const banner = useStore((state) => state.banner);
  const { pending, stuck } = useShown();
  const agents = useStore((state) => state.agents);
  const groups = useStore((state) => state.groups);
  const lastActive = useStore((state) => state.lastActive);
  const select = useStore((state) => state.select);
  const openMessage = useStore((state) => state.openMessage);
  const outcomes = useStore((state) => state.outcomes);
  const refreshOutcomes = useStore((state) => state.refreshOutcomes);
  const seeOutcomes = useStore((state) => state.seeOutcomes);
  const now = useNow(30_000);
  const panel = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<"needs" | "following" | "history">("needs");
  const asked = useStore((state) => state.forYouAsked);
  const [answered, setAnswered] = useState(asked);
  if (asked !== answered) {
    setAnswered(asked);
    setView("needs");
  }
  const [receipt, setReceipt] = useState("");
  const [recent, setRecent] = useState<Outcome[]>([]);
  useDismiss(onClose);
  useEffect(() => {
    panel.current?.focus();
    void refresh();
    void refreshOutcomes();
  }, [refresh, refreshOutcomes]);
  useEffect(() => {
    let current = true;
    void (async () => {
      try {
        const rows = await api.listOutcomes({
          since: Date.now() - EARLIER_MS,
          limit: EARLIER_LIMIT,
        });
        if (current) setRecent(rows);
      } catch (error) {
        console.warn(`Earlier could not be read: ${errorMessage(error)}`);
      }
    })();
    return () => {
      current = false;
    };
  }, [outcomes]);
  const news = finished(outcomes);
  const past = earlier(recent, now);
  const openOutcome = (outcome: Outcome) => {
    if (outcome.seenAt === null) void seeOutcomes({ runIds: [outcome.runId] });
    if (outcome.noteId) void openMessage(outcome.agentId, outcome.noteId, "result");
    else void select(outcome.agentId);
    onClose();
  };
  const open = orderedDecisions(decisions.filter(needsAnswer));
  const following = decisions.filter((item) => item.status === "answered" && !item.interrupted);
  const history = decisions
    .filter((item) => item.status === "completed" || item.status === "withdrawn")
    .sort((a, b) => b.updatedAt - a.updatedAt);
  const snoozed = open.filter((item) => item.snoozedUntil !== null && item.snoozedUntil > now);
  const due = open.filter((item) => !snoozed.includes(item));
  const shown = view === "needs" ? due : view === "following" ? following : history;
  const count = open.length + pending.length + stuck.length;
  const waiting = askers(
    [...pending, ...stuck, ...open],
    (id) => agents.find((agent) => agent.id === id)?.name ?? DELETED,
  );
  return (
    <Scrim>
      <button type="button" className="scrim__close" aria-label="Close For you" onClick={onClose} />
      <div
        className="dialog for-you"
        role="dialog"
        aria-modal="true"
        aria-label="For you"
        tabIndex={-1}
        ref={panel}
        onKeyDown={(event) => {
          if (event.key !== "Tab") return;
          const focusable = Array.from(
            panel.current?.querySelectorAll<HTMLElement>(
              "button:not(:disabled), input:not(:disabled), summary, a[href]",
            ) ?? [],
          ).filter((el) => el.getClientRects().length > 0);
          const first = focusable[0];
          const last = focusable.at(-1);
          if (
            event.shiftKey &&
            (document.activeElement === first || document.activeElement === panel.current)
          ) {
            event.preventDefault();
            last?.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first?.focus();
          }
        }}
      >
        <header className="for-you__head">
          <div className="for-you__title">
            <h2>For you {count > 0 && <span>{questions(count)}</span>}</h2>
            <button type="button" className="btn btn--quiet" onClick={onClose}>
              Close
            </button>
          </div>
          <p>
            {forYouSummary(waiting, news.length)}{" "}
            {following.length > 0 ? `${following.length} in progress after your answer.` : ""}
          </p>
          <nav className="for-you__tabs" aria-label="For you views">
            <button
              type="button"
              className="btn"
              aria-pressed={view === "needs"}
              onClick={() => setView("needs")}
            >
              Waiting on you
            </button>
            <button
              type="button"
              className="btn"
              aria-pressed={view === "following"}
              onClick={() => setView("following")}
            >
              Answered
            </button>
            <button
              type="button"
              className="btn"
              aria-pressed={view === "history"}
              onClick={() => setView("history")}
            >
              History
            </button>
          </nav>
        </header>
        <div className="for-you__body">
          {banner?.tone === "error" && (
            <p role="alert" className="for-you__error">
              {banner.text}
              <button
                type="button"
                className="btn btn--compact"
                onClick={() => useStore.getState().setBanner(null)}
              >
                Dismiss
              </button>
            </p>
          )}
          {error && (
            <p role="alert">
              Could not refresh questions: {error}{" "}
              <button type="button" className="btn btn--compact" onClick={() => void refresh()}>
                Retry
              </button>
            </p>
          )}
          {receipt && (
            <p role="status" className="for-you__receipt">
              {receipt}
            </p>
          )}
          {view === "needs" && (
            <Desk
              title={null}
              onOpenChannel={(agent) => {
                void select(agent);
                onClose();
              }}
            />
          )}
          {shown.map((item) => (
            <DecisionCard
              key={item.id}
              item={item}
              now={now}
              onReceipt={(message) => {
                setReceipt(message);
                panel.current?.focus();
              }}
              onClose={onClose}
            />
          ))}
          {view === "needs" && snoozed.length > 0 && (
            <details className="for-you__later">
              <summary>Set aside · {snoozed.length}</summary>
              {snoozed.map((item) => (
                <DecisionCard
                  key={item.id}
                  item={item}
                  now={now}
                  onReceipt={(message) => {
                    setReceipt(message);
                    panel.current?.focus();
                  }}
                  onClose={onClose}
                />
              ))}
            </details>
          )}
          {shown.length === 0 && view !== "needs" && (
            <p className="for-you__empty">
              {view === "history"
                ? "Completed and withdrawn questions will appear here."
                : "Nothing you answered is still in progress."}
            </p>
          )}
          {view === "needs" && news.length > 0 && (
            <section className="for-you__finished" aria-labelledby="for-you-finished">
              <header className="for-you__finished-head">
                <h3 id="for-you-finished">Finished</h3>
                <button
                  type="button"
                  className="btn btn--quiet btn--compact"
                  onClick={() => {
                    const through = drawnThrough(news);
                    if (through !== null) void seeOutcomes({ before: through });
                  }}
                >
                  Mark all seen
                </button>
              </header>
              <FinishedShelves shelves={shelves(news, groups)} now={now} onOpen={openOutcome} />
            </section>
          )}
          {view === "needs" && past.length > 0 && (
            <details className="for-you__later">
              <summary>Earlier · {past.length}</summary>
              <FinishedShelves shelves={shelves(past, groups)} now={now} onOpen={openOutcome} />
            </details>
          )}
          {view === "needs" && (
            <details className="for-you__later">
              <summary>When each agent last worked</summary>
              <p className="for-you__meta">
                Last activity shows when an agent worked. It does not confirm that a particular
                inbox was checked.
              </p>
              {agents
                .filter((agent) => agent.lifecycle !== "terminated")
                .map((agent) => (
                  <p key={agent.id}>
                    {agent.name} ·{" "}
                    {lastActive[agent.id]
                      ? `last active ${relativeTime(lastActive[agent.id]!, now)}`
                      : "no activity recorded"}
                  </p>
                ))}
            </details>
          )}
        </div>
        <footer className="for-you__foot">
          Reminders about open questions at 9 AM and 4 PM, plus one hour before deadlines, in the
          workspace’s timezone. Notification preferences apply.
        </footer>
      </div>
    </Scrim>
  );
}

function FinishedShelves({
  shelves,
  now,
  onOpen,
}: {
  shelves: Shelf[];
  now: number;
  onOpen: (outcome: Outcome) => void;
}) {
  const agents = useStore((state) => state.agents);
  return shelves.map((shelf) => (
    <div key={shelf.group?.id ?? "all"} className="for-you__shelf">
      {shelf.group && <h4>{shelf.group.name}</h4>}
      {shelf.outcomes.map((outcome) => (
        <FinishedLine
          key={outcome.runId}
          outcome={outcome}
          agent={agents.find((agent) => agent.id === outcome.agentId)}
          now={now}
          onOpen={() => onOpen(outcome)}
        />
      ))}
    </div>
  ));
}

export function FinishedLine({
  outcome,
  agent,
  now,
  onOpen,
}: {
  outcome: Outcome;
  agent: AgentCard | undefined;
  now: number;
  onOpen: () => void;
}) {
  const word = statusWord(outcome.status);
  const age = relativeTime(outcome.finishedAt, now);
  const colleagues = useStore((state) =>
    together(
      (outcome.colleagues ?? []).flatMap(
        (id) => state.agents.find((one) => one.id === id)?.name ?? [],
      ),
    ),
  );
  return (
    <button type="button" className="finished-line" title={outcome.headline} onClick={onOpen}>
      <AgentAvatar
        avatar={agent?.avatar ?? "blank"}
        color={agent?.color ?? ABSENT_ACCENT}
        size="xs"
        lifecycle={agent?.lifecycle}
        seed={agent?.id ?? outcome.runId}
      />
      <span className="finished-line__text">
        <span className="finished-line__who">
          {agent?.name ?? "A deleted agent"}
          {colleagues && `, with ${colleagues}:`}
        </span>{" "}
        {outcome.headline}
      </span>
      <span className="finished-line__meta">{word ? `${word} ${age}` : age}</span>
    </button>
  );
}

function DecisionCard({
  item,
  now,
  onReceipt,
  onClose,
}: {
  item: WorkDecision;
  now: number;
  onReceipt: (message: string) => void;
  onClose: () => void;
}) {
  const agent = useStore((state) => state.agents.find((agent) => agent.id === item.agentId));
  const group = useStore((state) => state.groups.find((group) => group.id === item.groupId));
  const refresh = useStore((state) => state.refreshDecisions);
  const select = useStore((state) => state.select);
  const [busy, setBusy] = useState(false);
  const [written, setWritten] = useState("");
  const [writing, setWriting] = useState(item.request.options.length === 0);
  const [error, setError] = useState<string | null>(null);
  const act = async (operation: () => Promise<unknown>, receipt: string) => {
    setBusy(true);
    setError(null);
    try {
      await operation();
      onReceipt(receipt);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      await refresh();
      setBusy(false);
    }
  };
  const answer = (text: string) =>
    void act(
      () => api.answerDecision(item.id, text, item.updatedAt),
      `Answer saved for ${agent?.name ?? "the agent"}. Its progress is under Answered.`,
    );
  const waiting = item.status === "pending";
  return (
    <article className="for-you__item">
      <div className="for-you__meta">
        <span>
          {agent?.name ?? "Deleted agent"} · {group?.name ?? "Group"}
        </span>
        <span>{decisionTime(item.createdAt)}</span>
      </div>
      <h3>{item.request.question}</h3>
      {item.dueAt !== null && (
        <p className="for-you__due">
          {item.dueAt <= now ? "Deadline passed" : "Reply by"} · {decisionTime(item.dueAt)}
        </p>
      )}
      {item.request.context && <p>{item.request.context}</p>}
      {item.request.recommendation && (
        <p>
          <strong>Recommendation:</strong> {item.request.recommendation}
        </p>
      )}
      {item.answer !== null && (
        <p>
          <strong>Your answer:</strong> {item.answer}
        </p>
      )}
      {item.outcome && (
        <p className="for-you__receipt">
          <strong>{item.status === "withdrawn" ? "Withdrawn:" : "Completed:"}</strong>{" "}
          {item.outcome} · {decisionTime(item.updatedAt)}
        </p>
      )}
      {item.status === "answered" && (
        <p className="for-you__meta">
          {item.interrupted
            ? "Work after your answer was interrupted. Review what was already done before resuming."
            : "Answer delivered. Awaiting a completion receipt from the agent."}
        </p>
      )}
      {item.snoozedUntil !== null && item.snoozedUntil > now && (
        <p className="for-you__meta">
          Returns {decisionTime(item.snoozedUntil)}
          {item.dueAt !== null && item.snoozedUntil > item.dueAt
            ? " · This is after the deadline."
            : ""}
        </p>
      )}
      {item.request.source && (
        <details className="for-you__source">
          <summary>Source</summary>
          <p>{item.request.source}</p>
        </details>
      )}
      {error && <p role="alert">{error}</p>}
      <div className="for-you__actions">
        {waiting &&
          item.request.options.map((option) => (
            <button
              key={option}
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => answer(option)}
            >
              {option}
            </button>
          ))}
        {waiting && !writing && (
          <button
            type="button"
            className="btn btn--quiet"
            disabled={busy}
            onClick={() => setWriting(true)}
          >
            Write an answer
          </button>
        )}
        {item.status === "answered" && item.interrupted && (
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy}
            onClick={() =>
              void act(
                () => api.resumeDecision(item.id),
                "Resumed. The agent checks what was already done before going on.",
              )
            }
          >
            Resume work
          </button>
        )}
        {needsAnswer(item) && (
          <button
            type="button"
            className="btn btn--quiet"
            disabled={busy}
            onClick={() =>
              void act(
                () => api.snoozeDecision(item.id, Date.now() + 60 * 60 * 1000),
                "Set aside for one hour. The question still needs your answer.",
              )
            }
          >
            Remind me in 1 hour
          </button>
        )}
        <button
          type="button"
          className="btn btn--quiet"
          onClick={() => {
            void select(item.agentId);
            onClose();
          }}
        >
          Open conversation
        </button>
      </div>
      {waiting && writing && (
        <form
          className="for-you__actions"
          onSubmit={(event) => {
            event.preventDefault();
            if (written.trim()) answer(written.trim());
          }}
        >
          <input
            className="field"
            aria-label="Your answer"
            placeholder="Your answer"
            value={written}
            maxLength={4000}
            disabled={busy}
            onChange={(event) => setWritten(event.target.value)}
          />
          <button type="submit" className="btn btn--primary" disabled={busy || !written.trim()}>
            Save answer
          </button>
        </form>
      )}
    </article>
  );
}
