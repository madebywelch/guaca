import type { ReactNode } from "react";

import type { DropTarget } from "../lib/rail";
import type { AgentCard, RepoStatus, Repository } from "../lib/types";

interface Props {
  /** This crew's repositories, already filtered. */
  repositories: Repository[];
  /** This crew's agents, in rail order. */
  crew: AgentCard[];
  /** What each one is doing, by id. Absent while it is being read for the
   *  first time, and for a directory that could not be read at all. */
  status: Record<string, RepoStatus>;
  /** Draws one agent as the rail draws every other row. */
  row: (agent: AgentCard) => ReactNode;
  isOver: (target: DropTarget) => boolean;
  onDragOver: (target: DropTarget) => void;
  onDragLeave: () => void;
  /** Whether a drag is in flight, which is when these become targets. */
  dragging: boolean;
}

/**
 * The codebases a crew works in, as headings with their agents under them.
 *
 * ## Why they are here and not in settings
 *
 * A repository was a row in a settings page, which is where a thing goes when
 * the app considers it configuration. For an operator whose day is repositories
 * that is the wrong shelf: which codebases this crew has, and who is on each,
 * is the first question asked on opening the app and it was three clicks deep.
 *
 * ## Why this is a tree and not two lists
 *
 * An agent works in at most one repository, so every agent has exactly one
 * place in the rail: under its codebase, or under the crew if it has none.
 * That is the whole reason the exclusive rule is worth its cost. A
 * many-to-many cannot be drawn as a tree, and the version that tried put every
 * name in two places at once and left the operator working out which of them
 * was the real row.
 *
 * The rows here are the rail's own rows, passed in rather than reinvented, so
 * an agent under a repository is the same row with the same menu, the same
 * activity and the same drag as an agent anywhere else. Only its indent
 * differs.
 *
 * ## Why they are not circles in the crews column
 *
 * That column is crews, and dropping an agent on one moves it between crews.
 * Dropping here moves it between codebases inside the crew it is already in.
 * Both are moves, which is what makes the gesture learnable, but they move
 * different things, so they are different furniture in different places.
 */
export function RailRepositories({
  repositories,
  crew,
  status,
  row,
  isOver,
  onDragOver,
  onDragLeave,
  dragging,
}: Props) {
  if (repositories.length === 0) return null;

  return (
    <div className="rail__repos">
      {repositories.map((repository) => {
        // Only agents still in this crew. The column outlives a move between
        // crews until something clears it, and a name here the runtime would
        // refuse is the one thing a panel about access must not draw.
        const inside = crew.filter((agent) => agent.repositoryId === repository.id);
        return (
          <div
            key={repository.id}
            className="rail__repo"
            data-over={isOver({ kind: "repository", id: repository.id }) ? "true" : undefined}
            onPointerEnter={() => onDragOver({ kind: "repository", id: repository.id })}
            onPointerLeave={onDragLeave}
          >
            <div className="rail__repo-head" title={repository.path}>
              <span className="rail__repo-mark" aria-hidden="true" />
              <span className="rail__repo-name">{repository.name}</span>
              <span className="rail__repo-count">{inside.length || ""}</span>
            </div>

            <RepoState status={status[repository.id]} />

            {inside.length > 0 ? (
              <div className="rail__repo-crew">{inside.map(row)}</div>
            ) : (
              // Says which state it is in rather than drawing an empty gap: a
              // repository linked and given to nobody is ordinary, and blank
              // reads as still loading.
              <p className="rail__repo-empty">
                {dragging ? "drop an agent here" : "nobody works here yet"}
              </p>
            )}
          </div>
        );
      })}
    </div>
  );
}

/**
 * What one repository is doing, in a line narrow enough for the rail.
 *
 * Nothing is drawn until the first read comes back. A row that said `main,
 * clean` before anything had been asked would be a claim about a directory
 * nobody had looked at, and the operator has no way to tell that apart from an
 * answer.
 *
 * The pieces are absent rather than zeroed, and each absence means something
 * different. No `dirty` is a clean tree. No arrows is a branch level with its
 * upstream, or one with no upstream at all, which is why the arrows are drawn
 * off `upstream` and not off the counts. No pull request count is `gh` not
 * installed, not signed in, or a repository GitHub has never heard of: drawn as
 * `0 PR` each of those would say there is nothing waiting for review, which is
 * a claim this app has no basis for.
 */
function RepoState({ status }: { status: RepoStatus | undefined }) {
  if (!status) return null;

  return (
    <p className="rail__repo-state">
      <span
        className="rail__repo-branch"
        data-detached={status.detached ? "true" : undefined}
        title={status.detached ? "detached HEAD" : `on ${status.branch}`}
      >
        {status.branch}
      </span>

      {status.dirty > 0 && (
        <span className="rail__repo-dirty" title={`${status.dirty} uncommitted`}>
          {status.dirty}&#8202;&#9679;
        </span>
      )}

      {status.upstream && status.ahead > 0 && (
        <span title={`${status.ahead} to push`}>&#8593;{status.ahead}</span>
      )}
      {status.upstream && status.behind > 0 && (
        <span title={`${status.behind} to pull`}>&#8595;{status.behind}</span>
      )}

      {status.pullRequests !== null && status.pullRequests > 0 && (
        <span
          className="rail__repo-prs"
          title={`${status.pullRequests} open pull request${status.pullRequests === 1 ? "" : "s"}`}
        >
          {status.pullRequests} PR
        </span>
      )}
    </p>
  );
}
