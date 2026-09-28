import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { relativeTime } from "../lib/time";
import {
  type AgentCard,
  type CodingSession,
  errorMessage,
  type Gate,
  type GuacaKey,
  HARNESSES,
  type Harness,
  type HarnessOnMachine,
  type ModelOffer,
  type Payer,
  type TerminalView,
  type Tuning,
} from "../lib/types";

interface Props {
  agent: AgentCard;
}

/** One harness's tuning out of the view, as the three values a save sends. */
function tuningOf(view: TerminalView, harness: Harness): Tuning {
  const row = view.tunings.find((known) => known.harness === harness);
  return { model: row?.model ?? null, effort: row?.effort ?? null, pays: row?.pays ?? "own" };
}

/** What an operator is shown for a harness, including one this build predates. */
function labelOf(harness: Harness): string {
  return HARNESSES.find((known) => known.id === harness)?.label ?? harness;
}

/**
 * An agent's terminal, in its panel: whether it has one, which program writes
 * its code, and whether its pushes ask first.
 *
 * Given and taken back like a computer, and for the same reason: it is a
 * decision about this agent, and what it can reach follows from it. The two
 * answers under it are written the moment they are clicked, which is what a
 * `.choice` means everywhere else in this app; staged under a Save button, a
 * harness switch made because a plan just ran out is the change most likely to
 * be lost.
 *
 * Taking it back keeps the directory and both answers. The work in it is the
 * agent's, and a change of mind about access is not a reason to delete it.
 *
 * The last coding session is here too, because it is what the operator's own
 * terminal would offer them: the line that opens it in its own program, and a
 * box that carries it on from where it stopped.
 */
export function TerminalPanel({ agent }: Props) {
  const [machine, setMachine] = useState<HarnessOnMachine[] | null>(null);
  const [view, setView] = useState<TerminalView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Which agent the panel is currently about. A lookup started for one agent
  // landing after the operator switched would paint its path under another.
  const showing = useRef(agent.id);
  const given = agent.hasTerminal;
  // A job starting or ending is what changes the session under this panel.
  const running = useStore((s) => s.building[agent.id]);

  const look = useCallback(async () => {
    const asked = agent.id;
    try {
      const [terminal, harnesses] = await Promise.all([
        api.agentTerminal(asked),
        api.codingHarnesses(),
      ]);
      if (showing.current !== asked) return;
      setView(terminal);
      setMachine(harnesses);
    } catch (caught) {
      if (showing.current === asked) setError(errorMessage(caught));
    }
  }, [agent.id]);

  useEffect(() => {
    showing.current = agent.id;
    setView(null);
    setMachine(null);
    setBusy(false);
    setError(null);
  }, [agent.id]);

  useEffect(() => {
    if (given) void look();
  }, [given, look, running]);

  /**
   * Every change here is a change to the card, and the roster refresh that
   * follows is what redraws this panel. Nothing is patched locally.
   */
  const decide = async (run: () => Promise<unknown>) => {
    const asked = agent.id;
    setBusy(true);
    setError(null);
    try {
      await run();
    } catch (caught) {
      if (showing.current === asked) setError(errorMessage(caught));
    } finally {
      if (showing.current === asked) setBusy(false);
    }
  };

  // Null is the check still running, and it must not disable every harness.
  const has = (harness: Harness) => {
    const row = machine?.find((known) => known.harness === harness);
    return row === undefined || (row.installed && !row.withheld);
  };
  const chosen = machine?.find((row) => row.harness === agent.harness);
  const lent = view !== null && tuningOf(view, agent.harness).pays === "guacaKey";

  return (
    <section className="worknotes" aria-label={`${agent.name}'s terminal`}>
      <div className="worknotes__head">
        <h3 className="worknotes__title">Terminal</h3>
      </div>

      {!given ? (
        <>
          <p className="worknotes__empty">
            {agent.name} has no terminal. Give it one and it gets a directory of its own on the
            machine Guaca runs on: it can run commands there, clone repositories, change files, and
            hand bigger changes to a coding agent.
          </p>
          <div className="screen__offer">
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy}
              onClick={() => void decide(() => api.giveAgentTerminal(agent.id))}
            >
              {busy ? "Working…" : "Give one"}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="field__hint">
            {view ? (
              <>
                Its directory is <code>{view.path}</code>. Commands run there as the backend's user,
                with that machine's git and GitHub sign-ins. This is not a sandbox.
              </>
            ) : (
              "Finding its directory…"
            )}
          </p>

          <div className="field">
            <span className="field__label">Writes code with</span>
            <div className="choices">
              {HARNESSES.map((harness) => (
                <button
                  key={harness.id}
                  type="button"
                  className="choice choice--tight"
                  aria-label={`Coding harness: ${harness.label}`}
                  aria-pressed={harness.id === agent.harness}
                  disabled={busy || !has(harness.id)}
                  onClick={() =>
                    void decide(() => api.setAgentCoding(agent.id, harness.id, agent.gate))
                  }
                >
                  {harness.label}
                </button>
              ))}
            </div>
            <span className="field__hint">
              {!chosen ? (
                `${labelOf(agent.harness)}: status not checked yet.`
              ) : chosen.withheld ? (
                `${labelOf(agent.harness)}: ${chosen.withheld}.`
              ) : !chosen.installed ? (
                <>
                  {labelOf(agent.harness)} is not installed. On the backend, run{" "}
                  <code>{chosen.install}</code>.
                </>
              ) : chosen.signedIn === false ? (
                <>
                  {labelOf(agent.harness)} is not signed in on the backend. Run{" "}
                  <code>{chosen.signIn}</code> as the backend's user. Guaca's own sign-in does not
                  sign in the coding tool.
                </>
              ) : lent ? (
                "pi runs on the backend, paid for with Guaca's key as set below."
              ) : (
                `${labelOf(agent.harness)} runs on the backend with its own sign-in. Its model and effort can be chosen below.`
              )}
            </span>
          </div>

          {view && (
            <Tuner
              key={`${agent.id}:${agent.harness}`}
              agent={agent}
              harness={agent.harness}
              tuning={tuningOf(view, agent.harness)}
              guacaKey={view.guacaKey}
              efforts={chosen?.efforts ?? []}
              usable={chosen === undefined || (chosen.installed && !chosen.withheld)}
              onSaved={() => void look()}
            />
          )}

          <label className="field field--row">
            <input
              type="checkbox"
              checked={agent.gate === "askBeforePushing"}
              disabled={busy}
              onChange={(event) => {
                const gate: Gate = event.target.checked ? "askBeforePushing" : "open";
                void decide(() => api.setAgentCoding(agent.id, agent.harness, gate));
              }}
            />
            <span>
              <span className="field__label">Ask me before pushing</span>
              <span className="field__hint">
                A push, pull request, merge or release waits on your desk first, whether{" "}
                {agent.name} runs it or its coding agent does. Everything else runs without asking.
                This is an approval step, not a sandbox.
              </span>
            </span>
          </label>

          {view?.session && (
            <LastSession
              key={agent.id}
              agent={agent}
              session={view.session}
              resume={view.resume}
              running={running !== undefined}
            />
          )}

          <div className="screen__offer">
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy}
              onClick={() => void decide(() => api.takeAgentTerminal(agent.id))}
              title="Take it back. A running coding job stops, and the directory is kept."
            >
              Take it back
            </button>
          </div>
        </>
      )}

      {error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

interface LastSessionProps {
  agent: AgentCard;
  session: CodingSession;
  resume: string | null;
  running: boolean;
}

/**
 * The session a follow-up carries on, and the two ways to carry it on.
 *
 * The box is the same call a running job's correction goes through, and the
 * runtime decides which it is. Hidden while a job runs, because the channel's
 * own panel is where that job is watched and steered, and two boxes that do the
 * same thing from two places is one too many.
 */
function LastSession({ agent, session, resume, running }: LastSessionProps) {
  const [message, setMessage] = useState("");
  const [sending, setSending] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const age = relativeTime(session.updatedAt, Date.now());
  const where = session.directory === "." ? "its terminal" : session.directory;
  const switched = session.harness !== agent.harness;

  const send = async () => {
    const said = message.trim();
    if (!said || sending) return;
    setSending(true);
    setNote(null);
    try {
      await api.messageCodingJob(agent.id, said);
      setMessage("");
      // What was typed goes to the harness and nowhere else, so this is the
      // only evidence it arrived until the job reports.
      setNote(`Carrying on in the same session. ${agent.name} is told what comes of it.`);
    } catch (caught) {
      setNote(errorMessage(caught));
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="field">
      <span className="field__label">Last coding session</span>
      <span className="field__hint">
        {labelOf(session.harness)} in {where}, {age === "now" ? "just now" : `${age} ago`}.
        {resume && (
          <>
            {" "}
            To open it yourself, run this on the backend as its user: <code>{resume}</code>
          </>
        )}
      </span>
      {running ? (
        <span className="field__hint">
          A job is running now. Steer it from {agent.name}'s channel.
        </span>
      ) : switched ? (
        <span className="field__hint">
          {labelOf(agent.harness)} cannot carry on a {labelOf(session.harness)} session. The next
          job starts a fresh one.
        </span>
      ) : (
        <div className="coding__say">
          <input
            className="input input--slim"
            placeholder="a follow-up: now add a test for it…"
            value={message}
            disabled={sending}
            aria-label="Send a follow-up to the last coding session"
            onChange={(event) => {
              setMessage(event.target.value);
              setNote(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && message.trim()) void send();
            }}
          />
          <button
            type="button"
            className="btn btn--small"
            disabled={sending || !message.trim()}
            onClick={() => void send()}
          >
            Continue
          </button>
        </div>
      )}
      {note && <span className="field__hint">{note}</span>}
    </div>
  );
}

interface TunerProps {
  agent: AgentCard;
  harness: Harness;
  tuning: Tuning;
  guacaKey: GuacaKey;
  /** Every effort word the program takes, from Rust. */
  efforts: string[];
  /** Whether the program is here to be asked for its models at all. */
  usable: boolean;
  onSaved: () => void;
}

/**
 * What the operator chooses inside the harness, the way its own window lets
 * them: the model, the effort, and for pi who pays.
 *
 * The model field is a text box that suggests the program's own list, asked of
 * the program, because the list is the program's and moves with its releases:
 * a name it does not show is still one it may take. Written on Enter or when
 * the field is left, and the two choices on the click, as every `.choice` here.
 */
function Tuner({ agent, harness, tuning, guacaKey, efforts, usable, onSaved }: TunerProps) {
  const [model, setModel] = useState(tuning.model ?? "");
  const [offers, setOffers] = useState<ModelOffer[] | null>(null);
  const [listing, setListing] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const label = labelOf(harness);
  const pays = tuning.pays;
  // The listing a response belongs to. A pi listing for its own sign-in that
  // lands after the operator switched it to Guaca's key would offer models the
  // job cannot reach.
  const asking = useRef(`${harness}:${pays}`);

  useEffect(() => {
    const asked = `${harness}:${pays}`;
    asking.current = asked;
    setOffers(null);
    setListing(null);
    if (!usable) return;
    api.codingModels(harness, pays).then(
      (found) => {
        if (asking.current === asked) setOffers(found);
      },
      (caught) => {
        if (asking.current === asked) setListing(errorMessage(caught));
      },
    );
  }, [harness, pays, usable]);

  const save = async (next: Tuning) => {
    setSaving(true);
    setNote(null);
    try {
      await api.setCodingTuning(agent.id, harness, next);
      setNote("Saved. The next job runs with it.");
      onSaved();
    } catch (caught) {
      setNote(errorMessage(caught));
    } finally {
      setSaving(false);
    }
  };

  const typed = model.trim() || null;
  const commitModel = () => {
    if (typed !== tuning.model) void save({ ...tuning, model: typed });
  };
  const choosePays = (next: Payer) => {
    // A model named for one account is not one the other can be assumed to
    // reach, so a switch goes back to the default rather than carrying it.
    if (next !== pays) {
      setModel("");
      void save({ model: null, effort: tuning.effort, pays: next });
    }
  };

  const offer = offers?.find((known) => known.id === (typed ?? offers.find((o) => o.default)?.id));
  // The model's own list when the program gave one, and every word the
  // program takes when it did not.
  const levels = offer ? offer.efforts : efforts;
  const fallback =
    pays === "guacaKey"
      ? guacaKey.defaultModel
      : (offers?.find((o) => o.default)?.label ?? `${label}'s own setting`);

  return (
    <>
      {harness === "pi" && (
        <div className="field">
          <span className="field__label">Paid for by</span>
          <div className="choices">
            {(
              [
                ["own", "pi's own sign-in"],
                ["guacaKey", "Guaca's API key"],
              ] as const
            ).map(([value, name]) => (
              <button
                key={value}
                type="button"
                className="choice choice--tight"
                aria-label={`Paid for by: ${name}`}
                aria-pressed={pays === value}
                disabled={saving || (value === "guacaKey" && !guacaKey.set && pays !== value)}
                onClick={() => choosePays(value)}
              >
                {name}
              </button>
            ))}
          </div>
          <span className="field__hint">
            {pays === "guacaKey" ? (
              <>
                Paid for with the key in Settings &gt; Provider, at <code>{guacaKey.endpoint}</code>
                . pi is handed a loopback address and a token for the job, never the key.
                {!guacaKey.set && " There is no key there now, so a job will be refused."}
              </>
            ) : guacaKey.set ? (
              "Whatever pi is signed in to on the backend pays."
            ) : (
              "Whatever pi is signed in to on the backend pays. Guaca has no API key in Settings > Provider to lend it."
            )}
          </span>
        </div>
      )}

      <label className="field">
        <span className="field__label">Model</span>
        <input
          className="input input--slim input--mono"
          list={`models-${agent.id}`}
          value={model}
          placeholder={fallback}
          disabled={saving}
          aria-label={`${label} model`}
          onChange={(event) => {
            setModel(event.target.value);
            setNote(null);
          }}
          onBlur={commitModel}
          onKeyDown={(event) => {
            if (event.key === "Enter") commitModel();
          }}
        />
        <datalist id={`models-${agent.id}`}>
          {(offers ?? []).map((known) => (
            <option key={known.id} value={known.id}>
              {known.detail ? `${known.label} · ${known.detail}` : known.label}
            </option>
          ))}
        </datalist>
        <span className="field__hint">
          {!usable
            ? `${label} is not here to ask for its models. A name typed here is passed to it as it is.`
            : listing
              ? listing
              : offers === null
                ? `Asking ${label} for its models…`
                : offers.length === 0
                  ? pays === "guacaKey"
                    ? "This endpoint publishes no list pi can read. Type the model name it serves."
                    : `${label} listed no models. Type a name it takes.`
                  : pays === "guacaKey"
                    ? `Suggested from pi's catalog for this endpoint. Empty is the key's own model, ${guacaKey.defaultModel}.`
                    : `Suggested from ${label}'s own list. Empty is ${label}'s default.`}
        </span>
      </label>

      <label className="field">
        <span className="field__label">Effort</span>
        <select
          className="select"
          value={tuning.effort ?? ""}
          disabled={saving || (offer !== undefined && offer.efforts.length === 0)}
          aria-label={`${label} effort`}
          onChange={(event) => void save({ ...tuning, effort: event.target.value || null })}
        >
          <option value="">{`${label}'s default`}</option>
          {levels.map((level) => (
            <option key={level} value={level}>
              {level}
            </option>
          ))}
          {tuning.effort && !levels.includes(tuning.effort) && (
            <option value={tuning.effort}>{tuning.effort}</option>
          )}
        </select>
        {offer !== undefined && offer.efforts.length === 0 && (
          <span className="field__hint">{`${offer.label} takes no effort setting.`}</span>
        )}
      </label>
      {note && <span className="field__hint">{note}</span>}
    </>
  );
}
