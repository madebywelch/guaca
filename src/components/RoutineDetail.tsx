import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import {
  anchorFor,
  choiceFor,
  clockTime,
  eventFields,
  eventSpec,
  firstRunDelay,
  type Moment,
  momentOf,
  nextRoundMoment,
  ordinal,
  parseTrigger,
  repeatLabel,
  repeats,
  routineTitle,
  TRIGGER_CHOICES,
  WEEKDAYS,
  webhookLine,
} from "../lib/routine";
import { relativeTime, useNow } from "../lib/time";
import {
  type AgentId,
  errorMessage,
  type Routine,
  type RoutineDraft,
  type RoutineId,
  type RoutineRun,
  type WebhookAddress,
} from "../lib/types";
import { compact, money, priced } from "./Spend";

interface Props {
  agentId: AgentId;
  routineId: RoutineId | "new";
  onBack: () => void;
}

interface Editing {
  name: string;
  what: string;
  trigger: string;
  moment: Moment;
  skipIfWorking: boolean;
}

const DEFAULT_TRIGGER = "daily";

const HIDDEN_SECRET = "••••••••";

function editingFor(routine: Routine): Editing {
  return {
    name: routine.name,
    what: routine.what,
    trigger: routine.trigger,
    moment: routine.nextRunAt === null ? nextRoundMoment() : momentOf(routine.nextRunAt),
    skipIfWorking: routine.skipIfWorking,
  };
}

function blank(): Editing {
  return {
    name: "",
    what: "",
    trigger: DEFAULT_TRIGGER,
    moment: nextRoundMoment(),
    skipIfWorking: false,
  };
}

function draftOf(editing: Editing, moved: boolean): RoutineDraft {
  return {
    name: editing.name.trim(),
    what: editing.what.trim(),
    trigger: editing.trigger,
    inSecs: moved ? firstRunDelay(editing.trigger, editing.moment) : null,
    skipIfWorking: repeats(editing.trigger) && editing.skipIfWorking,
  };
}

function triggerProblem(editing: Editing): string | null {
  if (eventFields(editing.trigger) && parseTrigger(editing.trigger).kind !== "event") {
    return "Name the service and what it reports, with no spaces in either.";
  }
  const anchor = anchorFor(editing.trigger);
  if (anchor === "none") return null;
  if (firstRunDelay(editing.trigger, editing.moment) !== null) return null;
  return anchor === "date"
    ? "That date and time have already passed. Pick one still ahead."
    : "That is not a time of day.";
}

export function RoutineDetail({ agentId, routineId, onBack }: Props) {
  const isNew = routineId === "new";
  const [routine, setRoutine] = useState<Routine | null>(null);
  const [draft, setDraft] = useState<Editing>(() => blank());
  const [runs, setRuns] = useState<RoutineRun[] | null>(null);
  const [moved, setMoved] = useState(false);
  const [loading, setLoading] = useState(!isNew);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [address, setAddress] = useState<WebhookAddress | null>(null);
  const [secret, setSecret] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const now = useNow(30_000);

  const load = useCallback(async () => {
    if (isNew) return;
    try {
      const found = (await api.agentRoutines(agentId)).find((r) => r.id === routineId);
      if (!found) {
        onBack();
        return;
      }
      setRoutine(found);
      setDraft(editingFor(found));
      setMoved(false);
      setRuns(await api.routineRuns(found.id));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }, [agentId, routineId, isNew, onBack]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await action();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const patch = (fields: Partial<Editing>) => setDraft((current) => ({ ...current, ...fields }));
  const patchMoment = (fields: Partial<Moment>) => {
    setDraft((current) => ({ ...current, moment: { ...current.moment, ...fields } }));
    setMoved(true);
  };

  const dirty =
    routine === null ||
    moved ||
    draft.name.trim() !== routine.name ||
    draft.what.trim() !== routine.what ||
    draft.trigger !== routine.trigger ||
    draftOf(draft, moved).skipIfWorking !== routine.skipIfWorking;

  const problem = triggerProblem(draft);
  const event = eventFields(draft.trigger);
  const waiting = event !== null;

  useEffect(() => {
    if (!waiting || address !== null) return;
    let stale = false;
    api
      .webhookAddress()
      .then((found) => {
        if (!stale) setAddress(found);
      })
      .catch((caught) => {
        if (!stale) setError(errorMessage(caught));
      });
    return () => {
      stale = true;
    };
  }, [waiting, address]);

  const save = () =>
    void run(async () => {
      if (routine) {
        await api.updateRoutine(routine.id, draftOf(draft, moved));
        await load();
      } else {
        const made = await api.createRoutine(agentId, draftOf(draft, true));
        setRoutine(made);
        setDraft(editingFor(made));
        setMoved(false);
        setRuns([]);
      }
    });

  if (loading) return <p className="routines__note">Loading…</p>;

  const anchor = anchorFor(draft.trigger);

  return (
    <div className="detail">
      <div className="detail__actions">
        {routine ? (
          <>
            <button
              type="button"
              role="switch"
              aria-checked={routine.active}
              className="toggle"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  setRoutine(await api.setRoutineActive(routine.id, !routine.active));
                })
              }
            >
              <span aria-hidden="true" className="toggle__track" />
              <span className="toggle__label">{routine.active ? "Active" : "Off"}</span>
            </button>

            <span style={{ flex: 1 }} />

            {confirmDelete ? (
              <>
                <button
                  type="button"
                  className="btn btn--small btn--danger"
                  disabled={busy}
                  onClick={() =>
                    void run(() => api.deleteRoutine(routine.id)).then((ok) => {
                      if (ok) onBack();
                    })
                  }
                >
                  Delete it
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  onClick={() => setConfirmDelete(false)}
                >
                  Keep
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  className="btn btn--small"
                  disabled={busy}
                  onClick={() => setConfirmDelete(true)}
                >
                  Delete
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy || dirty}
                  title={
                    dirty
                      ? "Save first, so what runs is what you are looking at"
                      : "Run it now. The schedule does not move."
                  }
                  onClick={() =>
                    void run(async () => {
                      await api.testRoutine(routine.id);
                      setRuns(await api.routineRuns(routine.id));
                      setNote("Sent. Watch the conversation.");
                    })
                  }
                >
                  Test run
                </button>
              </>
            )}
          </>
        ) : (
          <p className="routines__note" style={{ margin: 0 }}>
            New routine. It starts running as soon as you create it.
          </p>
        )}
      </div>

      <label className="field">
        <span className="field__label">Name</span>
        <input
          className="input"
          aria-label="Name"
          maxLength={64}
          value={draft.name}
          placeholder={
            draft.what.trim() ? routineTitle({ name: "", what: draft.what }) : "Listings sweep"
          }
          onChange={(event) => patch({ name: event.target.value })}
        />
      </label>

      <label className="field">
        <span className="field__label">Instruction</span>
        <textarea
          className="textarea"
          aria-label="Instruction"
          rows={7}
          placeholder="Check the listings and tell me what is new"
          value={draft.what}
          onChange={(event) => patch({ what: event.target.value })}
        />
        <span className="field__hint">
          Delivered to the agent as a message when it fires, in a fresh run with nothing else in it.
          Write it as something the agent can act on with no other context.
        </span>
      </label>

      <div className="field">
        <span className="field__label">When to run</span>
        <div className="when">
          <span aria-hidden="true" className="routine__mark" data-waiting={waiting || undefined} />
          <select
            className="input input--slim"
            aria-label="Trigger"
            value={choiceFor(draft.trigger)}
            onChange={(change) => {
              patch({ trigger: change.target.value });
              if (anchorFor(change.target.value) !== "none") setMoved(true);
            }}
          >
            {TRIGGER_CHOICES.map((choice) => (
              <option key={choice.spec} value={choice.spec}>
                {choice.label}
              </option>
            ))}
            {!TRIGGER_CHOICES.some((choice) => choice.spec === choiceFor(draft.trigger)) && (
              <option value={draft.trigger}>{repeatLabel(draft.trigger)}</option>
            )}
          </select>

          {event && (
            <>
              <input
                className="input input--slim"
                aria-label="Service"
                placeholder="stripe"
                value={event.service}
                onChange={(change) =>
                  patch({ trigger: eventSpec(change.target.value, event.topic) })
                }
              />
              <span className="hint">reports</span>
              <input
                className="input input--slim"
                aria-label="Event"
                placeholder="invoice.payment_failed"
                value={event.topic}
                onChange={(change) =>
                  patch({ trigger: eventSpec(event.service, change.target.value) })
                }
              />
            </>
          )}

          {anchor === "weekday" && (
            <select
              className="input input--slim"
              aria-label="Day of the week"
              value={draft.moment.weekday}
              onChange={(event) => patchMoment({ weekday: Number(event.target.value) })}
            >
              {WEEKDAYS.map((day) => (
                <option key={day.day} value={day.day}>
                  {day.label}
                </option>
              ))}
            </select>
          )}

          {anchor === "monthday" && (
            <select
              className="input input--slim"
              aria-label="Day of the month"
              value={draft.moment.monthday}
              onChange={(event) => patchMoment({ monthday: Number(event.target.value) })}
            >
              {Array.from({ length: 31 }, (_, index) => index + 1).map((day) => (
                <option key={day} value={day}>
                  {ordinal(day)}
                </option>
              ))}
            </select>
          )}

          {anchor === "date" && (
            <input
              className="input input--slim"
              type="date"
              aria-label="Date"
              value={draft.moment.date}
              onChange={(event) => patchMoment({ date: event.target.value })}
            />
          )}

          {anchor !== "none" && (
            <>
              <span className="hint">at</span>
              <input
                className="input input--slim"
                type="time"
                aria-label="Time of day"
                value={draft.moment.time}
                onChange={(event) => patchMoment({ time: event.target.value })}
              />
            </>
          )}
        </div>

        {anchor === "monthday" && draft.moment.monthday > 28 && (
          <span className="field__hint">
            Months without a {ordinal(draft.moment.monthday)} are skipped rather than moved, so it
            stays on the {ordinal(draft.moment.monthday)} of the months that have one.
          </span>
        )}
        {event && address && !address.url && (
          <span className="field__hint">
            Nothing on the clock. An event can only be posted to a Host, so in this workspace the
            routine fires only from Test run.
          </span>
        )}
        {event && address?.url && (
          <span className="field__hint">
            Nothing on the clock: it fires each time this is posted to Guaca, and Test run delivers
            it now.
          </span>
        )}
        {event && address?.url && parseTrigger(draft.trigger).kind === "event" && (
          <>
            <pre className="webhook">
              {webhookLine(address.url, secret ?? HIDDEN_SECRET, event.service, event.topic)}
            </pre>
            <div className="webhook__actions">
              <button
                type="button"
                className="btn btn--small btn--ghost"
                onClick={() => {
                  if (secret !== null) {
                    setSecret(null);
                    return;
                  }
                  api
                    .revealWebhookSecret()
                    .then(setSecret)
                    .catch((caught) => setError(errorMessage(caught)));
                }}
              >
                {secret === null ? "Show secret" : "Hide secret"}
              </button>
              <button
                type="button"
                className="btn btn--small btn--ghost"
                onClick={() => {
                  const url = address.url;
                  if (!url) return;
                  (secret === null ? api.revealWebhookSecret() : Promise.resolve(secret))
                    .then((shown) =>
                      navigator.clipboard.writeText(
                        webhookLine(url, shown, event.service, event.topic),
                      ),
                    )
                    .then(() => setCopied(true))
                    .catch((caught) => setError(errorMessage(caught)));
                }}
              >
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
            <span className="field__hint">
              The body, if there is one, reaches the agent under the instruction as data. Guaca
              calls nobody and polls nothing: whatever posts this is yours to wire.
            </span>
          </>
        )}
        {routine?.active && !dirty && routine.nextRunAt !== null && (
          <span className="field__hint">
            Next in {relativeTime(now, routine.nextRunAt)}, on{" "}
            {new Date(routine.nextRunAt).toLocaleDateString(undefined, {
              weekday: "long",
              month: "short",
              day: "numeric",
            })}
            .
          </span>
        )}
      </div>

      {repeats(draft.trigger) && (
        <label className="field field--row">
          <input
            type="checkbox"
            checked={draft.skipIfWorking}
            onChange={(event) => patch({ skipIfWorking: event.target.checked })}
          />
          <span>
            <span className="field__label">Skip it if the agent is already working</span>
            <span className="field__hint">
              A firing that comes due while this agent is mid-turn or has work waiting is dropped
              rather than queued behind it, and the next one comes at its usual time. For a sweep
              there is no point doing twice over. Leave it off for anything that has to happen even
              if it has to wait.
            </span>
          </span>
        </label>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0 0 0.7rem" }}>
          <span>{error}</span>
        </div>
      )}
      {note && !error && <p className="field__hint">{note}</p>}

      {dirty && (
        <div className="detail__save">
          <button
            type="button"
            className="btn btn--small btn--primary"
            disabled={busy || !draft.what.trim() || problem !== null}
            onClick={save}
          >
            {routine ? "Save changes" : "Create routine"}
          </button>
          {routine && (
            <button
              type="button"
              className="btn btn--small btn--ghost"
              onClick={() => {
                setDraft(editingFor(routine));
                setMoved(false);
              }}
            >
              Discard
            </button>
          )}
          {problem && <span className="field__hint">{problem}</span>}
        </div>
      )}

      {routine && <History runs={runs} now={now} />}
    </div>
  );
}

function History({ runs, now }: { runs: RoutineRun[] | null; now: number }) {
  return (
    <div className="field">
      <span className="field__label">Run history</span>
      {runs === null || runs.length === 0 ? (
        <p className="routines__note">No runs yet.</p>
      ) : (
        <ul className="history">
          {runs.map((entry, index) => (
            <li key={entry.runId ?? `${entry.kind}-${entry.at}-${index}`} className="history__row">
              <span className="history__when">
                {new Date(entry.at).toLocaleDateString(undefined, {
                  month: "short",
                  day: "numeric",
                })}{" "}
                at {clockTime(entry.at)}
              </span>
              {entry.kind === "test" && <span className="history__kind">test run</span>}
              {entry.kind === "scheduled" && entry.missed > 0 && (
                <span className="history__kind">
                  {`Fired late, missed ${entry.missed} ${entry.missed === 1 ? "time" : "times"} while the host was off`}
                </span>
              )}
              {entry.kind === "failed" ? (
                <span className="history__quiet">
                  Failed{entry.error ? `: ${entry.error}` : ""}
                </span>
              ) : entry.kind === "skipped" ? (
                <span
                  className="history__quiet"
                  title="This routine skips a firing that lands while the agent is already working, so nothing was delivered"
                >
                  skipped, already working
                </span>
              ) : entry.spent.calls === 0 ? (
                <span
                  className="history__quiet"
                  title="Delivered, but no model call was made under it"
                >
                  nothing ran
                </span>
              ) : (
                <span
                  className="history__spend"
                  title={`${entry.spent.prompt.toLocaleString()} in, ${entry.spent.completion.toLocaleString()} out, over ${entry.spent.calls} model call(s)`}
                >
                  {compact(entry.spent.prompt + entry.spent.completion)}
                  {priced(entry.spent.cost) && (
                    <span className="history__cost">{money(entry.spent.cost)}</span>
                  )}
                </span>
              )}
              <span className="history__ago">{relativeTime(entry.at, now)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
