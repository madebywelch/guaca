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
import { GitAuthor } from "./GitAuthor";
import { RepositoryConnection } from "./RepositoryConnection";

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
        The coding tool runs on the connected backend and uses its own account and model settings.
        {machine?.find((row) => row.harness === chosen)?.signedIn === false && (
          <span>
            {" "}
            {labelOf(chosen)} needs to be signed in on that backend before it can write code. Your
            Guaca chat sign-in does not sign in the coding tool.
          </span>
        )}
        {chosen === "codex" &&
          machine?.find((row) => row.harness === chosen)?.bridged === false && (
            <span> Update Codex to version 0.153 or newer for corrections and push approvals.</span>
          )}
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
        disabled={disabled || (!reachable && chosen !== "askBeforePushing")}
        onChange={(event) => onChoose(event.target.checked ? "askBeforePushing" : "open")}
      />
      <span>
        <span className="field__label">Ask me before pushing</span>
        <span className="field__hint">
          Ask for approval before a push, pull request, merge or release. Other commands run with
          the repository access available on the backend. This is an approval step, not a sandbox.
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
  const [githubAvailable, setGithubAvailable] = useState<boolean | null>(null);
  const [access, setAccess] = useState<"automatic" | "github" | "token">("automatic");
  const [source, setSource] = useState<"directory" | "remote">(
    capabilities.localFiles ? "directory" : "remote",
  );
  const directory = source === "directory" && capabilities.localDirectories;
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

  useEffect(() => {
    api
      .githubAppAvailable()
      .then(setGithubAvailable)
      .catch(() => {
        setGithubAvailable(false);
      });
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
    setAccess("automatic");
    setCloning({ remote: "", credential: "", username: "" });
    setDraft({ path: "", name: "", note: "", harness: "pi", gate: "open", bench: "own" });
  };

  const add = () =>
    void run("add", () =>
      api.createRepository({ groupId, ...draft } satisfies RepositoryDraft),
    ).then((ok) => ok && reset());

  /** The clone form's own two fields; everything else is shared with `draft`. */
  const [author, setAuthor] = useState({ name: "", email: "" });
  const [cloning, setCloning] = useState({ remote: "", credential: "", username: "" });

  const githubRemote = /^https:\/\/github\.com\//i.test(cloning.remote.trim());
  const githubApp = githubRemote && githubAvailable === true && access !== "token";

  const addClone = () =>
    void run("add", () =>
      (githubApp ? api.createGithubRepository : api.createRepository)({
        groupId,
        ...draft,
        path: "",
        remote: cloning.remote.trim(),
        author: author.name.trim() || author.email.trim() ? author : undefined,
        credential: githubApp ? undefined : cloning.credential.trim() || undefined,
        username: cloning.username.trim() || undefined,
      } satisfies RepositoryDraft),
    ).then((ok) => {
      if (ok) {
        reset();
        setCloning({ remote: "", credential: "", username: "" });
        setAuthor({ name: "", email: "" });
      }
    });

  if (repositories === null) return <p className="field__hint">Loading repositories…</p>;

  return (
    <div className="access">
      {repositories.map((repository) => (
        <div className="access__item" key={repository.id}>
          <div className="access__row">
            <strong className="access__name">{repository.name}</strong>
            <span className="access__where">{repository.remote ?? repository.path}</span>
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

          <RepositoryConnection id={repository.id} />

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
          {capabilities.localDirectories && (
            <label className="field">
              <span className="field__label">Repository source</span>
              <select
                className="input"
                aria-label="Repository source"
                value={source}
                onChange={(event) => {
                  setSource(event.target.value as "directory" | "remote");
                  setError(null);
                }}
              >
                <option value="remote">Clone from GitHub or another Git service</option>
                <option value="directory">Use a folder on the backend</option>
              </select>
            </label>
          )}
          <label className="field">
            <span className="field__label">
              {directory ? "Repository folder" : "Repository URL"}
            </span>
            <input
              className="input input--mono"
              ref={pathRef}
              aria-label={directory ? "Directory on backend" : "Remote to clone"}
              placeholder={
                directory
                  ? capabilities.localFiles
                    ? "/Users/you/dev/your-project"
                    : "/workspace/your-project"
                  : "https://github.com/you/your-project.git"
              }
              value={directory ? draft.path : cloning.remote}
              onChange={(event) => {
                if (directory) setDraft({ ...draft, path: event.target.value });
                else setCloning({ ...cloning, remote: event.target.value });
                setError(null);
              }}
            />
            <span className="field__hint">
              {directory
                ? "Use the path on the backend. For Docker, this must be a folder mounted into the container."
                : "Guaca keeps its own checkout on the backend. Your Mac’s existing checkout is unchanged."}
            </span>
          </label>
          {!directory && (
            <>
              <label className="field">
                <span className="field__label">Repository access</span>
                <select
                  className="input"
                  aria-label="Repository access"
                  disabled={githubAvailable === null || busy !== null}
                  value={githubApp ? "github" : "token"}
                  onChange={(event) => {
                    setAccess(event.target.value as "github" | "token");
                    setCloning({ ...cloning, credential: "" });
                    setError(null);
                  }}
                >
                  <option value="github" disabled={!githubAvailable || !githubRemote}>
                    {githubAvailable
                      ? "GitHub App connected to this host"
                      : "GitHub App (not configured on this host)"}
                  </option>
                  <option value="token">Access token or existing Git credentials</option>
                </select>
                <span className="field__hint">
                  {githubAvailable === null
                    ? "Checking GitHub connection…"
                    : githubApp
                      ? "Uses this host’s GitHub App to clone and push. A guaca.bot account is not required. After adding the repository, sign in under Git access so commits and pull requests use your account."
                      : "Private repositories need Git credentials. Your Guaca or coding-tool sign-in does not grant repository access. Public repositories need no token."}
                </span>
              </label>
              {!githubApp && (
                <>
                  <label className="field">
                    <span className="field__label">Access token</span>
                    <input
                      className="input"
                      type="password"
                      autoComplete="off"
                      spellCheck={false}
                      aria-label="Access token"
                      placeholder="Required for private HTTPS repositories without existing Git credentials"
                      value={cloning.credential}
                      onChange={(event) =>
                        setCloning({ ...cloning, credential: event.target.value })
                      }
                    />
                  </label>
                  <label className="field">
                    <span className="field__label">Git username (if required)</span>
                    <input
                      className="input"
                      aria-label="Git username"
                      autoComplete="off"
                      value={cloning.username}
                      onChange={(event) => setCloning({ ...cloning, username: event.target.value })}
                    />
                  </label>
                </>
              )}
            </>
          )}
          <div className="field">
            <span className="field__label">Coding tool</span>
            <HarnessChoice
              chosen={draft.harness}
              machine={machine}
              disabled={busy !== null}
              onChoose={(harness) => setDraft({ ...draft, harness })}
            />
          </div>
          <GateChoice
            chosen={draft.gate}
            harness={draft.harness}
            machine={machine}
            disabled={busy !== null}
            onChoose={(gate) => setDraft({ ...draft, gate })}
          />
          <details>
            <summary>More options</summary>
            <label className="field">
              <span className="field__label">Display name</span>
              <input
                className="input"
                placeholder="what you call it (optional)"
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              />
            </label>
            <label className="field">
              <span className="field__label">Instructions for assigned agents</span>
              <input
                className="input"
                placeholder="run ./scripts/ci.sh before you finish (optional)"
                value={draft.note}
                onChange={(event) => setDraft({ ...draft, note: event.target.value })}
              />
            </label>
            {!directory && (
              <GitAuthor author={author} disabled={busy !== null} onChange={setAuthor} />
            )}
          </details>
          <div className="access__row">
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={
                busy !== null ||
                (directory
                  ? !draft.path.trim()
                  : !cloning.remote.trim() || githubAvailable === null)
              }
              onClick={directory ? add : addClone}
            >
              {busy === "add" ? "Adding…" : "Link"}
            </button>
            <button
              type="button"
              className="btn btn--ghost btn--small"
              disabled={busy !== null}
              onClick={reset}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button type="button" className="btn btn--small" onClick={() => setAdding(true)}>
          Link a repository
        </button>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
