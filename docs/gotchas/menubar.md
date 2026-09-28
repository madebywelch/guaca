# The menu bar

Guaca with the window shut: what the icon says, what the panel under it shows,
and why closing the window does not end the app. *The menu bar is Guaca with the
window shut* in `docs/WORKSPACE.md`, then `menubar.rs`, `tray.rs`, `app.rs` and
`src/components/MenubarPanel.tsx`.

- **A presence field the window does not send refuses the whole report.** The
  tray process holds no workspace, so the icon is drawn from nothing but what
  the window reports, and serde turns a missing field into a refused call, not
  a missing number. `onMachine` outlived the only code that wrote it, every
  report after that was turned away, the frontend swallowed the refusal, and
  the icon sat at "nothing running" whatever the crew was doing.
  `ipc.contract.test.ts` compares the fields of `Presence` on both sides, and
  a refusal is logged to the window's console.
- **The panel is a second client, and must not be fed.** It has its own store
  and its own socket to the host. Handing it the window's state instead would
  be a second copy to keep in step, which is the thing the host already does
  for both of them.
- **Only the window notifies, and only the window reports to the icon.** The
  panel runs the same store and sees the same events, so either of those in
  the panel is every interruption twice and two reports racing for one icon.
- **The panel is told its host by the tray, not by storage.** The window
  writes its host to `localStorage` and the panel could read it there, but the
  panel would then decide for itself when that value is current. The tray makes
  the panel only after the window reports a host, and reloads it when the
  window reports a different one, so the value and the moment to act on it
  arrive together. `adoptRemote` holds it for the load and never stores it.
- **The same host twice is not a reload.** The window reports its host every
  time it loads. Reloading the panel for each would drop whatever the operator
  was typing in it.
- **No panel until the window has a host, and the icon opens the window
  instead.** Before setup there is nothing for the panel to connect to, and the
  window is where the host is chosen.
- **A click on the icon while the panel is open is first a click outside it.**
  Where the panel hides on losing the focus before the click arrives, the click
  would read as "open" and bring it straight back. `REOPEN` in `tray.rs` is the
  window in which a click after a hide is the click that caused it.
- **The panel is an ordinary window, and that is why closing it hides the
  app.** Showing it activates Guaca. Put away from the icon or with Escape
  while the window is not open, it hides the app, which is what hands the
  keyboard back to whatever had it. Put away by clicking somewhere else, it
  does nothing more, because somewhere else already has the keyboard. Making it
  an `NSPanel` instead means changing its class under tao, which objc2
  documents as undefined, and it still would not reach another app's
  full-screen space without that.
- **The icon's rectangle is in pixels, and which display's pixels is not
  said.** Each display reports itself at its own scale, and a Retina laptop
  beside a monitor at one pixel a point can place the same pixels on both.
  `menubar::place` keeps the display that reads the icon at a menu bar's
  height. Asking tao which display holds a point does not help: it compares
  points, and the tray hands over pixels.
- **The panel's height is measured inside the scroller, not off the window.**
  The body scrolls once the content outgrows the ceiling, and a panel measured
  by its own box then never learns it could be taller again. `useFit` adds the
  chrome to the content's own height.
- **A conversation follows its end and the list does not, and they share one
  scroller.** `useFollowBottom` is attached only while a conversation is open,
  and going back to the list puts the scroller at its top. Attached for good,
  its resize watch would drag the list to its bottom every time the window
  grew.
- **The panel is a page of its own, in three places.** `menubar.html` is a
  second Vite input, the daemon's image copies it into the web stage, and the
  panel's window has its own capability in `src-tauri/capabilities/`. A page
  missing from any of them is a panel that fails to load, an image that fails
  to build, or links in a reply that refuse to open.
- **The attention glyph is the one tray image that is not a template.** macOS
  tints a template image to match the menu bar, so a template glyph cannot have
  a color. Giving up the tint buys the one state that must not be missed, and
  the count beside the icon says the same thing in text.
- **The panel points the window at two things, and the window answers them
  differently.** `Reveal` is a tagged union rather than an agent id because an
  agent is `select` and For you is the desk. The two lists are compared by
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
