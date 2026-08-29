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
- **The attention glyph is the one tray image that is not a template.** macOS
  tints a template image to match the menu bar, so a template glyph cannot have
  a color. Giving up the tint buys the one state that must not be missed, and
  the count beside the icon says the same thing in text.
- **An ampersand in a menu item has to be doubled.** Every platform's menu reads
  `&` as a mnemonic marker and eats it, so an agent called `R&D` draws as `RD`.
  `menubar::escape_mnemonic` is applied on the way into an item and nowhere
  earlier, so the rows a test reads are the words a person would.
- **Closing the window hides it, and only while the tray exists.** Tauri exits
  when the last window closes, which for this app means a routine set for every
  morning stops firing the first time somebody tidies their screen. A hidden
  window is not a closed one, so preventing the close is the whole mechanism.
  The condition is not caution: an app with no window and no menu bar icon is
  one the operator cannot see, cannot reach and cannot stop.
