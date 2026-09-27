# Styles

Every rule here breaks in a way no DOM assertion can see: a cascade that lost on
source order, a flex item that shrank, a color written for paper and forgotten
in the ink block. `styles.test.ts` reads the stylesheet itself and is the gate,
and *Every length is named* in `AGENTS.md` is the rule they all sit under.

- **The full file view resets `overflow`, not just the height cap and the mask.**
  A clipping flex item has an automatic minimum size of zero, so a document left
  clipping shrinks to fit and eats the rest of itself, nothing overflows the body
  and the reading view has no scrollbar. The body is the only thing in that
  dialog that scrolls, so nothing inside it may clip.
- **`.dialog.dialog--file` is doubled because `.dialog` is declared after it.**
  A one-class modifier above the base rule loses every property they share on
  source order, which is invisible in a diff: the reading view opened at the
  ordinary 38rem for that reason. `styles.test.ts` walks the modifiers.
- **A chip's label is never shrunk to make room for what came back.** Flex
  shrinks in proportion to what each item asked for, so a refusal running to a
  paragraph took the row and left the label as `U…`: a chip saying one
  character about which call went wrong. A weighting is not the fix, at a
  hundred to one it still cost the last letter. The label does not shrink, the
  answer takes what is left, the chip clips the rest, and the refusal opens
  underneath where a command opens. `styles.test.ts` is the gate, because no
  DOM assertion sees a layout.
- **The rail and the inspector are one ground, and it is not the page's.** They
  were a near-black column on the left and a white panel on the right, which is
  three surfaces for two jobs: the app's own two edges did not look like each
  other and the heavier of them carried the least reading. They do the same job,
  so they are the same off-white, the page is the only white thing on screen,
  and the only saturation left is an agent's color and the one amber. The
  columns used to pin `--attention` and `--attention-wash`, and the crews' column
  `--danger` too, because they were ink under a reading column that could go dark. Nothing
  is pinned now, and what replaced the pins is the same trap read the other way:
  a `--rail-*` or `--grail-*` color declared for paper and forgotten in the ink
  block is a column that stays off-white in a dark room, which no DOM assertion
  sees. `styles.test.ts` reads both blocks and is the gate.
- **A `-wash` name is a ground, and drawing ink with one is invisible on both
  surfaces.** The bar marking which crew the rail is inside was the attention
  wash for as long as the crews' column has existed: 1.06 to 1 against the
  column on paper, 1.31 to 1 in a dark room. It is the trap two bullets down
  read a third way: not a column color declared for paper and forgotten in the
  ink block, but ink spelled from a name that was never ink, which both blocks
  then move faithfully and neither shows. The pairs are
  `--attention`/`--attention-wash` and `--system`/`--system-wash`, and the wash
  exists to fill a box behind something else. `styles.test.ts` reads the mark's
  token out of the rule, resolves it in both `:root` blocks and holds it to the
  3:1 WCAG asks of a mark that is not text.
- **A `-fill` carries its own text color, and the page's is the wrong one.** The
  asking chip in the rail set its text in `--ground` on the amber, which was
  right while the amber was dark on paper and light on ink. A fill is the same
  on both surfaces, so text in a page color is white on pale amber in one of
  them. Text on `--attention-fill` is `--on-attention-fill`, and on
  `--danger-fill` it is `--on-danger-fill`. `styles.test.ts` fails a rule that
  fills a background with one and colors its text with anything else.
- **The amber is not the accent, it is the request.** It had spread to the
  focus ring, primary and send buttons, toggles, links, running jobs and the
  streaming caret, so "you tabbed here" and "answer me" were one ring. Focus and
  drop targets are `--focus`, which is the system's slate; primary and on-states
  are ink; activity and links are `--system`. `styles.test.ts` fails a focus
  rule, a primary control or an on-state spelled from `--attention`.
- **No color in the `:root` blocks is a decision to edit.** They are the default
  theme as `lib/theme.ts` solves it, and `theme.test.ts` holds them equal. A hex
  nudged there fails the suite; a hex nudged in both places fails the gates the
  solver exists to meet. Change the ratio, the ladder or a family's hue and
  chroma in `theme.ts`, then copy what it solves.
- **The theme is written as blocks after the stylesheet, not as properties on
  the root.** Written inline, a `--rail-*` value is pinned to whichever surface
  was current when it was written, and switching to ink leaves the columns
  paper. `appearance.test.ts` fails if any theme token reaches the root's style
  attribute, and asserts the `<style>` is last in the head, since equal
  specificity means order is all that makes it win.
- **Reading size is `--type-read`, not `--type-body`.** The body step is also
  the size of titles in the rail, the palette and a dialog. Pointing the reading
  preference at it moved all of them, which is the interface scale again.
- **A control's border is `--edge-control`, not `--edge`.** `--edge` is a
  divider at 1.3 to 1, and a divider is decoration. The boundary of something
  typed into or pressed is how it is found, and is held to 3 to 1. A hover
  state steps past it to `--muted`; a hover that stops at the resting border is
  invisible.
- **A column's recessed surface is not the page's either.** `--sunken` is a hair
  off white, which is a field on paper and nothing at all on an off-white panel,
  so the three columns remap it onto `--rail-sunken` in one rule that names all
  of them. Remapped there rather than at each rule inside, so a row added to the
  inspector tomorrow is recessed from what it is actually drawn on.
- **One surface is ink whichever surface the operator picked, and it says so
  itself.** The full-window machine viewer pins `--stage-*` on `.screen`,
  shadow included: a pale chrome around somebody else's desktop is a chrome the
  eye keeps reading instead of the picture, and a `--lift-*` there would resolve
  against the reading column and put a paper-weight ring on a black surface.
- **`data-surface` is only ever `light` or `dark`.** `system` is resolved before
  it reaches the document. A stylesheet rule keyed on `system` would have to
  duplicate the one keyed on `dark`, and CSS has no way to share them.
