import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import {
  type AgentCard,
  BENCHES,
  type Bench,
  errorMessage,
  type Gate,
  type GroupId,
  HARNESSES,
  type Harness,
  type HarnessOnMachine,
  type Repository,
  type RepositoryDraft,
  type RepositoryId,
} from "../lib/types";

interface Props {
  groupId: GroupId;
  crew: AgentCard[];
}

/**
 * Which program does the writing, as two buttons and one sentence.
 *
 * Two buttons rather than a select, because the set is two and the whole point
 * is seeing both at once. The same shape the surface and the scale use in
 * settings, down to the `aria-pressed` and the label that names what is being
 * chosen. One the operator does not have is drawn and disabled with the command
 * that installs it, rather than hidden: the state this control exists for is a
 * plan that has just run out, and an absent option reads as a thing the app
 * cannot do.
 *
 * On an existing repository the click *is* the change, which is what a `.choice`
 * means everywhere else in this app and is why the caller writes it through
 * rather than staging it beside the name and the note. Staged, it sits under a
 * Save button that an operator has every reason to press before they get to it,
 * and the whole change is lost with nothing on screen saying so. On the link
 * form there is no row to write to yet, so there it rides on the draft.
 */
function HarnessChoice({
  chosen,
  machine,
  disabled,
  onChoose,
}: {
  chosen: Harness;
  machine: HarnessOnMachine[] | null;
  disabled: boolean;
  onChoose: (harness: Harness) => void;
}) {
  // Null is the check still running, and it must not disable both. The refusal
  // a job gives already names the install command.
  const has = (harness: Harness) => {
    const row = machine?.find((known) => known.harness === harness);
    return row === undefined || (row.installed && !row.withheld);
  };
  const missing = machine?.filter((row) => !row.installed && !row.withheld) ?? [];
  // Withheld by where the workspace runs rather than absent from the machine,
  // and the row says which: an install command for a program a server will
  // not run is an instruction that leads nowhere.
  const withheld = machine?.filter((row) => Boolean(row.withheld)) ?? [];

  return (
    <>
      <div className="choices">
        {HARNESSES.map((harness) => (
          <button
            key={harness.id}
            type="button"
            className="choice choice--tight"
            aria-label={`Coding harness: ${harness.label}`}
            aria-pressed={harness.id === chosen}
            disabled={disabled || !has(harness.id)}
            onClick={() => onChoose(harness.id)}
          >
            {harness.label}
          </button>
        ))}
      </div>
      <p className="field__hint">
        The program that writes the code here, run on this machine with its own sign-in. Guaca never
        pays for it, so a job's spend is not in this app's usage. Switch when the plan behind one of
        them runs out: a subscription is spent by the program it was issued to, and no amount of
        configuring the other one reaches it.
        {missing.map((row) => (
          <span key={row.harness}>
            {" "}
            {labelOf(row.harness)} is not installed: <code>{row.install}</code>.
          </span>
        ))}
        {withheld.map((row) => (
          <span key={row.harness}>
            {" "}
            {labelOf(row.harness)}: {row.withheld}.
          </span>
        ))}
      </p>
    </>
  );
}

/**
 * Whether a job here asks before it reaches outside the directory.
 *
 * A checkbox rather than a pair of buttons, because it is one decision with a
 * default rather than a choice between two things: the harness above is the
 * second shape and they should not look alike.
 *
 * Disabled on a harness that cannot be reached while it works, and the hint
 * says which, because a control that silently does nothing is worse than one
 * that is not offered. `pi` has no second interface at all.
 */
function GateChoice({
  chosen,
  harness,
  machine,
  disabled,
  onChoose,
}: {
  chosen: Gate;
  harness: Harness;
  machine: HarnessOnMachine[] | null;
  disabled: boolean;
  onChoose: (gate: Gate) => void;
}) {
  // Null is the check still running, and it must not disable the control on a
  // machine that would have supported it.
  const row = machine?.find((known) => known.harness === harness);
  const reachable = machine === null || (row?.bridged ?? true);

  return (
    <label className="field field--row">
      <input
        type="checkbox"
        checked={chosen === "askBeforePushing"}
        disabled={disabled || !reachable}
        onChange={(event) => onChoose(event.target.checked ? "askBeforePushing" : "open")}
      />
      <span>
        <span className="field__label">Ask me before pushing</span>
        <span className="field__hint">
          A push, a pull request, a merge or a release waits on your desk first. Everything else a
          job does is what the directory and git already cover. It is not a sandbox: the program
          runs as you, with your credentials, and a job that wanted to get around this could. What
          it buys is that the ordinary push is one you see first. Leave it off for a repository
          nobody is watching: a job is told nobody will answer a question, and one waiting on you is
          one that runs out its own clock.
          {!reachable && (
            <>
              {" "}
              {labelOf(harness)} cannot be reached while it works, so nothing here can stop it.
              {row?.version ? ` This machine has ${row.version}.` : ""}
            </>
          )}
        </span>
      </span>
    </label>
  );
}

/**
 * Where a job in this directory works.
 *
 * Two buttons rather than a checkbox, unlike the gate above, because neither
 * answer is the absence of the other: a worktree per agent and the linked
 * directory are two arrangements, and an operator choosing the second is
 * choosing it for a reason a checkbox called "off" would not prompt them to
 * think about.
 *
 * The hint under it is the whole of the argument, because this is the one
 * setting here whose cost is invisible from the panel. A worktree is a fresh
 * checkout, so the first job in one has no installed dependencies and no
 * gitignored environment file, and pays to get them; every job after that is
 * free, and the operator's own checkout is never touched again.
 */
function BenchChoice({
  chosen,
  disabled,
  onChoose,
}: {
  chosen: Bench;
  disabled: boolean;
  onChoose: (bench: Bench) => void;
}) {
  const hint = BENCHES.find((known) => known.id === chosen)?.hint;
  return (
    <div className="field">
      <span className="field__label">Where jobs work</span>
      <div className="choice">
        {BENCHES.map((bench) => (
          <button
            key={bench.id}
            type="button"
            className="btn btn--small"
            aria-label={`Where jobs work: ${bench.label}`}
            aria-pressed={bench.id === chosen}
            disabled={disabled}
            onClick={() => onChoose(bench.id)}
          >
            {bench.label}
          </button>
        ))}
      </div>
      <span className="field__hint">{hint}</span>
    </div>
  );
}

/** What an operator is shown for a harness, including one this build predates. */
function labelOf(harness: Harness): string {
  return HARNESSES.find((known) => known.id === harness)?.label ?? harness;
}

/** Who works in a repository, as a sentence. */
function worksIn(repository: Repository, crew: AgentCard[]): string {
  const named = crew.filter((agent) => agent.repositoryId === repository.id);
  if (named.length === 0) return "nobody yet";
  return named.map((agent) => agent.name).join(", ");
}

/**
 * The directories a crew may write code in, and who in it may.
 *
 * ## There is no engineer here, and that is the point
 *
 * The obvious control is a switch on an agent that says it is an engineer, or a
 * specialist, or whatever the tier gets called. It is not here, because a tier
 * carries no information this list does not. An agent with no repository is
 * offered nothing that reaches a working tree; an "engineer" with no repository
 * is the same agent with a badge. Two answers to one question is two places for
 * it to be wrong, and only one of them is the one the runtime reads.
 *
 * The part a tier feels like it would buy is already content and already ships.
 * The cafeteria hires a Software Engineer, a Code Reviewer and a QA Tester,
 * each with its own brief, and the model field suggests programming models for
 * an agent whose words read that way. Designating an engineer is hiring one and
 * giving it a directory. Nothing under here has to know the word.
 *
 * ## Linking is here; handing out is on the agent
 *
 * Two decisions, and they are split across two panels rather than stacked in
 * one, because they are asked at different moments. "What codebases does this
 * crew have" is a crew fact, set up once and edited when a directory moves.
 * "What can this agent work on" is asked every time somebody hires an engineer,
 * and it is answered where an agent is decided, beside its model and its
 * instructions. `AgentRepositories` is that panel.
 *
 * A newly linked repository therefore reaches nobody, and the line under each
 * row says so. That is a state to pass through rather than one to hide: a
 * crew's source handed to every agent at the moment it was linked is the
 * accident this feature most has to avoid.
 *
 * Who has what stays here as a read, and only as a read. Auditing is a real
 * question, and answering it should not mean opening six agents one at a time.
 * The same reason the plugin panel says who a sign-in is offered to.
 *
 * There is no "every agent" button here and there is one on the plugins panel.
 * An agent hired next week must not inherit the operator's own source. Names,
 * always, and only on the agent.
 *
 * The heading and the sentence explaining all of that are the section's, not
 * this panel's. It has a section of its own in the group editor rather than a
 * third block under Plugins, so drawing its own title under the section's would
 * be the same word twice at two sizes.
 *
 * ## Why the harness is a choice on the row and not a setting
 *
 * Because a subscription is spent by the program it was issued to. An operator
 * whose ChatGPT plan is spent cannot be helped by configuring `pi`: they need
 * Claude Code, which is a different program with a different sign-in. So the
 * choice is between programs, it sits beside the note because it is the same
 * kind of fact about how work happens in this directory, and one codebase can
 * answer it differently from the next.
 *
 * The one it offers that is not installed is offered anyway, disabled, with the
 * command that installs it underneath. Hiding it makes an operator whose plan
 * has just run out conclude the app cannot do the thing it can do, and enabling
 * it stores a choice whose only symptom is a coding job that never starts,
 * reported to an agent forty minutes later.
 */
export function RepositoryList({ groupId, crew }: Props) {
  const capabilities = useStore((s) => s.capabilities);
  const [repositories, setRepositories] = useState<Repository[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState<Omit<RepositoryDraft, "groupId">>({
    path: "",
    name: "",
    note: "",
    harness: "pi",
    gate: "open",
    bench: "own",
  });
  const [editing, setEditing] = useState<RepositoryId | null>(null);
  const [edit, setEdit] = useState<{
    name: string;
    note: string;
    harness: Harness;
    gate: Gate;
    bench: Bench;
  }>({
    name: "",
    note: "",
    harness: "pi",
    gate: "open",
    bench: "own",
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [machine, setMachine] = useState<HarnessOnMachine[] | null>(null);
  const pathRef = useRef<HTMLInputElement>(null);

  // The path is the only field that has to be filled in, so it takes the
  // cursor. Same reason the credential form focuses its first box.
  useEffect(() => {
    if (adding) pathRef.current?.focus();
  }, [adding]);

  const load = useCallback(async () => {
    try {
      setRepositories(await api.groupRepositories(groupId));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setRepositories([]);
    }
  }, [groupId]);

  useEffect(() => {
    void load();
  }, [load]);

  // Two process spawns, asked once when the panel opens. It is a question about
  // what is on the machine rather than about the workspace, so nothing in the
  // app can invalidate it: an operator who installs one while this is open sees
  // it the next time they open the panel.
  useEffect(() => {
    api
      .codingHarnesses()
      .then(setMachine)
      // A check that could not run must not disable both choices. Unknown reads
      // as installed: the refusal a job gives already names the install command,
      // and the failure mode of guessing the other way is a panel that refuses
      // to save the thing the operator can see working in their own terminal.
      .catch(() => setMachine(null));
  }, []);

  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await action();
      await load();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const reset = () => {
    setAdding(false);
    setDraft({ path: "", name: "", note: "", harness: "pi", gate: "open", bench: "own" });
  };

  const add = () =>
    void run("add", () =>
      api.createRepository({ groupId, ...draft } satisfies RepositoryDraft),
    ).then((ok) => ok && reset());

  if (repositories === null) return <p className="field__hint">Loading repositories…</p>;

  return (
    <div className="access">
      {repositories.map((repository) => (
        <div className="access__item" key={repository.id}>
          <div className="access__row">
            <strong className="access__name">{repository.name}</strong>
            <span className="access__where">{repository.path}</span>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy !== null}
              onClick={() => {
                setEditing(editing === repository.id ? null : repository.id);
                setEdit({
                  name: repository.name,
                  note: repository.note,
                  harness: repository.harness,
                  gate: repository.gate,
                  bench: repository.bench,
                });
              }}
            >
              {editing === repository.id ? "Done" : "Edit"}
            </button>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy !== null}
              onClick={() => void run(repository.id, () => api.deleteRepository(repository.id))}
            >
              Unlink
            </button>
          </div>

          {editing === repository.id ? (
            <>
              <div className="access__row">
                <input
                  className="input input--slim"
                  placeholder="what you call it"
                  value={edit.name}
                  onChange={(event) => setEdit({ ...edit, name: event.target.value })}
                />
                <input
                  className="input input--slim"
                  placeholder="run ./scripts/ci.sh before you finish"
                  value={edit.note}
                  onChange={(event) => setEdit({ ...edit, note: event.target.value })}
                />
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy !== null || !edit.name.trim()}
                  onClick={() =>
                    void run(`${repository.id}-edit`, () =>
                      api.updateRepository(
                        repository.id,
                        edit.name,
                        edit.note,
                        edit.harness,
                        edit.gate,
                        edit.bench,
                      ),
                    ).then((ok) => ok && setEditing(null))
                  }
                >
                  Save
                </button>
              </div>
              <HarnessChoice
                chosen={edit.harness}
                machine={machine}
                disabled={busy !== null}
                onChoose={(harness) => {
                  // The stored name and note, not the boxes above. A half-typed
                  // rename is not a thing the operator asked to save, and this
                  // click is not the gesture that saves it.
                  setEdit({ ...edit, harness });
                  void run(`${repository.id}-harness`, () =>
                    api.updateRepository(
                      repository.id,
                      repository.name,
                      repository.note,
                      harness,
                      repository.gate,
                      edit.bench,
                    ),
                  );
                }}
              />
              <GateChoice
                chosen={edit.gate}
                harness={edit.harness}
                machine={machine}
                disabled={busy !== null}
                onChoose={(gate) => {
                  // The stored name and note, for the reason the harness above
                  // uses them: a half-typed rename is not what this click saves.
                  setEdit({ ...edit, gate });
                  void run(`${repository.id}-gate`, () =>
                    api.updateRepository(
                      repository.id,
                      repository.name,
                      repository.note,
                      edit.harness,
                      gate,
                      edit.bench,
                    ),
                  );
                }}
              />
              <BenchChoice
                chosen={edit.bench}
                disabled={busy !== null}
                onChoose={(bench) => {
                  // The stored name and note, for the reason the two above use
                  // them: a half-typed rename is not what this click saves.
                  setEdit({ ...edit, bench });
                  void run(`${repository.id}-bench`, () =>
                    api.updateRepository(
                      repository.id,
                      repository.name,
                      repository.note,
                      edit.harness,
                      edit.gate,
                      bench,
                    ),
                  );
                }}
              />
              {/* Said where the field is, because the field is next to a path
                  and the two look like they change the same kind of thing. */}
              <p className="field__hint">
                The path is not editable. A different directory is a different repository: whoever
                you gave this one was given that directory, so moving it here would move their
                boundary without saying so.
              </p>
            </>
          ) : (
            repository.note && <p className="field__hint">{repository.note}</p>
          )}

          {/* The harness is on the row and not only behind Edit, because the
              question it answers is asked about the list: which of these
              directories is running on the plan that has stopped paying. */}
          <p className="field__hint">
            Code here is written by {labelOf(repository.harness)}
            {repository.bench === "own"
              ? ", each agent in a worktree of its own"
              : ", in this directory"}
            . Worked in by {worksIn(repository, crew)}. Put an agent in it by dragging it onto this
            repository in the rail.
          </p>
        </div>
      ))}

      {adding ? (
        <div className="access__item">
          <div className="access__row">
            <input
              className="input input--mono"
              placeholder="/Users/you/dev/your-project"
              ref={pathRef}
              value={draft.path}
              onChange={(event) => setDraft({ ...draft, path: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === "Enter" && draft.path.trim()) add();
              }}
            />
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy !== null || !draft.path.trim()}
              onClick={add}
            >
              Link
            </button>
            {/* Beside the thing it cancels rather than in a heading above the
                list. The section owns the heading now, and a Cancel a panel
                away from the form is one an operator hunts for. */}
            <button type="button" className="btn btn--ghost btn--small" onClick={reset}>
              Cancel
            </button>
          </div>
          <div className="access__row">
            <input
              className="input input--slim"
              placeholder="what you call it (optional)"
              value={draft.name}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
            />
            <input
              className="input input--slim"
              placeholder="run ./scripts/ci.sh before you finish (optional)"
              value={draft.note}
              onChange={(event) => setDraft({ ...draft, note: event.target.value })}
            />
          </div>
          <HarnessChoice
            chosen={draft.harness}
            machine={machine}
            disabled={busy !== null}
            onChoose={(harness) => setDraft({ ...draft, harness })}
          />
          <GateChoice
            chosen={draft.gate}
            harness={draft.harness}
            machine={machine}
            disabled={busy !== null}
            onChoose={(gate) => setDraft({ ...draft, gate })}
          />
          <p className="field__hint">
            The full path to the directory, which has to be the root of a git repository. Git is the
            undo: it is the reason an agent can be turned loose in there at all. The note is read by
            every agent that has it, on every turn.
          </p>
        </div>
      ) : capabilities.localDirectories ? (
        <button type="button" className="btn btn--small" onClick={() => setAdding(true)}>
          Link a repository
        </button>
      ) : (
        // Said in place of the button rather than after the path is typed: a
        // repository is a directory on the machine the workspace runs on, and
        // on a server that machine is not the operator's.
        <p className="field__hint">
          This workspace runs on a server, so it cannot open a directory on your own machine, and a
          repository is a directory. Give this crew a repository from a local workspace.
        </p>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
