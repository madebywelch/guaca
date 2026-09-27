# Skills

The `skill` tool, the three scopes, and the manual. `docs/SKILLS.md`, then
`domain/skill.rs` and `skills.rs`.

- **The directory is the name.** A hand-written file whose front matter says
  another name is still found, listed and written by its directory, because
  that is what every write and delete addresses. `check_name` refuses dots and
  separators for the same reason: `../x` is a directory outside the scope.
- **Read leniently, write strictly.** A file an operator wrote past a limit is
  served cut, with a line saying so; only a write is refused for size. A
  broken file is skipped with a warning rather than failing the whole list.
- **The index is appended, not threaded.** `system_prompt` takes sixteen
  arguments and thirty call sites. `prompt::add_skills` appends to the system
  message after assembly, the way `add_decisions` adds its own message.
- **"Stated skills" are something else.** An agent card's `skills` are tags
  describing what it is good at, and the prompt calls them "stated skills" in
  text the evals tuned. File skills are "skills" and a `skill` tool. Renaming
  either side changes a tuned prompt.
- **Guaca's manual is checked against the UI.** Add a pane to Settings or a
  section to a crew's settings and `manual.test.ts` fails until
  `src-tauri/skills/guaca/SKILL.md` names it in bold.
