/**
 * Draws the crew strip in the README.
 *
 * Rendered from `src/avatars/catalog.tsx`, not redrawn: the characters are the
 * app's own, so a redesign updates the front page by re-running this rather
 * than by someone noticing the picture is out of date.
 *
 *   ./scripts/make-crew.sh
 */

import { renderToStaticMarkup } from "react-dom/server";

import { CHARACTERS, drawCharacter, FORM } from "../src/avatars/catalog";

/** Who appears, in which color, and where they are looking. */
const CREW: { key: string; color: string; look: "left" | "right" | "up" | "down" | null }[] = [
  { key: "avocado", color: "#c7d96b", look: "right" },
  { key: "tomato", color: "#d9534f", look: "left" },
  { key: "lime", color: "#6faa5c", look: "up" },
  { key: "chilli", color: "#e2674a", look: "right" },
  { key: "radish", color: "#d97ea8", look: null },
  { key: "corn", color: "#e8b84b", look: "left" },
  { key: "mushroom", color: "#c2926b", look: "down" },
  { key: "carrot", color: "#e8954b", look: "right" },
];

/** How far a pupil slides. The app moves them the same distance. */
const GLANCE: Record<string, [number, number]> = {
  left: [-1.9, 0],
  right: [1.9, 0],
  up: [0, -1.9],
  down: [0, 1.9],
};

const CELL = 64;
const GAP = 6;

function markup(): string {
  const cells = CREW.map((member, index) => {
    const character = CHARACTERS.find((c) => c.key === member.key);
    if (!character) throw new Error(`no character named ${member.key}`);

    let drawn = renderToStaticMarkup(drawCharacter(character));

    // The app slides pupils with a CSS class it sets on a wrapper. There is no
    // wrapper here, so the same offset is applied to the same elements.
    const glance = member.look ? GLANCE[member.look] : null;
    if (glance) {
      drawn = drawn.replaceAll(
        'class="avatar__pupil"',
        `class="avatar__pupil" transform="translate(${glance[0]} ${glance[1]})"`,
      );
    }

    const x = index * (CELL + GAP);
    // The catalog paints with custom properties the stylesheet derives from an
    // agent's accent. A file in a README has no stylesheet, so the same
    // derivation is inlined per cell. `color-mix` rather than precomputed hex,
    // so the recipe stays in one place and this cannot drift from the app.
    const palette = [
      `--char-fill:${member.color}`,
      `--char-light:color-mix(in oklab, ${member.color} 46%, #ffffff)`,
      `--char-shade:color-mix(in oklab, ${member.color} 62%, #33240f)`,
      `--char-deep:color-mix(in oklab, ${member.color} 36%, #17200f)`,
      `--char-white:#fdfdf5`,
      `--char-ink:#17210f`,
    ].join(";");
    return `  <g transform="translate(${x} 0)" style="${palette}">${drawn}</g>`;
  });

  const width = CREW.length * CELL + (CREW.length - 1) * GAP;
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${CELL}" width="${width}" height="${CELL}" role="img" aria-label="Eight of Guaca's agent characters, looking around">`,
    `  <title>The crew</title>`,
    ...cells,
    `</svg>`,
    "",
  ].join("\n");
}

// Sanity: the catalog promises nothing is drawn outside this box, and a strip
// that violated it would clip its neighbors.
if (FORM.right > CELL || FORM.bottom > CELL) {
  throw new Error("a character no longer fits its cell; the strip would clip");
}

process.stdout.write(markup());
