# The workspace

The rail, the crews' column, and what deleting an agent does for thirty days.
`docs/WORKSPACE.md`, then `src/lib/rail.ts`, `src/lib/orb.ts` and the two halves
of `Runtime::discard_agent` and `Runtime::purge_agent`.

- **A deleted agent is `Terminated` with a stamp, not a fourth lifecycle.** The
  compost holds it for thirty days and everything it owns privately waits with
  it, but to the rest of the app it is deleted: unreachable, undiscoverable, out
  of the rail, out of every crew and out of the partial index that frees its
  name. That is the point of the column. Fifteen queries ask `lifecycle <>
  'terminated'` and every one of them is still right; a fourth state would be
  fifteen places to remember, each failing quietly and differently — a composted
  agent in a directory listing, in a crew count, in a disband, in the roster a
  peer is told to ask. `NULL` is both ends of the wait, and the lifecycle tells
  them apart. `docs/WORKSPACE.md`.
- **A composted agent still holds its sandbox, and `claimed_sandboxes` has to
  agree.** Deleting sleeps the machine rather than killing it, because the disk
  is where the operator's own sign-ins live and only they can put them back.
  Under the old rule — only a live agent holds a claim — the next sweep would
  kill it inside the minute, and a restore three weeks later would hand back an
  agent signed in to nothing.
- **A restore comes back paused, and settles its name on the way.** Paused
  because thirty days of a schedule have come due without it; renamed because
  the name was freed the moment it was thrown out and the crew may have hired
  into it, so `copy_name` steps around the clash rather than letting the unique
  index refuse a button whose job is to succeed.
- **`select` follows an agent into its crew; `focusGroup` lets a channel go.**
  One invariant from two ends — the rail draws the row of whatever the pane is
  showing — and the asymmetry is deliberate. `select` is the operator naming an
  agent, so going to that agent's crew is what they asked for. `focusGroup` is
  them naming a crew, and following the channel back out of it would undo the
  click. Before the crews had a column of their own, `select` dropped out to the
  overview instead, because that was the only view where every row was drawable.
