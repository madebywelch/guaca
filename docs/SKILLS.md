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

## A workspace starts with six, and is given them once

`skills::STARTERS` is six general procedures every workspace starts with:
`grounded-citations`, `grill-me`, `one-three-one`, `systematic-debugging`,
`document-to-action-items` and `meeting-action-items`. They are adapted from
Hermes Agent's own (MIT, and `src-tauri/skills/defaults/LICENSE` names the
commit and the two further upstreams), because Hermes ships a curated set that
already works and a person who has to curate one from nothing does not do it.

What was left out is the half of Hermes's set that names a program, a platform
or a tool an agent here does not have: iMessage and Find My, the coding CLIs,
anything built on its scripts. What was kept was rewritten for this app's tool
surface, and conditionally where it has to be, since most agents have no
browser, no repository and no computer. A test fails if a starter names one of
Hermes's tools, mentions a script or reads as another harness's.

They are copied into the operator's scope rather than compiled in as Guaca's,
because they are the operator's to edit and delete, and `boot.rs` copies them
through `Skills::offer`. That writes each name to `skills/.defaults` as it is
offered, so a starter the operator deleted is not put back on the next start,
one they already had under that name is left as theirs, and a later build that
ships a seventh offers only the seventh. Each carries `LICENSE.txt`, so the
notice travels wherever the skill is copied.

They are copied from the binary, not downloaded. A workspace opened offline on
a box gets the same six as one opened at a desk, and what reaches every crew
is a file that was reviewed in this repository.

## skills.sh is where the rest come from, and only the operator adds

skills.sh is Vercel's directory: every skill its CLI installs, ranked by how
many times, with a copy of each one's files. Settings and a crew's settings
open it inside the skills section (`SkillDirectory.tsx`): the three rankings
(most installed, trending, hot), a search, and a skill opened in place with its
whole `SKILL.md`, the files it carries and what skills.sh's audit partners said
about it. `skills_sh.rs` is the client.

The documented API needs a Vercel deployment token, so the client calls what
skills.sh's own pages and its own CLI call without one. The top of
`skills_sh.rs` lists the four, and `tests/skills_sh.rs` has a live half that
fails when any of them stops answering the way this build reads it.

Nothing is added unread. The preview carries a SHA-256 over every path and its
contents, the add sends it back, and the add fetches the skill again and
refuses when the hash moved: a skill is instructions a crew will follow, and a
version the operator did not read should not arrive in place of the one they
did. The hash is computed here rather than taken from skills.sh, whose own
`hash` field covers something this build cannot check.

An agent cannot reach skills.sh at all. The directory is a public, mostly
unvetted list (researchers found hundreds of malicious skills in its largest
neighbor in February 2026), and the likeliest agent to go looking in it is one
that has just read a page telling it to. The operator browses, reads and adds;
the manual tells agents to name a skill that would help and ask.

What is added lands in the scope the section belongs to, whole: `SKILL.md`
exactly as its author wrote it, every file beside it, and `.origin`, the page it
came from. `Skills::install` assembles it in a hidden directory and renames it
into place, never over a skill already there. A skill a site publishes itself
rather than from a GitHub repository is listed and not addable: skills.sh keeps
no copy of it, and fetching it from the site is a second protocol this does not
speak yet.

## A skill carries files, and an agent reads the ones it lists

Other harnesses let a skill carry `references/`, `templates/` and `scripts/`
beside its `SKILL.md`, and most skills worth adding use them. `Skill::files`
lists them, `view` names them after the instructions, and `view` with `file`
reads one. Only a listed path is read, the listing does not follow symlinks,
and a path that climbs is refused before the disk is touched, so nothing a
skill carries can point a read outside it.

Nothing a skill carries is run. A script is a file an agent can read; running
it would mean running someone else's code on the operator's machine because a
document said to, and the door for running code is a repository's (`shell`,
`code`), which the operator gives on purpose.

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

- **Running a skill's scripts.** Read, never run; see above.
- **Sites that publish their own skills.** Listed from skills.sh, not added.
- **Updating an added skill.** `.origin` says where it came from; nothing
  checks it for a newer version. Delete it and add it again.
- **Per-agent enablement.** Every agent in a crew sees the crew's skills and the
  operator's. An operator who wants a skill for one agent writes it into that
  agent's instructions.
- **Export with a crew.** Import / export moves a crew's agents and settings,
  not its skills directory.
