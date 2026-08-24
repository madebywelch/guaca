import type { DropTarget } from "../lib/rail";
import type { AgentCard, AgentId, Repository } from "../lib/types";

interface Props {
  /** This crew's repositories, already filtered. */
  repositories: Repository[];
  /** This crew, for turning the ids on a repository into names. */
  crew: AgentCard[];
  isOver: (target: DropTarget) => boolean;
  onDragOver: (target: DropTarget) => void;
  onDragLeave: () => void;
  /** Whether a drag is in flight, which is when these become targets. */
  dragging: boolean;
}

/**
 * The codebases a crew works in, in the rail, above the crew.
 *
 * ## Why they are here and not in settings
 *
 * A repository was a row in a settings page, which is where a thing goes when
 * the app considers it configuration. For an operator whose day is repositories
 * that is the wrong shelf: which codebases this crew has, and who is on each,
 * is the first question asked on opening the app and it was three clicks deep.
 *
 * ## Why they are not circles in the crews column
 *
 * The column on the far left is crews, and dropping an agent on one *moves* it,
 * because an agent is in exactly one crew. Dropping on a repository *grants*,
 * because an agent can work in several. Two gestures that look identical and
 * mean opposite things is how an operator loses an agent while trying to give
 * it a codebase.
 *
 * So repositories sit inside the crew, drawn as their own furniture rather than
 * as more rows, and the difference between the two drops is visible before the
 * hand commits to either.
 *
 * ## Why an agent is named here and still drawn below
 *
 * A crew's roster is every agent in it, once. These are a second view over some
 * of the same agents, and the honest way to draw a many-to-many is to let a
 * name appear in both places rather than to move a row out of the roster into a
 * section: an agent on two repositories would have to be in two sections at
 * once, and an agent on none would look like it had been left out of the crew.
 * The names here are text rather than rows for the same reason. A row is a
 * channel you open; this is a statement about who can reach the code.
 */
export function RailRepositories({
  repositories,
  crew,
  isOver,
  onDragOver,
  onDragLeave,
  dragging,
}: Props) {
  if (repositories.length === 0) return null;

  const nameOf = (id: AgentId) => crew.find((agent) => agent.id === id)?.name;

  return (
    <div className="rail__repos">
      {repositories.map((repository) => {
        // Only agents still in this crew. Reach outlives a move until something
        // clears it, and a name here that the runtime would refuse is the one
        // thing a permission panel must not draw.
        const on = repository.reach.map(nameOf).filter(Boolean) as string[];
        return (
          <div
            key={repository.id}
            className="rail__repo"
            data-over={isOver({ kind: "repository", id: repository.id }) ? "true" : undefined}
            onPointerEnter={() => onDragOver({ kind: "repository", id: repository.id })}
            onPointerLeave={onDragLeave}
            title={repository.path}
          >
            <div className="rail__repo-head">
              <span className="rail__repo-name">{repository.name}</span>
            </div>
            <p className="rail__repo-who">
              {on.length > 0 ? (
                on.join(", ")
              ) : (
                // The empty state is an instruction, because this is the one
                // moment the operator can act on: a repository nobody has is
                // linked and not handed out, which is a state to pass through
                // rather than a failure.
                <span className="rail__repo-nobody">
                  {dragging ? "drop to give it this" : "nobody yet"}
                </span>
              )}
            </p>
          </div>
        );
      })}
    </div>
  );
}
