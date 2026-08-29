/**
 * Draws the crew strip in the README.
 *
 * Rendered from the app's own drawing code rather than redrawn: `form.ts` for
 * the body, `eyes.ts` for the face, `moods.ts` for the expression. A redesign
 * updates the front page by re-running this rather than by somebody noticing
 * the picture is out of date, which is what happened to the cast before this
 * one.
 *
 *   ./scripts/make-crew.sh
 *
 * One frame, held still. The strip has to say in a static picture what the app
 * says by moving, so what varies down the row is the expression: eight
 * characters, eight moods, eight directions of gaze. The eight are chosen to
 * cover all five silhouettes, because the strip is also the only place a reader
 * sees the cast is more than one shape.
 */

import { CHARACTERS } from "../src/avatars/catalog";
import { eyePath, eyesAt } from "../src/avatars/eyes";
import { bodyPoints, FORM, outline, type Point } from "../src/avatars/form";
import { markFor, MOODS, type Mood } from "../src/avatars/moods";

/**
 * Who appears, in which accent, doing what, and where it is looking.
 *
 * The moods are chosen for what survives one frame. `listening` and `surprised`
 * are a pair of open eyes held still, and next to `idle` in a picture that
 * cannot move they are the same drawing three times.
 */
const CREW: { key: string; color: string; mood: Mood; gaze: Point; t: number }[] = [
  { key: "orb", color: "#8b8f45", mood: "idle", gaze: [0.26, -0.06], t: 0.9 },
  { key: "husk", color: "#4f5f96", mood: "thinking", gaze: [-0.22, -0.16], t: 2.1 },
  { key: "crumb", color: "#bf5f3c", mood: "working", gaze: [0.12, 0.09], t: 0.32 },
  { key: "wave", color: "#5e8158", mood: "pleased", gaze: [0, -0.05], t: 1.1 },
  { key: "pip", color: "#a8453a", mood: "frustrated", gaze: [-0.04, 0.02], t: 0.7 },
  { key: "slab", color: "#5a7d99", mood: "blocked", gaze: [0.27, -0.28], t: 3.2 },
  { key: "gourd", color: "#b3805c", mood: "stuck", gaze: [-0.03, 0.06], t: 2.4 },
  { key: "pebble", color: "#6c6f70", mood: "paused", gaze: [0, 0], t: 2.6 },
];

const CELL = FORM.box;
const GAP = 6;

/* The stylesheet derives both of these from an agent's accent. A file in a
   README has no stylesheet, so the same derivation is inlined per cell:
   `color-mix` rather than precomputed hex, so the recipe stays in one place and
   this cannot drift from `.avatar` in `src/styles.css`. `--flesh` is the amber
   of the light theme, spent on one mark and nothing else. */
function palette(color: string): string {
  return [
    `--accent:${color}`,
    `--eye:color-mix(in oklab, ${color} 34%, #171410)`,
    `--flesh:#b4530a`,
  ].join(";");
}

function cell(member: (typeof CREW)[number], index: number): string {
  const lump = CHARACTERS.find((c) => c.key === member.key);
  if (!lump) throw new Error(`no character named ${member.key}`);
  const mood = MOODS[member.mood];

  const body = outline(bodyPoints(lump, mood.shape, member.t, member.gaze).pts);
  /* Not live: a still picture must not be caught mid-blink, and the jitter and
     the breath are motion with nothing to show for themselves in one frame. */
  const eyes = eyesAt(lump, mood.eye, mood.watch, member.t, false, member.gaze)
    .map((e) => `<path d="${eyePath(e)}" stroke-width="${e.h.toFixed(2)}"/>`)
    .join("");

  /* What `.avatar[data-mood="paused"]` does in the app, which is the one mood
     whose color is not its accent. */
  const dim = mood.dim ? ";filter:grayscale(0.72);opacity:0.5" : "";
  const x = index * (CELL + GAP);
  return [
    `  <g transform="translate(${x} 0)" style="${palette(member.color)}${dim}">`,
    `<path d="${body}" fill="var(--accent)"/>`,
    `<g fill="none" stroke="var(--eye)" stroke-linecap="round">${eyes}</g>`,
    markFor(member.mood),
    `</g>`,
  ].join("");
}

const width = CREW.length * CELL + (CREW.length - 1) * GAP;
const label =
  "Eight agents across the five shapes: idle, thinking, working, pleased, " +
  "frustrated, waiting on a person, stuck, and paused";

process.stdout.write(
  [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${CELL}" width="${width}" height="${CELL}" role="img" aria-label="${label}">`,
    `  <title>The crew</title>`,
    ...CREW.map(cell),
    `</svg>`,
    "",
  ].join("\n"),
);
