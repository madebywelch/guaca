import { Fragment, type RefObject, useId, useRef } from "react";

import { blendEyes, cueBlink, type Drawn, eyesAt, LASH_REACH, pathOf, saccadeBlink } from "./eyes";
import { blend, bodyPoints, outline } from "./form";
import type { Paint } from "./frame";
import { MOODS, MORPH, type Mood, markFor } from "./moods";
import { Skin } from "./Skin";

const HINT = { px: 44, most: 2.2 } as const;

export function CutArt({ paint }: { paint: RefObject<Paint | null> }) {
  const id = useId();
  const skin = useRef<SVGPathElement>(null);
  const clips = useRef<SVGDefsElement>(null);
  const eyes = useRef<SVGGElement>(null);
  const mark = useRef<SVGGElement>(null);
  const marked = useRef<Mood | null>(null);

  paint.current = (f) => {
    const body = skin.current;
    const face = eyes.current;
    const defs = clips.current;
    if (!body || !face || !defs) return;

    if (marked.current !== f.mood) {
      marked.current = f.mood;
      if (mark.current) mark.current.innerHTML = markFor(f.mood);
    }

    const to = MOODS[f.mood];
    const from = MOODS[f.from];
    const raw = f.live ? Math.min(1, f.since / MORPH) : 1;
    const u = raw * raw * (3 - 2 * raw);

    const next = bodyPoints(f.lump, to.shape, f.t, f.body);
    const pts =
      u >= 1 ? next.pts : blend(bodyPoints(f.lump, from.shape, f.t, f.body).pts, next.pts, u);
    body.setAttribute("d", outline(pts));

    const eye = u >= 1 ? to.eye : blendEyes(from.eye, to.eye, u);
    const watch = u > 0.5 ? to.watch : from.watch;
    const cue = f.live
      ? Math.max(f.look ? 0 : saccadeBlink(f.t, watch?.gaze), cueBlink(f.sinceLook))
      : 0;
    const weight = Math.max(1, Math.min(HINT.most, HINT.px / f.px));
    write(
      face,
      defs,
      eyesAt(f.lump, eye, watch, { t: f.t, live: f.live, gaze: f.eyes, cue, weight }),
    );
  };

  return (
    <>
      <Skin pathRef={skin} />
      <defs ref={clips}>
        {[0, 1].map((i) => (
          <Fragment key={i}>
            <clipPath id={`${id}-open-${i}`}>
              <path />
            </clipPath>
            <clipPath id={`${id}-lid-${i}`}>
              <circle />
            </clipPath>
          </Fragment>
        ))}
      </defs>
      <g ref={eyes} className="avatar__eyes">
        {[0, 1].map((i) => (
          <g key={i}>
            <path className="avatar__white" />
            <g clipPath={`url(#${id}-open-${i})`}>
              <circle className="avatar__pupil" />
              <path className="avatar__shade" />
            </g>
            <g clipPath={`url(#${id}-lid-${i})`} className="avatar__lids">
              <path />
              <path />
            </g>
          </g>
        ))}
      </g>
      <g ref={mark} className="avatar__mark" />
    </>
  );
}

function write(group: SVGGElement, defs: SVGDefsElement, drawn: Drawn[]) {
  for (let i = 0; i < 2; i++) {
    const eye = drawn[i];
    const parts = group.children[i] as SVGGElement;
    const [white, inside, lids] = Array.from(parts.children) as [
      SVGPathElement,
      SVGGElement,
      SVGGElement,
    ];
    const [pupil, shade] = Array.from(inside.children) as [SVGCircleElement, SVGPathElement];
    const [lash, under] = Array.from(lids.children) as [SVGPathElement, SVGPathElement];
    const opening = defs.children[i * 2]?.firstElementChild;
    const reach = defs.children[i * 2 + 1]?.firstElementChild;
    if (!eye) {
      parts.setAttribute("display", "none");
      continue;
    }
    parts.removeAttribute("display");
    const d = pathOf(eye.opening, true);
    white.setAttribute("d", d);
    opening?.setAttribute("d", d);
    reach?.setAttribute("cx", eye.x.toFixed(2));
    reach?.setAttribute("cy", eye.y.toFixed(2));
    reach?.setAttribute("r", (eye.r * LASH_REACH).toFixed(2));
    pupil.setAttribute("cx", eye.pupil.x.toFixed(2));
    pupil.setAttribute("cy", eye.pupil.y.toFixed(2));
    pupil.setAttribute("r", eye.pupil.r.toFixed(2));
    shade.setAttribute("d", pathOf(eye.shade, true));
    lash.setAttribute("d", pathOf(eye.lash, false));
    lash.setAttribute("stroke-width", eye.lashWidth.toFixed(2));
    under.setAttribute("d", pathOf(eye.under, false));
    under.setAttribute("stroke-width", eye.underWidth.toFixed(2));
    under.setAttribute("opacity", eye.underOpacity.toFixed(2));
  }
}
