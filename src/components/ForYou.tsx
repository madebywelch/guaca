import { useEffect, useRef, useState } from "react";

import { decisionTime, needsAnswer, orderedDecisions } from "../lib/decisions";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { relativeTime, useNow } from "../lib/time";
import { errorMessage, type WorkDecision } from "../lib/types";
import { Desk } from "./Desk";

export function ForYou({ onClose }: { onClose: () => void }) {
  const decisions = useStore((state) => state.decisions);
  const refresh = useStore((state) => state.refreshDecisions);
  const error = useStore((state) => state.decisionError);
  const banner = useStore((state) => state.banner);
  const pending = useStore((state) => state.pending);
  const stuck = useStore((state) => state.stuck);
  const agents = useStore((state) => state.agents);
  const lastActive = useStore((state) => state.lastActive);
  const now = useNow(30_000);
  const panel = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<"needs" | "following" | "history">("needs");
  const [receipt, setReceipt] = useState("");
  useEffect(() => {
    const previous = document.activeElement;
    panel.current?.focus();
    void refresh();
    return () => {
      if (previous instanceof HTMLElement) previous.focus();
    };
  }, [refresh]);
  const open = orderedDecisions(decisions.filter(needsAnswer));
  const following = decisions.filter((item) => item.status === "answered" && !item.interrupted);
  const history = decisions
    .filter((item) => item.status === "completed" || item.status === "withdrawn")
    .sort((a, b) => b.updatedAt - a.updatedAt);
  const snoozed = open.filter((item) => item.snoozedUntil !== null && item.snoozedUntil > now);
  const due = open.filter((item) => !snoozed.includes(item));
  const shown = view === "needs" ? due : view === "following" ? following : history;
  const count = open.length + pending.length + stuck.length;
  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close For you" onClick={onClose} />
      <div
        className="dialog for-you"
        role="dialog"
        aria-modal="true"
        aria-label="For you"
        tabIndex={-1}
        ref={panel}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            onClose();
          }
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
            <h2>
              For you <span>{count}</span>
            </h2>
            <button type="button" className="btn btn--ghost" onClick={onClose}>
              Close
            </button>
          </div>
          <p>
            {count === 0
              ? "Nothing needs your answer right now."
              : `${count} ${count === 1 ? "item needs" : "items need"} your attention.`}{" "}
            {following.length > 0 ? `${following.length} awaiting follow-through.` : ""}
          </p>
          <nav className="for-you__tabs" aria-label="Decision views">
            <button
              type="button"
              className="btn"
              aria-pressed={view === "needs"}
              onClick={() => setView("needs")}
            >
              Needs you
            </button>
            <button
              type="button"
              className="btn"
              aria-pressed={view === "following"}
              onClick={() => setView("following")}
            >
              Following through {following.length}
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
                className="btn btn--small"
                onClick={() => useStore.getState().setBanner(null)}
              >
                Dismiss
              </button>
            </p>
          )}
          {error && (
            <p role="alert">
              Could not refresh decisions: {error}{" "}
              <button type="button" className="btn btn--small" onClick={() => void refresh()}>
                Retry
              </button>
            </p>
          )}
          {receipt && (
            <p role="status" className="for-you__receipt">
              {receipt}
            </p>
          )}
          {view === "needs" && <Desk />}
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
                ? "Completed and withdrawn decisions will appear here."
                : "No answered decisions awaiting action."}
            </p>
          )}
          {view === "needs" && (
            <details className="for-you__later">
              <summary>Team activity</summary>
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
          Decision reminders at 9 AM and 4 PM, plus one hour before deadlines, in the workspace’s
          timezone. Notification preferences apply.
        </footer>
      </div>
    </div>
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
      `Answer saved for ${agent?.name ?? "the agent"}. Follow-through is tracked separately.`,
    );
  const waiting = item.status === "pending";
  return (
    <article className="for-you__item">
      <div className="for-you__meta">
        <span>
          {agent?.name ?? "Deleted agent"} · {group?.name ?? "Crew"}
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
            ? "Follow-through was interrupted. Review previous actions before resuming."
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
            className="btn btn--ghost"
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
                "Follow-through resumed. The agent will check previous actions before continuing.",
              )
            }
          >
            Resume follow-through
          </button>
        )}
        {needsAnswer(item) && (
          <button
            type="button"
            className="btn btn--ghost"
            disabled={busy}
            onClick={() =>
              void act(
                () => api.snoozeDecision(item.id, Date.now() + 60 * 60 * 1000),
                "Set aside for one hour. The decision still needs your attention.",
              )
            }
          >
            Remind me in 1 hour
          </button>
        )}
        <button
          type="button"
          className="btn btn--ghost"
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
