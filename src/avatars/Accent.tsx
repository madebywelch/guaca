import type { Season } from "../lib/season";
import { FORM, type Lump, perch } from "./form";

export const MAPLE =
  "M12 1 13.4 4.6 15.4 3.8 14.8 8.2 17.8 5.8 18.6 7.6 21.8 7 20.4 10.4 22.6 11.6 17.4 15.6 18 17.6 12.9 16.8 12.9 22.6 11.1 22.6 11.1 16.8 6 17.6 6.6 15.6 1.4 11.6 3.6 10.4 2.2 7 5.4 7.6 6.2 5.8 9.2 8.2 8.6 3.8 10.6 4.6Z";

export const SEAT: Record<Season, { angle: number; sink: number }> = {
  thanksgiving: { angle: -Math.PI / 2 + 0.75, sink: 3 },
  christmas: { angle: -Math.PI / 2 + 0.38, sink: 4 },
};

export function seatOf(lump: Lump, season: Season): { x: number; y: number; tilt: number } {
  const { angle, sink } = SEAT[season];
  const [px, py] = perch(lump, angle);
  const dx = px - FORM.center;
  const dy = py - FORM.center;
  const out = Math.hypot(dx, dy) || 1;
  return {
    x: px - (dx / out) * sink,
    y: py - (dy / out) * sink,
    tilt: ((angle + Math.PI / 2) * 180) / Math.PI,
  };
}

function Hat() {
  return (
    <>
      <path className="d-hat" d="M-10 0Q-7-16 5-15.5Q11.5-15 15-8Q9.5-11.5 5-8.5Q8-4.5 10 0Z" />
      <circle className="d-fur" cx="15" cy="-7.6" r="3.3" />
      <rect className="d-fur" x="-11.5" y="-2.8" width="23" height="5.6" rx="2.8" />
    </>
  );
}

function Leaf() {
  return (
    <g transform="translate(-10.2 -18.8) scale(0.85)">
      <path className="d-rust" d={MAPLE} />
    </g>
  );
}

export function Accent({ lump, season }: { lump: Lump; season: Season }) {
  const { x, y, tilt } = seatOf(lump, season);
  return (
    <g
      className="avatar__accent"
      data-accent={season}
      transform={`translate(${x.toFixed(2)} ${y.toFixed(2)}) rotate(${tilt.toFixed(1)})`}
    >
      {season === "christmas" ? <Hat /> : <Leaf />}
    </g>
  );
}
