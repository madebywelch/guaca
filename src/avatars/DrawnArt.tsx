import { type RefObject, useId, useRef } from "react";

import {
  brush,
  DRAWN,
  emanata,
  FPS,
  faceAt,
  hint,
  mixFace,
  POSES,
  pigment,
  seedOf,
  tuftOf,
  tuftPath,
} from "./drawn";
import { blend, bodyPoints, FORM } from "./form";
import type { Paint } from "./frame";
import { MOODS, markFor } from "./moods";

/**
 * The drawn cast: a brush line on twos, over a pigment out of register.
 *
 * Owns its elements and redraws them twelve times a second. Between drawings
 * it does nothing but keep the mark on its head swinging, which is simulated
 * every frame so its lag is the same whatever the frame rate, and drawn on
 * twos like the rest.
 */
export function DrawnArt({ paint }: { paint: RefObject<Paint | null> }) {
  const id = useId();
  const fill = useRef<SVGPathElement>(null);
  const tuft = useRef<SVGPathElement>(null);
  const line = useRef<SVGPathElement>(null);
  const face = useRef<SVGGElement>(null);
  const mark = useRef<SVGGElement>(null);
  const state = useRef({ angle: 0, vel: 0, drawn: "", beside: "" });

  paint.current = (f) => {
    const s = state.current;
    const to = DRAWN[f.mood];
    const from = DRAWN[f.from];

    /* The mark on its head: pulled back against a sideways look, knocked
       over by a message landing, and swinging back on a spring. */
    if (f.live) {
      const want =
        to.tuft.rest - f.body[0] * 60 + (f.gesture === "receive" && f.sinceGesture < 0.3 ? 30 : 0);
      const k = 90 * to.tuft.stiff;
      s.vel += (k * (want - s.angle) - 2 * Math.sqrt(k) * 0.35 * s.vel) * f.dt;
      s.angle += s.vel * f.dt;
      if (f.mood === "frustrated") s.angle += Math.sin(f.t * 60) * 1.4;
    } else {
      s.angle = to.tuft.rest;
      s.vel = 0;
    }

    const frame = f.live ? Math.floor(f.t * FPS) : 0;
    const pose = f.live ? Math.min(POSES.length - 1, Math.floor(f.since * FPS)) : POSES.length - 1;
    const key = `${f.lump.key}|${f.px}|${frame}|${pose}|${f.mood}|${f.from}|${f.look ?? ""}`;
    if (key === s.drawn) return;
    const pigmentPath = fill.current;
    const edge = line.current;
    const head = tuft.current;
    const strokes = face.current;
    if (!pigmentPath || !edge || !head || !strokes) return;
    s.drawn = key;

    const t = f.live ? frame / FPS : f.t;
    const u = POSES[pose] as number;
    const settled = pose === POSES.length - 1;

    const next = bodyPoints(f.lump, MOODS[f.mood].shape, t, f.body).pts;
    const pts = settled
      ? next
      : blend(bodyPoints(f.lump, MOODS[f.from].shape, t, f.body).pts, next, u);
    pigmentPath.setAttribute("d", pigment(pts));

    const seed = seedOf(f.lump.key);
    const ln = settled ? to.line : mixFace(from.line, to.line, Math.max(0, Math.min(1, u)));
    const boil = ln.rate > 0 && f.live ? Math.floor(f.t * ln.rate) : 0;
    edge.setAttribute("d", brush(pts, ln, boil, seed, hint(f.px, 1.7)));
    head.setAttribute(
      "d",
      tuftPath(tuftOf(f.lump.key), pts, s.angle, ln.weight * 0.85 * hint(f.px, 1.6)),
    );

    const held = u > 0.5 ? to : from;
    const drawn = faceAt(
      f.lump,
      {
        ...held,
        eye: settled ? to.eye : mixFace(from.eye, to.eye, u),
        brow: settled ? to.brow : mixFace(from.brow, to.brow, u),
        mouth: settled ? to.mouth : mixFace(from.mouth, to.mouth, u),
      },
      {
        t,
        live: f.live,
        gaze: f.eyes,
        ...(f.look ? {} : { watch: MOODS[f.mood].watch?.gaze }),
        px: f.px,
      },
      pts,
    );
    const [eyeA, eyeB, browA, browB, mouth, o] = Array.from(strokes.children) as [
      SVGPathElement,
      SVGPathElement,
      SVGPathElement,
      SVGPathElement,
      SVGPathElement,
      SVGEllipseElement,
    ];
    eyeA.setAttribute("d", drawn.eyes[0] ?? "");
    eyeB.setAttribute("d", drawn.eyes[1] ?? "");
    browA.setAttribute("d", drawn.brows[0] ?? "");
    browB.setAttribute("d", drawn.brows[1] ?? "");
    mouth.setAttribute("d", drawn.mouth);
    o.setAttribute("cx", (drawn.o?.x ?? 0).toFixed(2));
    o.setAttribute("cy", (drawn.o?.y ?? 0).toFixed(2));
    o.setAttribute("rx", (drawn.o?.rx ?? 0).toFixed(2));
    o.setAttribute("ry", (drawn.o?.ry ?? 0).toFixed(2));
    o.setAttribute("stroke-width", (drawn.o?.w ?? 0).toFixed(2));

    /* Written only when it changes: most marks hold for several drawings, and
       markup is the one thing here that is not an attribute. The amber one is
       `markFor`'s, the same in both casts. */
    const beside =
      to.mark === "bang"
        ? markFor(f.mood)
        : emanata(to.mark, { frame, since: f.since, live: f.live, px: f.px }, pts, seed);
    if (beside !== s.beside && mark.current) {
      s.beside = beside;
      mark.current.innerHTML = beside;
    }
  };

  return (
    <>
      <defs>
        <clipPath id={`${id}-reach`}>
          <circle cx={FORM.center} cy={FORM.center} r={FORM.reach} />
        </clipPath>
      </defs>
      <g className="avatar__drawn" clipPath={`url(#${id}-reach)`}>
        <path ref={fill} fill="var(--accent)" />
        <path ref={tuft} className="avatar__edge" />
        <path ref={line} className="avatar__edge" />
      </g>
      <g ref={face} className="avatar__brush">
        <path />
        <path />
        <path />
        <path />
        <path />
        <ellipse />
      </g>
      <g ref={mark} className="avatar__mark" />
    </>
  );
}
