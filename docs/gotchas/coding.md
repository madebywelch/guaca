# Writing code

An agent's terminal, the three doors into it, the gate in front of every door,
and the three harnesses that write the code. `docs/CODING.md` is the long
version; `domain/terminal.rs`, `terminal.rs`, `coding/`, `shell.rs`, `repo.rs`
and `programs.rs` are the code.

- **A repository row was bookkeeping about a directory the agent already
  owned.** By the end every agent worked in a worktree of its own and the host
  ran in a container, so the row's link, clone, token, identity, harness, gate
  and worktree policy were a setup an operator had to get through before an
  agent could write a line, about a directory nobody else used. The terminal is
  that directory, owned outright. A crew-level list of linked repositories
  would be a second answer to where an agent works; the agent clones what it
  needs. `docs/CODING.md`.
- **A terminal has three doors and one gate, and the gate is one function.**
  `code` hands a brief to a harness for minutes, `shell` runs one line and
  answers in the turn, and `read`, `write` and `edit` work on files. The second
  exists because the first was the only way in, which made `gh pr merge` cost a
  coding job and made an agent whose harness would not start report that it
  had no shell at all. What the doors must not add is a second answer to *what
  counts as outward-facing*, so `shell` and every harness ask
  `coding::bridge::outward` and park through `Runtime::ask_about_push`. Two
  readings of one gate is a gate an agent walks around by picking the other
  tool.
- **The gate follows a `cd`.** Read from the top of the terminal, `cd guaca &&
  npm run release` names a `package.json` that is not there, so a gate that was
  switched on stopped nothing the first time an agent held two repositories.
  `scripts_in` carries where each segment runs, and a `cd` out of the tree
  leads somewhere nothing is read.
- **`read` is a disk and `read_file` is an attachment, one word apart.** Each
  description says which it is in its first sentence, because a model that
  takes the wrong one reports a file as missing that is sitting in its
  terminal. The terminal tools keep pi's and Claude Code's names because that
  is what a model reaches for, and `edit` accepts either harness's spelling of
  a replacement for the same reason.
- **`write` and `edit` refuse outside the directory, and `read` does not.**
  Reading changes nothing and the shell can read anywhere, so refusing it buys
  a retry through `cat`. An agent editing outside the directory it was given is
  wrong about where it is standing, and the refusal says so, resolved through
  links so one inside the directory cannot point out of it. Neither is
  confinement; the shell can write anywhere its user can.
- **`~` is the user's home, not the terminal.** A model that writes
  `~/.gitconfig` means that file, and quietly reading a different one would
  answer a question it did not ask. Relative paths are the terminal's; `~` is
  what the shell would make of it.
- **An edit that does not match writes nothing, and says so first.** Every
  replacement is matched against the file as it was, each must match exactly
  once, and none may overlap; if any of that fails, nothing is written. Half an
  edit is a file nobody asked for, and an edit a model believes landed is the
  next edit's wrong `old_text`, which is why the refusal ends with *the file
  was not changed*.
- **One job per agent is the whole lock.** It used to be one per work tree,
  keyed by directory, because two agents could share a repository. An agent's
  terminal is its own, so the agent is the thing that can only take one harness
  at a time, and two changes at once are two agents.
- **Dropping `agents.repository_id` needed a table rebuild.** The column
  carries a REFERENCES clause and SQLite refuses to drop a column that is part
  of a foreign key. Migration 54 rebuilds `agents` the way migration 4 did,
  with enforcement off around the sequence so the old table's drop does not
  cascade into everything that points at an agent, and recreates both of its
  indexes. The test that goes through it checks a working note survives and
  that two live agents still cannot share a name.
- **Every part of the bridge fails open, and that is the whole error
  handling.** A bridge that could not bind, a `curl` that is not installed, a
  Claude Code too old for the contract and a server that already dropped the job
  all end the same way: an empty answer, exit zero, and a job that runs exactly
  as it did before any of this existed. The direction cannot be reversed.
  Everything the bridge adds is an improvement on a job that already worked, so
  a bridge that refused to start a job would trade a working harness for a
  feature built on top of it. The one place that fails *closed* is the gate's
  verdict: a dropped sender answers deny, because a permission that granted
  whenever the plumbing broke is worse than none.
- **The `Stop` hook blocks and delivers in the same call, and that is what
  makes it terminate.** A `Stop` hook's `reason` reaches the model as feedback
  on the refusal to stop, so the pending mail goes out in the answer that
  refuses. Blocking without delivering would find the same mail pending on the
  next `Stop`, refuse again, and go round until the forty-five minute ceiling
  killed a job that had finished its work. The same reason `take_mail` reads and
  clears together: mail delivered twice is an instruction the model was given
  twice.
- **A `PreToolUse` hook's `deny` overrides `--permission-mode
  bypassPermissions`, and every job here runs in that mode.** Measured against
  2.1.247. Without it the gate would be a suggestion, and there would be no way
  to have both a job that never stops for the ordinary tool call and a job that
  stops before it pushes. No offline test can see this, or the two beside it
  (`Stop`'s `reason`, `PostToolUse`'s `additionalContext`): all three are
  promises about how the program *behaves* rather than flags it accepts, which
  is what the `#[ignore]`d half of `tests/coding.rs` is for.
- **A job's session id is chosen rather than read back.** `--session-id` takes a
  UUID, so one value is the job's address on the bridge, the key of its mailbox,
  and what an operator hands to `claude --resume`. That last one is the reason:
  `claude -c` resumes whatever ran last in the directory, which after two jobs is
  the wrong one. Chosen also means a job killed at the ceiling, and one that died
  before its first event, both still have one to hand over.
- **The gate reads what a line runs, and stops short of what it cannot read.**
  Those are one decision, not a rule and a hole in it. Reading the words alone
  is what one level of indirection walks straight past: `./scripts/ship.sh` is
  not `git push`, so a repository whose release is a script had a gate that was
  switched on, said it was holding, and stopped nothing. So a package script and
  a file in the work tree are read and asked the same question, three deep. A
  Makefile target, a compiled program and anything that is not text are not, and
  that is the decision rather than the gap: treating *there is something here I
  cannot see through* as a reason to ask parks a turn for `./target/release/app`
  and `./node_modules/.bin/vite`, which is the wrong yes that teaches an
  operator to switch the gate off, after which it holds nothing at all.
  `docs/CODING.md`.
- **A no is remembered for the run, and only a no is.** A model that has just
  been refused a push tries the push, which is ordinary rather than confused:
  what it read says the operator did not allow it, not that they never will. The
  operator pays, in a second card and a third for a question they are sitting
  there answering. `Runs::refused` is keyed by the outward action the card
  named rather than by the line, because a key that told `git push origin main`
  from `git push --force` would remember nothing a retry could not walk around.
  An expiry is not remembered: that is the operator being somewhere else rather
  than answering, and held against them a request nobody saw would refuse the
  one they would have seen two minutes later. Per run, so the operator's next
  message clears it; in memory, for the reason a job's lock is, since a refusal
  that outlived the process is an agent quietly refusing pushes with no
  decision behind it.
- **`shell` takes no lock, and `code` takes one.** They look like the same
  decision about one work tree and are opposite ones. Two harnesses in a
  directory interleave their edits over minutes and nothing downstream could say
  which of them wrote what; one line is the operator typing in their own
  terminal while a job runs, which nothing prevents and which is ordinary.
  Refusing it would take away the read an agent most wants while a job is going,
  which is what the job is doing.
- **A coding job is not a turn, so it must not move the agent's activity.**
  `Runtime::park_with` is `park` with exactly that one difference. A parked turn
  is an agent genuinely stopped mid-inference and the dot beside its name has to
  say so; a job outlived the turn that started it by many minutes and its agent
  may be idle or answering somebody else. Everything else about a request, the
  row, the waker, the ten-minute window and the expiry, is shared rather than
  copied, because a second copy is a second place for a request to be left
  waiting on nobody.
- **The gate is off unless the operator turned it on, and not for the reason it
  looks like.** Not compatibility. `APPENDED_PROMPT` tells every job that nobody
  will answer a question, and switching the gate on everywhere would make that
  sentence false for every agent at once: a job that believes it while a
  hook silently holds it is a job that reports a push it never made.
- **A coding job inherits the host's whole Claude Code setup on purpose,
  and one thing in it can hold a job open.** No `--strict-mcp-config` and no
  `--setting-sources`, which is the exact opposite of `llm/claude.rs` and right
  for the opposite reason: a job works in a repository, where the rules file
  and the servers configured there are what make it good. Measured at 16 MCP
  servers, 229 tools, 100 slash commands and 8 agents on one machine. The hazard
  is theirs too: a `Stop` hook of their own answering `{"decision":"block"}`
  holds a job against its own completion until the ceiling, in a loop nothing
  here can see. `docs/CODING.md`.
- **A harness is two functions, and the process around them is one.** What `pi`
  and Claude Code share is the shape of a job: one process, in one directory,
  whose stdout is JSON objects one per line, that ends. So the spawn, the read
  loop, the forty-five minute ceiling, the kill and the exit handling are in
  `coding/mod.rs` once, and each submodule holds only the argument vector and
  the fold from an event into an `Outcome`. Two of everything would be two
  places for `kill_on_drop` to be forgotten.
- **`claude` refuses `--output-format stream-json` without `--verbose`, and the
  refusal is on the command line.** So a vector that is one flag wrong is a job
  that never starts rather than a job that fails, which is why `tests/coding.rs`
  asserts the vector against a stand-in on `PATH` and keeps an `#[ignore]`d half
  that asks the real program. No offline test can see a flag the vendor renamed.
- **The two tool tables are separate and must stay separate.** `pi`'s built-ins
  are lowercase and carry `path`; Claude Code's are capitalized and carry
  `file_path`. Merged, one program's field name is read out of the other's
  arguments, and a wrong guess there prints somebody's file contents into a
  channel. A tool in neither table draws no detail at all, which is every MCP
  tool the operator has connected.
- **A tool call's line is cut after the `cd` comes off it, not before.** Every
  harness is started in its directory with `current_dir` and the brief names
  that directory by its absolute path, so a model reads the path as somewhere
  to go and writes `cd "<110 characters>" && pnpm test` in front of every
  command it runs. Cut from the head at 120, that drew a panel of nine
  identical lines, each one most of a path and none of what ran. So `detail_of`
  hands up the whole first line and `run` cuts it: `run` is the level that
  knows where the job is standing, and a `cd` is only redundant against that.
  `cd frontend && pnpm test` and `cd elsewhere || echo no` both stay whole, one
  because it says where the tests ran and the other because it runs *because*
  the `cd` failed. This is the half that does not depend on the brief being read.
- **A cost from a harness is what it *said*, not money that moved.** On a
  subscription each reports the equivalent API price. They agree with each other,
  and `Outcome::cost` claims no more than that; zero is absent rather than free,
  for the reason it always was.
- **A double-clicked app does not have the operator's `PATH`.** `launchd` starts
  one from the Dock or the Finder with `/usr/bin:/bin:/usr/sbin:/sbin` and
  nothing else, so `claude` under `~/.local/bin` and `pi` and `gh` under
  `/opt/homebrew/bin` are all missing from the only list this app looks a
  program up in. Started from a terminal it inherits that terminal's `PATH` and
  finds every one of them, which is why the whole suite, `pnpm app` and
  `cargo run` pass and only the built app fails, and why the first report of it
  was an operator being told `claude is not installed` with `claude` on their
  path in the window they had built the app in. `programs.rs` asks their shell
  once at startup, and the shell has to be a login shell *and* an interactive
  one: a zsh user's `PATH` is written in `.zshrc`, which `zsh -l -c` never
  reads, so a login-only probe is a fix that changes nothing and looks like it
  worked.
