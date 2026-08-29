# Writing code

The repository, the two doors into it, the gate in front of both, and the two
harnesses that write the code. `docs/CODING.md` is the long version;
`domain/repository.rs`, `coding/`, `shell.rs`, `repo.rs` and `programs.rs` are
the code.

- **An edit to a repository is a `RepositoryEdit`, not a draft with a stand-in
  path.** A `RepositoryDraft` validates the path before anything else, so an
  edit routed through one has to invent a path it does not have. The stand-in
  was `/`, which is the empty string once `clean` takes its trailing separator
  off: every rename, every note and every harness switch came back *a repository
  needs a directory; pick one to link*, about a directory the operator had
  already picked and could read on the row above the box. Neither the panel nor
  the store was wrong and both are tested, which is how it shipped and stayed.
  A type with no path on it is the only version of "the path is not editable"
  that nothing downstream can forget.
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
- **A repository has two doors and one gate, and the gate is one function.**
  `code` hands a brief to a harness for minutes; `shell` runs one line and
  answers in the turn. The second exists because the first was the only way in,
  which made `gh pr merge` cost a coding job and made an agent whose harness
  would not start — a spent plan, a program missing, a work tree already busy —
  report that it had no shell at all, on a machine where `gh` was installed and
  signed in. It adds no reach: a job in that directory already ran arbitrary
  commands as the operator under `bypassPermissions`. What it must not add is a
  second answer to *what counts as outward-facing*, so both doors ask
  `coding::bridge::outward` and both park through `Runtime::ask_about_push`. Two
  readings of one gate is a gate an agent walks around by picking the other
  tool, which is worse than none: the operator switched it on and would be told
  it was holding. `docs/CODING.md`.
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
  that outlived the process is a repository quietly refusing pushes with no
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
  sentence false in every repository at once: a job that believes it while a
  hook silently holds it is a job that reports a push it never made.
- **A coding job inherits the operator's whole Claude Code setup on purpose,
  and one thing in it can hold a job open.** No `--strict-mcp-config` and no
  `--setting-sources`, which is the exact opposite of `llm/claude.rs` and right
  for the opposite reason: a job works in the operator's own repository, where
  their rules file and their servers are what make it good. Measured at 16 MCP
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
- **A cost from either harness is what it *said*, not money that moved.** On a
  subscription both report the equivalent API price. They agree with each other,
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
