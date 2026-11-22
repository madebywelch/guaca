import { type ReactNode, useContext } from "react";

import { MAPLE } from "../avatars/Accent";
import { type Season, SeasonContext, useSeason } from "../lib/season";
import { useStore } from "../lib/store";

export function SeasonalLooks({ children }: { children: ReactNode }) {
  const season = useSeason(useStore((state) => state.prefs.seasonal));
  return <SeasonContext.Provider value={season}>{children}</SeasonContext.Provider>;
}

export const GREETINGS: Record<Season, string> = {
  thanksgiving: "Happy Thanksgiving",
  christmas: "Merry Christmas",
};

const HOLLY_LEAF =
  "M0 0Q1.5-1.2 2-3Q3-1.6 4.5-2.6Q5.2-1 7-2.4Q7.6-.8 10 0Q7.6.8 7 2.4Q5.2 1 4.5 2.6Q3 1.6 2 3Q1.5 1.2 0 0Z";

const LEAVES = ["d-rust", "d-olive", "d-ochre", "d-pumpkin", "d-rust", "d-bark"] as const;
const BULBS = ["d-cranberry", "d-gold", "d-pine", "d-cream"] as const;
const TILE = 64;
const TILES = 48;

function Leaf({
  x,
  y,
  size,
  turn,
  tone,
}: {
  x: number;
  y: number;
  size: number;
  turn: number;
  tone: string;
}) {
  const scale = size / 24;
  return (
    <path
      className={tone}
      d={MAPLE}
      transform={`translate(${x} ${y}) rotate(${turn}) scale(${scale}) translate(-12 -12)`}
    />
  );
}

function Holly() {
  return (
    <>
      <path className="d-pine" d={HOLLY_LEAF} transform="translate(11 13) rotate(-160)" />
      <path className="d-pine" d={HOLLY_LEAF} transform="translate(13 13) rotate(-20)" />
      <circle className="d-cranberry" cx="10.4" cy="13.6" r="2" />
      <circle className="d-cranberry" cx="13.8" cy="13.8" r="2" />
      <circle className="d-cranberry" cx="12.1" cy="11.2" r="2" />
    </>
  );
}

export function SeasonGreeting() {
  const season = useContext(SeasonContext);
  if (!season) return null;
  return (
    <p className="roster__greeting">
      <svg className="roster__greeting-mark decor" viewBox="0 0 24 24" aria-hidden="true">
        {season === "christmas" ? (
          <Holly />
        ) : (
          <Leaf x={12} y={12} size={22} turn={-18} tone="d-rust" />
        )}
      </svg>
      {GREETINGS[season]}
    </p>
  );
}

function Pumpkin({ x, y, rx, ry }: { x: number; y: number; rx: number; ry: number }) {
  return (
    <g>
      <path
        className="d-olive"
        d={`M${x - 1.5} ${y - ry + 2}q-.5-6 2.5-8.5l2 1.4q-2.4 2.6-1.8 7.1z`}
      />
      <ellipse className="d-rind" cx={x - rx * 0.42} cy={y} rx={rx * 0.58} ry={ry * 0.94} />
      <ellipse className="d-rind" cx={x + rx * 0.42} cy={y} rx={rx * 0.58} ry={ry * 0.94} />
      <ellipse className="d-pumpkin" cx={x} cy={y} rx={rx * 0.5} ry={ry} />
      <ellipse
        className="d-gloss"
        cx={x - rx * 0.16}
        cy={y - ry * 0.18}
        rx={rx * 0.07}
        ry={ry * 0.55}
      />
    </g>
  );
}

function Harvest() {
  return (
    <svg className="roster__scene-art decor" viewBox="0 0 248 80" aria-hidden="true">
      <path className="d-mound" d="M14 80Q40 68 124 70T236 80Z" />
      <Leaf x={36} y={30} size={22} turn={-22} tone="d-rust" />
      <Leaf x={152} y={22} size={18} turn={14} tone="d-pumpkin" />
      <Leaf x={206} y={44} size={22} turn={-8} tone="d-ochre" />
      <Pumpkin x={48} y={66} rx={14} ry={11} />
      <Pumpkin x={104} y={58} rx={30} ry={20} />
      <Pumpkin x={170} y={64} rx={21} ry={15} />
      <Leaf x={126} y={74} size={16} turn={30} tone="d-rust" />
      <Leaf x={222} y={72} size={18} turn={-30} tone="d-olive" />
      <Leaf x={76} y={76} size={14} turn={60} tone="d-ochre" />
    </svg>
  );
}

function Tree({ x, base, height }: { x: number; base: number; height: number }) {
  const top = base - height;
  const tiers = [0, 0.27, 0.52].map((at, i) => {
    const apex = top + height * at;
    const depth = height * (0.36 + i * 0.06);
    const half = height * (0.2 + i * 0.07);
    return { apex, depth, half };
  });
  const width = height * 0.06;
  return (
    <g>
      <rect
        className="d-bark"
        x={x - width}
        y={base - height * 0.12}
        width={width * 2}
        height={height * 0.12}
      />
      {[...tiers].reverse().map(({ apex, depth, half }) => (
        <g key={apex}>
          <path className="d-pine" d={`M${x} ${apex}L${x + half} ${apex + depth}H${x - half}Z`} />
          <path
            className="d-snow"
            d={`M${x} ${apex}L${x + half * 0.42} ${apex + depth * 0.42}q-${half * 0.14} ${depth * 0.1}-${half * 0.28} 0t-${half * 0.28} 0t-${half * 0.28} 0Z`}
          />
        </g>
      ))}
      <circle
        className="d-cranberry"
        cx={x - height * 0.09}
        cy={top + height * 0.62}
        r={height * 0.035}
      />
      <circle
        className="d-gold"
        cx={x + height * 0.12}
        cy={top + height * 0.74}
        r={height * 0.035}
      />
      <circle
        className="d-cream"
        cx={x - height * 0.15}
        cy={top + height * 0.42}
        r={height * 0.03}
      />
      <circle
        className="d-cranberry"
        cx={x + height * 0.06}
        cy={top + height * 0.36}
        r={height * 0.03}
      />
      <circle
        className="d-gold"
        cx={x - height * 0.02}
        cy={top + height * 0.84}
        r={height * 0.03}
      />
      <path
        className="d-gold"
        transform={`translate(${x} ${top - 1}) scale(${height / 90})`}
        d="M0-9 2.3-3.2 8.6-2.8 3.7 1.2 5.3 7.3 0 3.9-5.3 7.3-3.7 1.2-8.6-2.8-2.3-3.2Z"
      />
    </g>
  );
}

function Pines() {
  return (
    <svg className="roster__scene-art decor" viewBox="0 0 248 128" aria-hidden="true">
      <path className="d-mound" d="M0 128V124Q124 96 248 124V128Z" />
      <Tree x={62} base={118} height={70} />
      <Tree x={186} base={118} height={64} />
      <Tree x={124} base={116} height={100} />
    </svg>
  );
}

export function SeasonScene() {
  const season = useContext(SeasonContext);
  if (!season) return null;
  return (
    <div className="roster__scene" data-season={season} aria-hidden="true">
      {season === "christmas" ? <Pines /> : <Harvest />}
    </div>
  );
}

function Vine() {
  return (
    <>
      {Array.from({ length: TILES }, (_, i) => {
        const x = i * TILE;
        const tone = LEAVES[i % LEAVES.length] as string;
        const next = LEAVES[(i + 3) % LEAVES.length] as string;
        return (
          <g key={x}>
            <path className="d-vine" d={`M${x} 2Q${x + TILE / 2} 5 ${x + TILE} 2`} />
            <Leaf x={x + 16} y={11} size={17} turn={160 + ((i * 37) % 40)} tone={tone} />
            <Leaf x={x + 47} y={10} size={14} turn={165 + ((i * 23) % 30)} tone={next} />
          </g>
        );
      })}
    </>
  );
}

function Lights() {
  return (
    <>
      {Array.from({ length: TILES }, (_, i) => {
        const x = i * TILE;
        return (
          <g key={x}>
            <path className="d-wire" d={`M${x} 2Q${x + TILE / 2} 18 ${x + TILE} 2`} />
            {[0.18, 0.39, 0.61, 0.82].map((t, j) => {
              const bx = x + TILE * t;
              const by = 2 + 2 * (1 - t) * t * 16;
              const slope = (16 * (1 - 2 * t) * 2) / TILE;
              const turn = (Math.atan(slope) * 180) / Math.PI;
              const n = i * 4 + j;
              return (
                <g
                  key={t}
                  className={`lights__bulb lights__bulb--${n % 2 ? "b" : "a"}`}
                  transform={`translate(${bx.toFixed(1)} ${by.toFixed(1)}) rotate(${turn.toFixed(1)})`}
                >
                  <rect className="d-socket" x="-1.6" y="0" width="3.2" height="3" rx="0.8" />
                  <ellipse className={BULBS[n % BULBS.length]} cx="0" cy="6.2" rx="2.6" ry="3.8" />
                </g>
              );
            })}
          </g>
        );
      })}
    </>
  );
}

export function SeasonGarland() {
  const season = useContext(SeasonContext);
  if (!season) return null;
  return (
    <svg className="pane__garland decor" data-season={season} aria-hidden="true">
      {season === "christmas" ? <Lights /> : <Vine />}
    </svg>
  );
}
