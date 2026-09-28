# Artifacts

Pages a crew keeps. *An artifact is a page a crew keeps* in
`docs/WORKSPACE.md`, then `src-tauri/src/domain/artifact.rs`,
`src-tauri/src/db/artifacts.rs`, `Runtime::keep_artifact` and
`src/components/Artifacts.tsx`.

- **The crew comes from the agent's card, never from the call.** Every store
  call `keep_artifact` makes is scoped to `card.group_id`, and an id from
  another crew comes back as `None`. The refusal says *no artifact with the
  id*, never *not yours*, for the calendar's reason: the second sentence
  confirms the page exists and hints whose it is.
- **An edit does not change the owner.** Anyone in the crew may edit; ownership
  moves only by `take` or by the operator handing it over. Folding the two
  together makes whoever touched a page last answerable for it, which is the
  opposite of what the owner column is for.
- **An owner who left is still the owner.** Deleted or moved to another crew,
  it is named and marked `gone` until somebody decides to take the page.
  Clearing it automatically is a decision nobody made.
- **Never message the owner about an edit.** The owner reads it from its own
  prompt. A message is a paid turn, and an owner woken by every edit edits back
  and wakes the editor. The tool description and the answer to `update` both
  say so, and a cascade test holds the second.
- **The log stores names, not only ids.** `actor` and `owner` are the names at
  the moment, and `agent_id` has no foreign key. A log that joined names in at
  read time would say a purged agent's edits were made by nobody.
- **`Change::makes_version` and the table's CHECK constraint say the same
  thing.** A row that made a version carries a page and no other row does. The
  store test that inserts every change both ways is what keeps them together.
- **`version` and `updated_at` on the row are a copy.** The newest history row
  that made a version is the truth; the row holds a copy so the list, read on
  every turn of every agent in the crew, is one indexed query. The two are
  written in one transaction in `db/artifacts.rs` and nowhere else.
- **A restore is a new version.** Rewinding `version` would leave two rows
  claiming the same number and lose the page that was replaced.
- **The frame server's cap is `MAX_PAGE`, by reference.** Two numbers that
  agree today are a kept version the frame server refuses tomorrow.
- **A crew export carries artifacts, and a log row can name an agent that is
  not in the file.** An editor who moved to another crew before the export is
  given a fresh id that points at nothing, and keeps its name; an owner who
  moved is imported as nobody. The import's "refers to data outside its group"
  check skips `artifact_history.agent_id` for that reason and no other.
- **The history, the owner and the version are not the point, and are drawn
  that way.** One faint line of details, and the history and owner control
  behind a button. A history column beside the page and a table of owner,
  version and date in the list were both tried and both took the room from the
  page. Keep new metadata in the details line or behind that button.
- **A page never chooses the arguments of a read.** They are fixed when the
  page is written and allowed by the operator as written. A page that could
  choose them could send what it read somewhere, as a search query. A "filter"
  argument the page fills in is that hole with a friendlier name; filter in the
  page's own script instead.
- **An approval is of an exact list, compared as stored.** `allowed_sources`
  holds the string the operator allowed, and a version's reads run only when it
  declares that same string. Re-serializing either side before comparing, or
  approving "the artifact" rather than the list, is how a changed list slips
  through without being seen.
- **Reads run as the owner, and are checked against the owner when declared.**
  An editor who is not the owner cannot give a page reads the owner could not
  make. A page nobody in the crew owns reads nothing, and says why.
- **A read never calls a model.** `Runtime::read_artifact` goes through
  `call_connector` and nothing else. A read that woke an agent would make
  opening a board a paid turn.
- **A kept page's send needs the operator's click, one at a time, and never
  fires by itself.** Focus in the frame is the evidence of a click; an owner
  still working refuses the next one. A page that sends on a timer or when its
  data changes is every open spending a turn, and a loop with its owner. Work
  on a clock is a routine.
- **A send is the operator's message with the page's value fenced inside it.**
  `sentMessage` writes the sentence and names the page; nothing the page wrote
  arrives as an instruction in the operator's voice, for `answerMessage`'s
  reason.
- **In a fenced page, `guaca.send` is an answer.** It has nobody to reach but
  the operator, so it waits in the strip. Only a kept page, whose host passes
  `onSend`, carries it to an agent.
- **The dialog is the reading view, so its page has no Open.** `HtmlArtifact`
  is drawn with `fill` there: no cap on the frame, and nothing to press. Its
  Open belongs to a message, where a page is bounded. Offered inside the
  dialog it opened a second modal showing the same page, and trapped it: the
  transform `pop` leaves on every dialog makes that dialog the box a
  `position: fixed` scrim is drawn in, so the full view came out a strip of
  the dialog's height, scrolling sideways.
- **Delete is the operator's only.** Agents have `update`, which keeps what it
  replaced. What the operator keeps is theirs to throw away.

## On the status bar

*The status bar holds pages the crews keep, and a click on one is the bar's* in
`docs/WORKSPACE.md`, then `domain/widget.rs`, `src/lib/widget.ts` and
`PinnedPage` in `src/components/StatusBar.tsx`.

- **The condensed view is a second document, not the page made responsive.**
  A page that folds itself down to one row still loads everything it is, all
  day, and can still put a small copy of everything in the strip. The 16 KB
  cap on a separate document is what keeps the strip a strip.
- **It is part of the version.** Written, kept by an edit that leaves it out,
  and put back by a restore, with the page. A column on the artifact row
  instead would pin today's number beside last month's board after a restore.
- **A version can have none, and a pinned page can be on one.** A restore can
  bring back a version from before the strip existed. The bar draws the title
  in its place, and the dialog offers only to take it off.
- **A pin is a setting, not a column.** What is on the bar is the operator's
  decision about their screen, and every window has to draw the same bar. Both
  acts that delete pages call `Runtime::unpin_missing`; a third one has to too.
- **The frame takes no pointer events, and its messages are never heard.** A
  click on a widget is the bar's button, and it opens the page. Giving the
  strip `guaca.send` puts a row of tiny controls that each spend the owner's
  turn one mis-click away, and a click inside a frame on another origin cannot
  be seen from outside anyway.
- **An agent's pin allows the reads the card showed, and nothing else.**
  `allow_artifact_sources_seen` compares the list the card was built from with
  the one the version declares when the operator answers. A crewmate who
  changed the reads while the card waited gets a pin whose reads still wait.
- **The operator's pin allows nothing.** Reads are allowed above the page,
  where the list is drawn in full. A pin that allowed them too would be an
  approval of a list nobody was shown.
- **The bar's look is solved from the preferences, not read off the document.**
  A child's effect runs before `App`'s, so reading computed colors when the
  theme changes reads the old ones and frames the strip in them.
- **The widths are stated to agents in pixels.** `--widget-narrow` and
  `--widget-wide` in `styles.css` and the sentence in the `artifact` tool are
  one fact in two places, and `widget.test.ts` fails when they disagree.
- **Reads do not run while the window is hidden, or on a clock when nobody
  allowed them.** A read nobody sees is a connector call spent on nothing, and
  reads that are refused are refused the same way every time.
