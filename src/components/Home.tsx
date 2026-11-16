import { useEffect, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { needsAnswer } from "../lib/decisions";
import { api } from "../lib/ipc";
import { drawnThrough, finished, statusWord } from "../lib/outcomes";
import { useStore } from "../lib/store";
import { clockLabel, useNow } from "../lib/time";
import { errorMessage, type Outcome } from "../lib/types";
import { Desk } from "./Desk";
import { Field } from "./Field";

/** How many filed decisions are drawn as cards before the rest are a count. */
const FILED_SHOWN = 3;

interface Props {
  onSearch: (query: string) => void;
  onManageGroups: () => void;
}

/**
 * The opening screen, in the order a person opens the window for: the field,
 * then what needs them, answered where it is drawn, then what finished while
 * they were away. With none of those it is one sentence, which is the screen
 * seen most and the one that has to be calm.
 */
export function Home({ onSearch, onManageGroups }: Props) {
  const pending = useStore((state) => state.pending);
  const stuck = useStore((state) => state.stuck);
  const decisions = useStore((state) => state.decisions);
  const outcomes = useStore((state) => state.outcomes);
  const agents = useStore((state) => state.agents);
  const groups = useStore((state) => state.groups);
  const activity = useStore((state) => state.activity);
  const select = useStore((state) => state.select);
  const openMessage = useStore((state) => state.openMessage);
  const seeOutcomes = useStore((state) => state.seeOutcomes);
  const refreshOutcomes = useStore((state) => state.refreshOutcomes);
  const showForYou = useStore((state) => state.showForYou);
  const now = useNow(30_000);

  useEffect(() => {
    void refreshOutcomes();
  }, [refreshOutcomes]);

  const news = finished(outcomes);
  const open = decisions.filter(needsAnswer);
  const due = open.filter((item) => item.snoozedUntil === null || item.snoozedUntil <= now);
  const setAside = open.length - due.length;
  const working = agents.filter((agent) => activity[agent.id]?.state === "thinking");
  const quiet = pending.length + stuck.length + due.length + news.length === 0;

  const openOutcome = (outcome: Outcome) => {
    if (outcome.seenAt === null) void seeOutcomes({ runIds: [outcome.runId] });
    if (outcome.noteId) void openMessage(outcome.agentId, outcome.noteId);
    else void select(outcome.agentId);
  };

  return (
    <section className="home" aria-label="Today">
      <Field onSearch={onSearch} onManageGroups={onManageGroups} />

      {quiet ? (
        <Quiet working={working.map((agent) => agent.name)} decisions={setAside} now={now} />
      ) : (
        <>
          <Desk onOpenChannel={(agent) => void select(agent)} />
          {due.length > 0 && (
            <section className="home__filed" aria-label="Waiting in For you">
              {due.slice(0, FILED_SHOWN).map((item) => {
                const agent = agents.find((one) => one.id === item.agentId);
                const name = agent?.name ?? "A deleted agent";
                return (
                  <article key={item.id} className="home__card">
                    <AgentAvatar
                      avatar={agent?.avatar ?? "blank"}
                      color={agent?.color ?? "#8a8a8a"}
                      size="sm"
                      seed={agent?.id ?? item.id}
                    />
                    <div className="home__cardtext">
                      <p className="home__question">{item.request.question}</p>
                      {/* No time on it: nothing is waiting any more. The agent
                          was told it is on For you and went on without it, and
                          an answer reaches it as a new message, which is what
                          starts it on this again. */}
                      <p className="home__meta">
                        {name} went on without an answer. Your answer reaches {name} as a new
                        message.
                      </p>
                    </div>
                    <button
                      type="button"
                      className="btn btn--small"
                      onClick={() => showForYou(true)}
                    >
                      Answer in For you
                    </button>
                  </article>
                );
              })}
              {due.length > FILED_SHOWN && (
                <button type="button" className="home__more" onClick={() => showForYou(true)}>
                  {due.length - FILED_SHOWN} more in For you
                </button>
              )}
            </section>
          )}
          {news.length > 0 && (
            <section className="home__finished" aria-labelledby="home-finished">
              <header className="home__head">
                <h2 id="home-finished">
                  Finished <span className="home__count">{news.length}</span>
                </h2>
                <button
                  type="button"
                  className="home__link"
                  onClick={() => {
                    const through = drawnThrough(news);
                    if (through !== null) void seeOutcomes({ before: through });
                  }}
                >
                  Mark all seen
                </button>
              </header>
              <ul className="home__list">
                {news.map((outcome) => {
                  const agent = agents.find((one) => one.id === outcome.agentId);
                  const group = groups.find((one) => one.id === outcome.groupId);
                  const word = statusWord(outcome.status);
                  return (
                    <li key={outcome.runId}>
                      <button
                        type="button"
                        className="home__row"
                        data-status={outcome.status}
                        onClick={() => openOutcome(outcome)}
                      >
                        <AgentAvatar
                          avatar={agent?.avatar ?? "blank"}
                          color={agent?.color ?? "#8a8a8a"}
                          size="sm"
                          lifecycle={agent?.lifecycle}
                          seed={agent?.id ?? outcome.runId}
                        />
                        <span className="home__rowtext">
                          <span className="home__meta">
                            <strong>{agent?.name ?? "A deleted agent"}</strong>
                            {group && groups.length > 1 ? ` ${group.name},` : ""}{" "}
                            {clockLabel(outcome.finishedAt)}
                            {word ? `, ${word}` : ""}
                          </span>
                          <span className="home__headline">{outcome.headline}</span>
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          )}
        </>
      )}
    </section>
  );
}

/**
 * Nothing waiting and nothing new. Said in one sentence, with who is still at
 * work and the one way back into what already happened today.
 */
function Quiet({ working, decisions, now }: { working: string[]; decisions: number; now: number }) {
  const showForYou = useStore((state) => state.showForYou);
  const [today, setToday] = useState<number | null>(null);
  const midnight = new Date(now).setHours(0, 0, 0, 0);

  useEffect(() => {
    let current = true;
    api
      .listOutcomes({ since: midnight, limit: 100 })
      .then((rows) => {
        if (current) setToday(rows.length);
      })
      .catch((error: unknown) => {
        // A count under a calm sentence is not worth a banner.
        console.warn(`Today's finished runs could not be read: ${errorMessage(error)}`);
      });
    return () => {
      current = false;
    };
  }, [midnight]);

  const still =
    working.length === 0
      ? "You've seen everything."
      : working.length === 1
        ? `You've seen everything. ${working[0]} is working.`
        : `You've seen everything. ${working.length} agents are working.`;
  return (
    <div className="home__quiet">
      <h2 className="home__calm">Nothing needs you.</h2>
      <p className="home__still">{still}</p>
      {(decisions > 0 || (today ?? 0) > 0) && (
        <button type="button" className="home__more" onClick={() => showForYou(true)}>
          {decisions > 0
            ? `${decisions === 1 ? "1 decision" : `${decisions} decisions`} set aside in For you`
            : `Today, ${today} finished`}
        </button>
      )}
    </div>
  );
}
