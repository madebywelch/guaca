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
- **The flow board is not a channel, and `ACTIVITY_CHANNEL` is gone.** Who
  spoke to whom is analysis: somebody arrives at it having decided to look into
  something, and it sat at the top of the rail under the wordmark, which is a
  claim about how often anybody wants it that every other row in the rail paid
  for. It is a pane in the group editor now. Deleting the key is most of the
  value: a board addressed as a channel meant seven functions took a channel
  that was not an agent and carried a branch for it, including which crew the
  rail follows, what `loadChannel` reads, and what `messageAppended` maintains
  against a board nobody had necessarily opened. It is one crew's traffic,
  scoped in SQL, because the board is the newest four hundred messages and a
  busy crew filling that window would hand a quiet one an empty board.
  `docs/WORKSPACE.md`.
- **A crew's spend is a card over the rail, and nothing may put it back on the
  heading.** A readout is a fixed width and a crew's name is not, so the name is
  the only item on that line with anything to give and flex takes it:
  "StopTheScam" was drawn as "StopTh…" as the heading of its own column. Two
  things about the card are load-bearing and neither is visible in a DOM
  assertion, because jsdom lays nothing out and the node is there whatever the
  rule says. It is `position: fixed`, so hovering a heading cannot reflow the
  column the change existed to widen. And it is `pointer-events: none`, or it
  takes the pointer off the heading that is the only reason it is open, closes
  itself, hands the pointer back and opens again, under a hand that has not
  moved. `styles.test.ts` reads the cascade and is the gate.
- **It opens on a dwell, and the timer is cleared on the way out.** A heading is
  a band across the top of its own rows, so a pointer travelling from the search
  box to an agent crosses one every time and opening on contact flashed a panel
  over the row being aimed at. The heading is measured when the pointer arrives
  rather than when the timer fires: `currentTarget` is null by the time React is
  finished with the event.
- **The crews' column slides out; it does not stand open.** Two thresholds, not
  one, or a hand resting at the boundary flickers it. Both distances are read
  off a box CSS sizes rather than written in the component, so they are lengths
  in the one stylesheet at the operator's own scale. The zone starts below the
  top of the window because macOS floats the close button over that corner, and
  it is decided from the pointer rather than from `:hover` on a strip: a strip
  wide enough to aim at is a strip laid over the left edge of every agent row
  behind it. Proximity and focus inside it bring it out. A drag does neither: it
  suspends the closing threshold, so a column already reached for cannot slide
  away mid-gesture and take the drop target with it. Held out for the whole of
  every drag instead, it came over the rail the moment a row was picked up
  anywhere in the window, covering the left edge of every row a reorder was
  aimed at, which is what most drags are. `src/lib/reach.ts`.
- **A crew's circle is not a toggle, and the mark on the current one is ink.**
  Clicking the crew the rail is already inside used to take it back out to the
  overview, so the gesture for opening a crew went in and straight back out when
  it was made twice, and there was nothing on screen saying which of the two
  states it had ended in: the mark for the current crew was a bar spelled
  `--flesh-soft`, which is the accent's ground rather than an ink, at 1.06 to 1
  on paper and 1.31 to 1 in a dark room. Two fixes for one complaint. The way
  out of a crew is the circle at the top of the column, which is on screen for
  the whole of every gesture, so a circle does not have to be its own way out;
  and the current crew is a band plus an ink bar, which is what `.agent-row`
  draws for the open channel one column over. `styles.test.ts` holds the bar to
  three to one against `--grail-ground` on both surfaces, because the element
  was in the document with the right class on it the whole time and no DOM
  assertion sees a color. `docs/WORKSPACE.md`.
- **`select` follows an agent into its crew; `focusGroup` lets a channel go.**
  One invariant from two ends — the rail draws the row of whatever the pane is
  showing — and the asymmetry is deliberate. `select` is the operator naming an
  agent, so going to that agent's crew is what they asked for. `focusGroup` is
  them naming a crew, and following the channel back out of it would undo the
  click. Before the crews had a column of their own, `select` dropped out to the
  overview instead, because that was the only view where every row was drawable.
  What `focusGroup` falls back to is nothing, rather than the first row of the
  crew being entered: opening a channel is the operator naming somebody, and a
  crew that picked one for them would put an agent's history on screen as a side
  effect of a click that was about the crew.
