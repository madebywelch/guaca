# Skills

The `skill` tool, the three scopes, the starters, skills.sh, and the manual.
`docs/SKILLS.md`, then `domain/skill.rs`, `skills.rs` and `skills_sh.rs`.

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
- **A skill from elsewhere is not held to `MAX_DESCRIPTION`.** More than half
  of the forty most installed on skills.sh describe themselves in more than 300
  characters. `Package` accepts any length and the read cuts it, as it cuts a
  hand-written file's; refusing them refuses most of the directory. The body
  is still refused past `MAX_BODY`, because a procedure cut short is a
  different procedure.
- **The documented skills.sh API is the wrong one.** `/api/v1/` answers 401
  without a Vercel OIDC token, which a desktop app cannot mint. `skills_sh.rs`
  calls what the site and its CLI call. Moving it to `/api/v1/` looks like a
  cleanup and breaks every request; the live half of `tests/skills_sh.rs` is
  what says when the undocumented ones move.
- **A rename moves the directory.** It used to write the new name and delete
  the old, which was fine while a skill was one file and loses every reference
  a skill carries now. `Skills::rename` moves it, and only a rename onto a name
  already taken falls back to write and delete.
- **Editing an added skill rewrites its `SKILL.md`.** `Clean::render` writes
  `name` and `description` only, so the author's license and author lines in
  its front matter do not survive an edit made here. Its `.origin` and any
  `LICENSE` file it carries do.
- **A starter is offered once, by name.** `skills/.defaults` is the ledger.
  Seeding whenever a starter is missing puts back one the operator deleted;
  making the starters bundled takes away the operator's right to edit or
  delete them. Both were considered and are wrong.
