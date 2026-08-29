# Figures, callouts and pages

What a reply can be drawn as when it is not prose. *A reply can be a figure* and
*A reply can mark the one part that needs a person* in `docs/WORKSPACE.md`, then
`src/lib/figure.ts`, `src/lib/chart.ts`, `src/lib/palette.ts`,
`src/lib/callout.ts` and `src-tauri/src/artifact.rs`.

- **A callout is a quote and a figure is a fence, and neither could be the
  other.** A chart spec is text, so it fits in the one block markdown has for
  text. What goes in a callout is prose: a list, a link, a mention, a table, a
  line of code, none of which survive a fence. The syntax is GitHub's alert
  marker rather than one of ours because models write `> [!IMPORTANT]`
  unprompted, so it draws on a reply written before the prompt mentioned it and
  on every transcript already stored; a marker the closed set does not know
  stays a quote with its own words in it. Five markers draw two boxes and carry
  two labels, and the label is the app's word: an agent writing `[!CAUTION]`
  and one writing `[!WARNING]` mean the same thing, and what the operator needs
  off the box is whether it is for them. The amber one is the only filled panel
  in the reading column, and it means there what it means in the rail.
- **A figure is a fence in the reply, not a tool call and not a new part.** An
  agent has the numbers in hand at the moment it writes the sentence about them,
  so a chart behind a tool call is a round trip spent sending back something it
  had already finished computing. A fence also needs no runtime change at all:
  `as_plain_text` returns the text of a message, so the record, the prompt, the
  dedup fingerprint, search and a peer's copy keep working untouched, and the
  agent can read back what it drew. `Part::Json` is still unused and is still
  not this.
- **A chart spec that has not finished arriving is neither drawn nor refused.**
  A reply lands a token at a time, so a chart spends most of its life on screen
  as half a JSON object, and calling that an error puts a red box under every
  figure for a second, which teaches an operator the feature is broken.
  `looksComplete` counts braces outside strings, since a category legitimately
  named `}` must not end the document early, and until they balance the figure
  says it is still drawing. Once they balance, a spec that still will not parse is
  wrong rather than late, and says so.
- **A refused chart is drawn as its own source with the reason under it.** Both
  halves are load-bearing. The operator needs to see what their agent thought it
  was showing them, and the agent needs a sentence it can act on next turn,
  which is why every refusal in `readChart` names the field and the fix. "Invalid
  chart" costs a whole turn and teaches nothing.
- **The eight series colors are the output of a check, and the *order* is the
  check.** Neighboring slots are what touch in a stack and cross in a line
  chart, so neighbors are the pairs that decide whether a chart is readable to a
  colorblind operator, and nobody can verify that by looking. The order came out
  of enumerating all 40,320 and keeping the 160 that pass on this app's own two
  surfaces. `palette.test.ts` recomputes every figure in `palette.ts`'s comment
  from the hexes themselves, so a hex nudged because a screenshot looked slightly
  off fails the suite. A ninth hue is refused rather than generated: a generated
  one is indistinguishable from one of the eight under colorblindness.
- **Nothing inside a chart's drawing is focusable, and the Figures table is
  why.** The `svg` is one `role="img"` with a sentence on it, which makes its
  subtree invisible to a screen reader by definition, so a label on a band
  would be announced to nobody, and tab stops would put twelve invisible
  rectangles between one message and the next for a readout the table already
  holds in a form somebody can read. The table is also the relief that lets
  three light-mode hues sit under 3:1 against the surface, and
  `palette.test.ts` asserts that debt so it cannot be dropped quietly.
- **A page an agent wrote is framed from `artifact.rs`, never from `srcdoc`.**
  A frame pointed at `srcdoc:`, `blob:` or `about:blank` inherits the framing
  document's content policy, and this app's forbids script. The page would draw
  and its script would silently never run: an empty rectangle that passes every
  test, which is the same failure `FileCard` has a note about. So it gets an
  origin of its own, and the round trip through `frame_artifact` is what buys it.
- **A page hands a value back and never a message, and the two clicks are the
  lock.** `guaca.answer` posts to the window that framed the page, which is the
  one channel an opaque origin has and the one the height reporter already used;
  the renderer draws the value in Guaca's own chrome and waits for the operator.
  Letting the page send directly is a page that sends again every time it is
  scrolled past, because a transcript re-frames one whenever it draws it, and
  every send is a turn nobody asked for and somebody paid for. The value is also
  JSON and never a sentence, so nothing the page wrote can arrive as an
  instruction in the operator's voice: `answerMessage` is the wording around it.
  Same line `domain::approval` draws between a permission and a question.
- **A page is framed once, whole; a chart is redrawn every token.** They look
  like the same decision and are opposite ones. A chart is a pure function to
  coordinates, so redrawing it is free and is what makes one assemble itself on
  screen. A page is registered and then pointed at, so redrawing it is a reload:
  a round trip per token, an entry per token in a store that holds two dozen, and
  a frame that throws away whatever the operator had done in it. `live` on
  `Markdown` is the whole mechanism, and `StreamingMessage` is its one caller.
- **`allow-scripts` and `allow-same-origin` must never appear together.** On the
  frame or in `ARTIFACT_CSP`. Together they let the page remove its own sandbox
  attribute and reload out of the box, which is the whole lock. `default-src
  'none'` is the other half and is not decoration: `<img src="https://…/?data=">`
  is the cheapest exfiltration there is, and a model's page is content written by
  something that may have read a hostile web page earlier in the same turn.
- **The height reporter is prepended to a model's page, not appended.** A
  model's page is exactly where an unclosed tag lives, and an unclosed tag
  swallows everything after it. Ahead of the doctype it is still parsed and run.
  The parent trusts the message by the window that sent it and by nothing else:
  an opaque origin reports itself as `"null"`, so an origin check would either
  reject every real message or accept every forged one.
- **A peer is not told any of this.** The figure section is in the prompt for
  every reply mode but `ToPeer`. A peer is a model and wants the numbers, so a
  chart spec on that path is tokens spent drawing something nobody will look at.
