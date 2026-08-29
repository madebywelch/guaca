# A turn on screen while it runs

The live bubble, the trail, the thinking, and whether the view may move under
the operator. `docs/WORKSPACE.md`, then `src/lib/reasoning.ts`,
`src/lib/trail.ts` and `src/lib/follow.ts`. A chip's own layout rule is in
`styles.md`.

- **The live bubble joins a turn's rounds with `ROUND_BREAK`, exactly as the
  accumulator does.** A model narrating its work says a sentence before each
  tool call, and all of a turn's rounds stream into one placeholder. Without the
  break the operator watches the next round start mid-sentence
  (`…who is here.Two of us.`) and then watches it correct itself when the real
  message lands. The pen writes the break in front of the round's first token,
  so a round that turns out to be tool calls and nothing said leaves none
  behind, and it decides from what has been *drawn* rather than from what has
  been collected: a retry throws the bubble away and keeps the accumulator.
- **The live trail is a count, and the chips behind it share the working's
  slot.** Both look like the drawing being timid about what it has. Drawn open,
  a long turn's whole record sits between the transcript and the composer,
  seven kinds of work across four rows, reflowing every time a call comes back
  and moving the box somebody is typing in; and stacked with the thinking, the
  transcript gives up twice the height for a question asked once. Nothing is
  lost: the transcript draws every chip from the same rules the moment the turn
  ends. The two things that stay on the line are the two a count cannot carry —
  a failure, which is the one part somebody may have to act on, and a
  credential by name, which is their audit trail for their own tokens.
- **Only the component drawing the live bubbles subscribes to `streams`.** One
  level higher, a single token re-renders every message in the transcript. The
  same split is why the line above the composer, the turn's chips and the open
  thinking are three components: they sit next to each other and change at
  wildly different rates, and written as one every token re-rendered every chip.
- **A turn's thinking is held whole and drawn one line at a time.** Those are
  two decisions, and holding 240 characters made them one: the tail was all
  there was, which is fine for a wait of thirty seconds and no use for one of
  ten minutes. Nothing about holding it widens what "never kept" means. It is
  the same slice, dropped by the same event, and it reaches no channel, no
  prompt and no hash.
- **The line drawn is the last sentence that *finished*, under the model's own
  heading.** Not the tail. A tail replaced every sixteen milliseconds is a
  flicker that says a turn is alive, which is what the creature beside it
  already said, and nobody can read a sentence as it is typed. Waiting for the
  period costs a second of staleness and is the difference between a line and a
  blur.
- **The live trail and the thinking have one lifetime, because they have one
  mechanism.** `ToolStarted` and `ToolFinished` are addressed to the placeholder
  exactly as `ReasoningDelta` is, so a retry that reopens under a new id starts
  both again. What was done is not lost by that: it is in the message that lands
  at the end of the turn, which is the record. These are only what that record
  looks like before it exists.
- **A transcript decides where the operator is by comparing the offset, not by
  listening for a scroll event.** The event is delivered after the fact and a
  token committing in between arrives first, so anything that waits to be told
  has already put the view back on the floor: under streaming text a trackpad
  could not climb out of a channel at all. `lib/follow.ts` remembers the offset
  it wrote and checks the box is still there before writing again, which is why
  one pixel is enough and no threshold is. Its listener is bound by a ref
  callback for the same reason: the node is replaced whenever the pane shows a
  pair thread or nothing at all, and an effect cannot re-bind on that. The
  size observer beside it is bound there for that reason too, and it is not
  decoration: everything under a transcript takes height from it without
  anything arriving or scrolling, so a composer growing a line or the working
  panel opening put the newest message under the fold and left it there.
