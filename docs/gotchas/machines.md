# Computers and browsers

Two places, not two views of one: an E2B machine with a screen, and a hosted
Chrome that is asked rather than looked at. `docs/MACHINES.md` and
`docs/BROWSERS.md`, then `e2b.rs`, `kernel.rs`, `cdp.rs` and
`Runtime::surfaces_for`.

- **The sign-in tests carry real cookie names.** A cookie's presence is not a
  login. Do not loosen them without a fresh capture from a live machine.
- **All four conditions in `needs_consent` are load-bearing.** Each one alone
  refuses honest work. Read the doc comment before narrowing or widening any.
  The first is the operator's and is off by default: `Consent` on the card, set
  per agent under the browser pane. It is not caution about a migration and not
  a switch waiting to be flipped on for everybody. A gate that fires on every
  search press is answered without being read, and an agent doing research
  leaves the granted site and returns on every cycle, so it fired on every
  cycle. Per site is not the missing granularity either: which account on a
  site an agent may act as is an instruction only the model can hold.
  The fourth clause is not one of them: a yes is remembered against that site
  for the rest of the turn, because asking per press produced four dialogs in a
  row for one account and a question in that shape is one an operator clicks
  through. It is not a standing yes. It lives on the turn's `Reading`, reaches
  no table, and `Reading::took_in` drops it the moment the turn takes in a page
  from anywhere else.
- **A key in settings says what the workspace can hand out; the card says who
  was given it.** Both have to be true, and `Surfaces::given_to` is the only
  place they meet. Deciding from the key alone is what this replaced: every
  agent was offered `run_command` and `browse`, and the first one to think of it
  rented a machine mid-turn. Deciding from what an agent is holding instead
  would be worse than either, because a machine is reclaimed on the provider's
  clock: the tools would vanish from a working agent the moment its sandbox
  slept.
- **The gate is in `ensure_computer` and `ensure_browser`, not at the tool call
  sites.** Those two functions are the only places a machine or a browser is
  made, and tools are not the only route to them: a file arriving for an agent
  is placed on its machine, and a text file too long to inline is placed there
  too. A gate at the call sites would rent a machine for an agent the operator
  deliberately did not give one. `Runtime::not_given` sits in front of the
  dispatcher as well, and that is not a duplicate: it is what turns a model
  calling a tool it was never offered into a refusal it can act on rather than
  an error that reads like a broken machine.
- **What an agent is *given* comes from the turn's card; what it *holds* comes
  from the row.** `run_turn` reads one `AgentCard` and passes it through every
  round, so a machine or a browser provisioned by the first tool call is on the
  row and not on that snapshot. Read from the snapshot, the second call of a
  turn provisions again: a duplicate sandbox that bills until the sweep finds
  it, and a second browser Kernel refuses by name, which is a `browse` tool that
  fails for the rest of the turn after the first page loads. `Runtime::held` is
  the read, and the card stays the authority on `has_computer` and `has_browser`.
- **A 409 from Kernel's create is an orphan to adopt, not a failure to
  report.** The name is one per agent, so a conflict is this agent's own
  browser, alive and unrecorded: a crash between creating one and writing it
  down. It is found by its `guac-agent` tag rather than by the name the conflict
  was about, and asked for by id, because a list row is not documented to carry
  a socket and a session without one names a browser nothing can talk to.
- **Taking a computer back leaves `sandbox_id` where it is.** The machine sleeps
  and its disk stays, because that disk is where the operator's sign-ins live.
  A revoke that destroyed it would make giving the computer back mean signing
  everything in again, which is the one thing an agent cannot do for itself.
  Taking a browser back closes it instead, for the same reason from the other
  end: closing is what writes the cookies to the profile.
- **Every `use_screen` action answers with a picture, and only the newest one
  stays.** The first is what stops a model acting on a screen two actions old;
  the second is what keeps that affordable. Removing either breaks the other.
- **A machine's Chrome opens no debugging port.** Two ways to use the web on one
  screen disagreed about which window was in front, and each fix moved the
  disagreement. `docs/BROWSERS.md` has the history before you add one back.
- **A sign-in is stored against the surface it was found on.** Both are scanned
  independently, so a replace that took the agent's whole set would erase the
  other's findings on every scan.
