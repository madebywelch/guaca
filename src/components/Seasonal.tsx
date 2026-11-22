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
};

const LEAVES = ["d-rust", "d-olive", "d-ochre", "d-pumpkin", "d-rust", "d-bark"] as const;
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

export function SeasonGreeting() {
  const season = useContext(SeasonContext);
  if (!season) return null;
  return (
    <p className="roster__greeting">
      <svg className="roster__greeting-mark decor" viewBox="0 0 24 24" aria-hidden="true">
        <Leaf x={12} y={12} size={22} turn={-18} tone="d-rust" />
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

export function SeasonScene() {
  const season = useContext(SeasonContext);
  if (!season) return null;
  return (
    <div className="roster__scene" data-season={season} aria-hidden="true">
      <Harvest />
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

export function SeasonGarland() {
  const season = useContext(SeasonContext);
  if (!season) return null;
  return (
    <svg className="pane__garland decor" data-season={season} aria-hidden="true">
      <Vine />
    </svg>
  );
}
