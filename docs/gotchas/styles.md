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
  columns used to pin `--flesh` and `--flesh-soft` — the crews' column `--alarm`
  too — because they were ink under a reading column that could go dark. Nothing
  is pinned now, and what replaced the pins is the same trap read the other way:
  a `--rail-*` or `--grail-*` color declared for paper and forgotten in the ink
  block is a column that stays off-white in a dark room, which no DOM assertion
  sees. `styles.test.ts` reads both blocks and is the gate.
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
