/**
 * The cast as it ships, drawn by `src/avatars` itself.
 *
 * Everything geometric is imported. What is ported is the frame loop from
 * `AgentAvatar.tsx`, which is a React component, and the three layers of
 * `Skin.tsx`, whose constants are not exported. Both are copied line for line
 * so the baseline on this page is the app's, not a sketch of it.
 */

import { AIM, aimedEye, blendEyes, eyePath, eyesAt, gazeAt, settle } from "../../src/avatars/eyes";
import { blend, bodyPoints, outline } from "../../src/avatars/form";
import { MOODS, markFor, type Mood } from "../../src/avatars/moods";
import { C, type Design, el, type Point, REACH, uid } from "./shared";

const MORPH = 0.6;
const KNOCK = {
  send: { amp: 0.26, hz: 1.5, decay: 0.28, life: 0.9 },
  receive: { amp: 0.3, hz: 2.2, decay: 0.3, life: 0.8 },
};
const PAPER = {
  ink: "#252824",
  shadow: { x: 1.1, y: 2.3, opacity: 0.18 },
  edge: { x: -0.4, y: -0.5, width: 0.55, opacity: 0.45 },
};

/** The three layers of `Skin.tsx`, over one outline. Returns the outline. */
export function paper(svg: SVGSVGElement, id: string): SVGPathElement {
  const skin = el("g", { "clip-path": `url(#${id}-reach)` }, svg);
  const defs = el("defs", {}, skin);
  const shape = el("path", { id: `${id}-shape` }, defs);
  const clip = el("clipPath", { id: `${id}-reach` }, defs);
  el("circle", { cx: C, cy: C, r: REACH }, clip);
  el(
    "use",
    {
      href: `#${id}-shape`,
      x: PAPER.shadow.x,
      y: PAPER.shadow.y,
      fill: PAPER.ink,
      opacity: PAPER.shadow.opacity,
    },
    skin,
  );
  el("use", { href: `#${id}-shape`, fill: "var(--accent)" }, skin);
  el(
    "use",
    {
      href: `#${id}-shape`,
      x: PAPER.edge.x,
      y: PAPER.edge.y,
      fill: "none",
      stroke: "#fff",
      "stroke-width": PAPER.edge.width,
      "stroke-opacity": PAPER.edge.opacity,
    },
    skin,
  );
  return shape;
}

export const baseline: Design = {
  key: "now",
  name: "Now",
  create(svg, who) {
    const shape = paper(svg, uid("now"));
    const face = el(
      "g",
      { fill: "none", stroke: "var(--eye)", "stroke-linecap": "round" },
      svg,
    );
    const eyes = [el("path", {}, face), el("path", {}, face)];
    const mark = el("g", { class: "avatar__mark" }, svg);
    const mass = { gaze: [0, 0] as Point, vel: [0, 0] as Point };
    let marked: Mood | null = null;

    return {
      paint(f) {
        if (marked !== f.mood) {
          marked = f.mood;
          mark.innerHTML = markFor(f.mood);
          svg.classList.toggle("is-dim", Boolean(MOODS[f.mood].dim));
        }
        const to = MOODS[f.mood];
        const from = MOODS[f.from];
        const raw = f.live ? Math.min(1, f.since / MORPH) : 1;
        const u = raw * raw * (3 - 2 * raw);

        let eyeGaze: Point;
        if (f.look) eyeGaze = [0, f.look === "up" ? -AIM.up : AIM.down];
        else if (!f.live) eyeGaze = [0, 0];
        else if (f.point) eyeGaze = f.point;
        else {
          const a = gazeAt(f.t, from.watch?.gaze);
          const b = gazeAt(f.t, to.watch?.gaze);
          eyeGaze = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
        }
        if (f.live) settle(mass, eyeGaze, f.dt);
        else {
          mass.gaze = [eyeGaze[0], eyeGaze[1]];
          mass.vel = [0, 0];
        }
        eyeGaze = [mass.gaze[0], mass.gaze[1]];
        const bodyGaze: Point = [mass.gaze[0], mass.gaze[1]];

        if (f.gesture && f.live) {
          const knock = KNOCK[f.gesture];
          if (f.age < knock.life) {
            const wave =
              f.gesture === "send"
                ? Math.sin(f.age * knock.hz * Math.PI * 2)
                : Math.cos(f.age * knock.hz * Math.PI * 2);
            const away = f.look === "down" ? -1 : 1;
            const push = away * knock.amp * wave * Math.exp(-f.age / knock.decay);
            bodyGaze[1] += push;
            eyeGaze[1] += push * 0.35;
          }
        }

        const next = bodyPoints(who, to.shape, f.t, bodyGaze);
        const pts =
          u >= 1 ? next.pts : blend(bodyPoints(who, from.shape, f.t, bodyGaze).pts, next.pts, u);
        shape.setAttribute("d", outline(pts));

        const blended = u >= 1 ? to.eye : blendEyes(from.eye, to.eye, u);
        const eye = f.look ? aimedEye(blended, f.look) : blended;
        const watch = u > 0.5 ? to.watch : from.watch;
        const drawn = eyesAt(who, eye, watch, f.t, f.live, eyeGaze);
        eyes.forEach((path, i) => {
          const e = drawn[i];
          path.setAttribute("d", e ? eyePath(e) : "");
          if (e) path.setAttribute("stroke-width", e.h.toFixed(2));
        });
      },
    };
  },
};
