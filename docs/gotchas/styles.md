# Styles

Every rule here breaks in a way no DOM assertion can see: a cascade that lost on
source order, a flex item that shrank, a token read by an element nothing else
descends from. `styles.test.ts` is the gate, and *Every length is named* in
`AGENTS.md` is the rule they all sit under.

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
- **`--flesh` and `--flesh-soft` are pinned on `.rail`.** The rail is dark in
  both surfaces and reads both tokens, so a dark value for either would repaint
  it and no test would notice. Pinning them on the one element every rail rule
  descends from makes the rail a color scope rather than a naming convention.
- **`data-surface` is only ever `light` or `dark`.** `system` is resolved before
  it reaches the document. A stylesheet rule keyed on `system` would have to
  duplicate the one keyed on `dark`, and CSS has no way to share them.
