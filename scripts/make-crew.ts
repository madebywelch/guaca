import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CHARACTERS } from "../src/avatars/catalog";
import { eyesAt, LASH_REACH, pathOf } from "../src/avatars/eyes";
import { bodyPoints, FORM, outline, type Point } from "../src/avatars/form";
import { markFor, MOODS, type Mood } from "../src/avatars/moods";
import { Skin } from "../src/avatars/Skin";

export const CREW: { key: string; color: string; mood: Mood; gaze: Point; t: number }[] = [
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

function palette(color: string): string {
  return [`--accent:${color}`, "--eye:#252824", "--sclera:#fbf8f1", "--attention-fill:#ffb346", "--on-attention-fill:#161616"].join(";");
}

function cell(member: (typeof CREW)[number], index: number): string {
  const lump = CHARACTERS.find((c) => c.key === member.key);
  if (!lump) throw new Error(`no character named ${member.key}`);
  const mood = MOODS[member.mood];

  const body = outline(bodyPoints(lump, mood.shape, member.t, member.gaze).pts);
  const at = { t: member.t, live: false, gaze: member.gaze };
  const eyes = eyesAt(lump, mood.eye, mood.watch, at)
    .map((e, i) => {
      const open = `crew-${index}-open-${i}`;
      const lid = `crew-${index}-lid-${i}`;
      const opening = pathOf(e.opening, true);
      return [
        `<defs><clipPath id="${open}"><path d="${opening}"/></clipPath>`,
        `<clipPath id="${lid}"><circle cx="${e.x.toFixed(2)}" cy="${e.y.toFixed(2)}" r="${(e.r * LASH_REACH).toFixed(2)}"/></clipPath></defs>`,
        `<path d="${opening}" fill="var(--sclera)"/>`,
        `<g clip-path="url(#${open})"><circle cx="${e.pupil.x.toFixed(2)}" cy="${e.pupil.y.toFixed(2)}" r="${e.pupil.r.toFixed(2)}" fill="var(--eye)"/>`,
        `<path d="${pathOf(e.shade, true)}" fill="var(--eye)" opacity="0.16"/></g>`,
        `<g clip-path="url(#${lid})" fill="none" stroke="var(--eye)" stroke-linecap="round" stroke-linejoin="round">`,
        `<path d="${pathOf(e.lash, false)}" stroke-width="${e.lashWidth.toFixed(2)}"/>`,
        `<path d="${pathOf(e.under, false)}" stroke-width="${e.underWidth.toFixed(2)}" opacity="${e.underOpacity.toFixed(2)}"/></g>`,
      ].join("");
    })
    .join("");

  const dim = mood.dim ? ";filter:grayscale(0.72);opacity:0.5" : "";
  const x = index * (CELL + GAP);
  const skin = renderToStaticMarkup(createElement("svg", null, createElement(Skin, { d: body })), {
    identifierPrefix: `crew-${index}-`,
  }).replace(/^<svg>|<\/svg>$/g, "");
  return [
    `  <g transform="translate(${x} 0)" style="${palette(member.color)}${dim}">`,
    skin,
    eyes,
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
