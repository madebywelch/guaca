import { type Ref, useId } from "react";

import { FORM } from "./form";

const PAPER = {
  ink: "#252824",
  shadow: { x: 1.1, y: 2.3, opacity: 0.18 },
  edge: { x: -0.4, y: -0.5, width: 0.55, opacity: 0.45 },
};

export function Skin({ d, pathRef }: { d?: string; pathRef?: Ref<SVGPathElement> }) {
  const id = useId();
  const shape = `${id}-shape`;
  const reach = `${id}-reach`;
  return (
    <g className="avatar__skin" clipPath={`url(#${reach})`}>
      <defs>
        <path id={shape} ref={pathRef} d={d} />
        <clipPath id={reach}>
          <circle cx={FORM.center} cy={FORM.center} r={FORM.reach} />
        </clipPath>
      </defs>
      <use
        href={`#${shape}`}
        x={PAPER.shadow.x}
        y={PAPER.shadow.y}
        fill={PAPER.ink}
        opacity={PAPER.shadow.opacity}
      />
      <use href={`#${shape}`} fill="var(--accent)" />
      <use
        href={`#${shape}`}
        x={PAPER.edge.x}
        y={PAPER.edge.y}
        fill="none"
        stroke="#fff"
        strokeWidth={PAPER.edge.width}
        strokeOpacity={PAPER.edge.opacity}
      />
    </g>
  );
}
