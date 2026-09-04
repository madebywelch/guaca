# The menu bar

Guaca with the window shut: what the strip says, what the menu offers, and why
closing the window does not end the app. *The menu bar is Guaca with the window
shut* in `docs/WORKSPACE.md`, then `menubar.rs`, `tray.rs` and `app.rs`.

- **The menu bar's presence is read, not accumulated.** Every number on it but
  the session total is a fresh read of the roster, the activity map, the pending
  requests and the usage table. One assembled by adding up events drifts the
  moment one is missed, and what drifts is the number the operator is using to
  decide whether to go and look.
- **`menubar::plan` exists so an open menu is not replaced under the operator.**
  Same row shapes in the same order is the same menu saying different numbers,
  which is a text edit; anything else is a rebuild. The spend on that menu moves
  every few seconds while a crew works, so a strip that rebuilt on every change
  would close itself exactly when it was worth reading.
- **A crew is named only when there is another crew to tell it from.** The rule
  is the crews' column's, and it is what keeps the change invisible to the
  workspace that has never made a second crew: `Presence::crew_named` returns
  nothing while `crews` holds fewer than two, which collapses the working list's
  sort back to the two keys it always had and takes every heading out of it. A
  strip that always named the crew would put the same word on every row of a
  menu shared with every other app on the machine.
- **A crew's heading is emitted with the first row under it, never before the
  run.** The working list is capped, and the cap is counted in agents; a heading
  written ahead of a crew whose rows all fall past it is the menu naming a crew
  that is working and then listing nobody from it.
- **The count on a crew's heading is out of `Row::shape` on purpose.** It moves
  whenever an agent starts or stops, and past the cap it moves with no row
  arriving or leaving, which is exactly the case `plan` exists to edit in place
  rather than rebuild.
- **The attention glyph is the one tray image that is not a template.** macOS
  tints a template image to match the menu bar, so a template glyph cannot have
  a color. Giving up the tint buys the one state that must not be missed, and
  the count beside the icon says the same thing in text.
- **An ampersand in a menu item has to be doubled.** Every platform's menu reads
  `&` as a mnemonic marker and eats it, so an agent called `R&D` draws as `RD`.
  `menubar::escape_mnemonic` is applied on the way into an item and nowhere
  earlier, so the rows a test reads are the words a person would.
- **The strip points at two things and the window answers them differently.**
  `Reveal` is a tagged union rather than an agent id because an agent is
  `select`, which follows it into whatever crew it is in, and a crew is
  `focusGroup`, which opens the crew and picks nobody: a crew answered by
  choosing one of its agents puts somebody's history on screen as a side effect
  of a click that was about the crew. The two lists are compared by
  `ipc.contract.test.ts`, because a variant added on one side is a click that
  arrives and does nothing.
- **Closing the window hides it, and only while the tray exists.** Tauri exits
  when the last window closes, which for this app means a routine set for every
  morning stops firing the first time somebody tidies their screen. A hidden
  window is not a closed one, so preventing the close is the whole mechanism.
  The condition is not caution: an app with no window and no menu bar icon is
  one the operator cannot see, cannot reach and cannot stop.
- **The machine mark is a guard, not a pair of writes.** `Runtime::on_machine`
  inserts and its `Drop` removes, so a run stopped in the middle of a
  `use_screen` call clears the mark with the future it drops. Written as an
  insert before the call and a remove after it, a stop would leave the agent
  reported on its computer for the rest of the session.
