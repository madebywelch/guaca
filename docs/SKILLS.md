# Skills

A skill is a markdown document of instructions for one kind of task. An agent
is told each skill's name and the one line saying when to load it, and reads
the rest with the `skill` tool when a task fits. `domain/skill.rs` holds the
rules, `skills.rs` holds the disk, and `src-tauri/skills/guaca/SKILL.md` is the
one Guaca ships.

## A name in every prompt, a body on demand

Memory is in every prompt because it is what one agent knows and cannot look
up. A procedure is the opposite: most turns do not need it, and a crew with
forty would pay for forty on every call. So the prompt carries an index, capped
at `LISTED` lines, and the body is a tool call away. The index is appended to
the system message after assembly (`prompt::add_skills`), read from disk on
every turn, so a skill a crewmate wrote a moment ago is offered on the next
turn without a restart.

The format is the `SKILL.md` other harnesses read: front matter with `name` and
`description`, then markdown. A skill written here can be carried to Claude
Code or Hermes, and one written there can be dropped into a directory here.
Front matter is read for those two keys only, in the three spellings a person
writes them in (plain, quoted, folded); anything else in it is kept and
ignored. Writes quote the description, because a plain YAML scalar ends at the
first `: ` and a description is prose.

## Three places, and who may write each

| Scope | On disk | Read by | Written by |
|---|---|---|---|
| Bundled | compiled in | every agent | nobody |
| Workspace | `skills/workspace/<name>/SKILL.md` | every agent | the operator |
| Crew | `skills/crews/<group>/<name>/SKILL.md` | that crew | the operator and that crew's agents |

A skill is instructions another agent will follow, so where it lives is a trust
decision. An agent writes only to its own crew: a skill one crew's agent could
put in front of another crew's agents is a way to hand instructions across the
wall every other crew-scoped thing keeps, and the likeliest author of one is an
agent that has just read a hostile page. The crew is read off the calling
agent's card at dispatch, never from the call, the way the calendar's is.

A crew's skill shadows the operator's of the same name, so a crew can keep its
own variant of a house procedure. Guaca's names are reserved and shadow
nothing. Disbanding a crew removes its directory.

## The manual is a skill, and a test keeps it true

`guaca` is the app described to the agents that run in it: where it runs, what
the operator sees, every Settings pane and crew section, the limits, skills,
plugins, secrets, routines and updates. The prompt names it in the section's
own sentence, because "where do I change the model" is a question an agent
answers before it would think to look for a tool.

It is compiled in rather than copied to disk, so an updated host serves the
manual of the build it runs. `src/lib/manual.test.ts` reads it beside the two
settings dialogs and fails when a pane is drawn that the manual does not name.
An agent that sends the operator to a pane that was renamed is worse than one
that says it does not know.

## Connectors, tools and skills are three sections of a crew's settings

A crew's settings show the three things an agent can use side by side.
**Connectors** are MCP servers the crew signed in to, each with its own tools
and per-agent answers. **Tools** is Guaca's own functions, read from the
definitions agents are sent (`tools::catalog`), each with what an agent has to
be given before it is offered one; `offered` decides that and `needs` says it,
and a test holds them together. **Skills** are the documents on this page. A
skill is not a tool: it is instructions an agent reads with one.

## What is not built

- **Supporting files.** Other harnesses allow `references/` and `scripts/`
  beside `SKILL.md`. The directory layout leaves room for them; nothing reads
  them yet.
- **Per-agent enablement.** Every agent in a crew sees the crew's skills and the
  operator's. An operator who wants a skill for one agent writes it into that
  agent's instructions.
- **Export with a crew.** Import / export moves a crew's agents and settings,
  not its skills directory.
