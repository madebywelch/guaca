import type { ReactNode } from "react";

/**
 * Agent characters.
 *
 * The crew is the recipe. Every agent is a different ingredient, because the
 * silhouette is the only thing that still separates two agents at the 22px the
 * composer draws them at. Color does not: the previous set was one shared body
 * in twelve accents, and a rail of it read as one agent twelve times.
 *
 * Variety in the outline only survives if nothing else varies. Every ingredient
 * therefore obeys one construction:
 *
 *   - it fills the same optical box, so a row of them sits at one weight
 *   - it carries the same `FORM.rim` stroke in `deep`, so nothing floats
 *   - it takes light from the same direction, as one stroke in `light`
 *   - its face derives from a single number, so no face can drift from the set
 *
 * An earlier pass gave each agent a freehand outline and the rail looked like a
 * sticker sheet. That was a craft failure, not an argument against variety, and
 * the fix is a spec rather than a single shape. The pass after it deleted the
 * variety instead and left an egg.
 *
 * Agents store the preset `key`. No drawing is persisted, so any of this can be
 * redrawn without touching the database, and `ALIASES` keeps agents made by
 * every earlier set rendering.
 */

const FILL = "var(--char-fill)";
const SHADE = "var(--char-shade)";
const DEEP = "var(--char-deep)";
const LIGHT = "var(--char-light)";
const INK = "var(--char-ink)";
const WHITE = "var(--char-white)";

/**
 * The construction spec. Six unlike shapes only read as one cast if they agree
 * about how much room they take and how they are drawn, so these are shared and
 * `catalog.test.ts` holds every character to them.
 */
export const FORM = {
  /** Nothing is drawn outside this box. */
  left: 12,
  right: 52,
  top: 6,
  bottom: 59,
  /** Where a character's mass wants to sit, so a row of them shares a baseline. */
  centerX: 32,
  centerY: 36,
  /** On every silhouette without exception. An edge is the difference between a
      drawn character and a colored region. */
  rim: 2.2,
  /** One stroke, always down the upper left. Light comes from one place. */
  sheen: 2.6,
} as const;

// ---- faces ----------------------------------------------------------------

type EyeKind = "open" | "shut" | "wink" | "half" | "dot" | "tall" | "stern" | "sparkle";
type MouthKind = "smile" | "grin" | "flat" | "oh" | "smirk" | "cat" | "tiny" | "wobble";

/**
 * A face is one number.
 *
 * `r` drives the pupil, the highlight, the mouth's width, its drop and its
 * stroke. A wide tomato and a narrow chilli then carry the same face at two
 * scales instead of two different drawings, which is the whole reason a row of
 * different vegetables still reads as one crew.
 */
export interface Face {
  /** Center of the eye line. */
  y: number;
  /** Half the gap between the eyes. Narrow bodies pull this in. */
  spread: number;
  /** Eye radius. Everything else derives from it. */
  r: number;
  /** Faces sit on the body's axis, which is not always the middle of the box. */
  cx?: number;
  eyes?: EyeKind;
  mouth: MouthKind;
}

/**
 * Everything a face derives from `r`. Shared with the test, which uses them to
 * bound a face without drawing it, so the spec has one source of truth.
 */
export const FACE = {
  pupil: 0.55,
  specular: 0.22,
  /** Far enough below the eyes to leave a brow, close enough to stay one face. */
  drop: 1.95,
  width: 1.15,
  stroke: 0.46,
  /** Half-height of the tallest eye kind, in multiples of `r`. */
  tallest: 1.22,
} as const;

function eyeball(cx: number, cy: number, r: number) {
  return (
    <>
      <circle cx={cx} cy={cy} r={r} fill={WHITE} />
      <circle className="avatar__pupil" cx={cx} cy={cy} r={r * FACE.pupil} fill={INK} />
      <circle
        cx={cx - r * 0.32}
        cy={cy - r * 0.36}
        r={r * FACE.specular}
        fill="#fff"
        opacity="0.95"
      />
    </>
  );
}

/** A closed, happy arc. Same weight as the mouth, so a face has one line. */
function lid(cx: number, cy: number, r: number) {
  return (
    <path
      d={`M${cx - r} ${cy + r * 0.34}q${r} ${-r * 1.4} ${r * 2} 0`}
      stroke={INK}
      strokeWidth={r * FACE.stroke}
      fill="none"
      strokeLinecap="round"
    />
  );
}

/** Cuts the top off an open eye. Only works because a face always sits on fill. */
function hood(cx: number, cy: number, r: number) {
  return <path d={`M${cx - r} ${cy - r * 0.1}a${r} ${r} 0 01${r * 2} 0z`} fill={FILL} />;
}

function drawEyes(face: Face): ReactNode {
  const { y, r } = face;
  const cx = face.cx ?? FORM.centerX;
  const left = cx - face.spread;
  const right = cx + face.spread;

  switch (face.eyes ?? "open") {
    case "shut":
      return (
        <>
          {lid(left, y, r)}
          {lid(right, y, r)}
        </>
      );
    case "wink":
      return (
        <>
          <g className="avatar__eye">{eyeball(left, y, r)}</g>
          {lid(right, y, r)}
        </>
      );
    case "half":
      return (
        <>
          <g className="avatar__eye">
            {eyeball(left, y, r)}
            {hood(left, y, r)}
          </g>
          <g className="avatar__eye">
            {eyeball(right, y, r)}
            {hood(right, y, r)}
          </g>
        </>
      );
    case "dot":
      return (
        <>
          <g className="avatar__eye">
            <circle cx={left} cy={y} r={r * 0.62} fill={INK} />
          </g>
          <g className="avatar__eye">
            <circle cx={right} cy={y} r={r * 0.62} fill={INK} />
          </g>
        </>
      );
    case "tall":
      return (
        <>
          <g className="avatar__eye">
            <ellipse cx={left} cy={y} rx={r * 0.84} ry={r * FACE.tallest} fill={WHITE} />
            <ellipse
              className="avatar__pupil"
              cx={left}
              cy={y}
              rx={r * 0.46}
              ry={r * 0.66}
              fill={INK}
            />
          </g>
          <g className="avatar__eye">
            <ellipse cx={right} cy={y} rx={r * 0.84} ry={r * FACE.tallest} fill={WHITE} />
            <ellipse
              className="avatar__pupil"
              cx={right}
              cy={y}
              rx={r * 0.46}
              ry={r * 0.66}
              fill={INK}
            />
          </g>
        </>
      );
    case "stern":
      return (
        <>
          <g className="avatar__eye">{eyeball(left, y, r)}</g>
          <g className="avatar__eye">{eyeball(right, y, r)}</g>
          <path
            d={`M${left - r * 1.1} ${y - r * 1.7}l${r * 2} ${r * 0.55}M${right + r * 1.1} ${y - r * 1.7}l${-r * 2} ${r * 0.55}`}
            stroke={INK}
            strokeWidth={r * 0.42}
            strokeLinecap="round"
          />
        </>
      );
    case "sparkle":
      return (
        <>
          <g className="avatar__eye">
            {eyeball(left, y, r)}
            <path
              d={`M${left + r * 0.6} ${y - r} l${r * 0.18} ${r * 0.38} ${r * 0.38} ${r * 0.18} ${-r * 0.38} ${r * 0.18} ${-r * 0.18} ${r * 0.38} ${-r * 0.18} ${-r * 0.38} ${-r * 0.38} ${-r * 0.18} ${r * 0.38} ${-r * 0.18}z`}
              fill="#fff"
            />
          </g>
          <g className="avatar__eye">{eyeball(right, y, r)}</g>
        </>
      );
    default:
      return (
        <>
          <g className="avatar__eye">{eyeball(left, y, r)}</g>
          <g className="avatar__eye">{eyeball(right, y, r)}</g>
        </>
      );
  }
}

function drawMouth(face: Face): ReactNode {
  const cx = face.cx ?? FORM.centerX;
  const y = face.y + face.r * FACE.drop;
  const w = face.r * FACE.width;
  const line = { stroke: INK, strokeWidth: face.r * FACE.stroke, fill: "none" } as const;

  switch (face.mouth) {
    case "grin":
      return (
        <path d={`M${cx - w} ${y - w * 0.3}h${w * 2}a${w} ${w} 0 01${-w * 2} 0z`} fill={INK} />
      );
    case "flat":
      return <path d={`M${cx - w * 0.8} ${y}h${w * 1.6}`} {...line} strokeLinecap="round" />;
    case "oh":
      return <ellipse cx={cx} cy={y} rx={w * 0.62} ry={w * 0.78} fill={INK} />;
    case "smirk":
      return (
        <path
          d={`M${cx - w} ${y}q${w * 0.9} ${w * 0.8} ${w * 1.7} ${-w * 0.5}`}
          {...line}
          strokeLinecap="round"
        />
      );
    case "cat":
      return (
        <path
          d={`M${cx - w} ${y - w * 0.35}q${w * 0.5} ${w * 0.62} ${w} 0q${w * 0.5} ${w * 0.62} ${w} 0`}
          {...line}
          strokeLinecap="round"
        />
      );
    case "tiny":
      return (
        <path
          d={`M${cx - w * 0.5} ${y}q${w * 0.5} ${w * 0.55} ${w} 0`}
          {...line}
          strokeLinecap="round"
        />
      );
    case "wobble":
      return (
        <path
          d={`M${cx - w} ${y}q${w * 0.5} ${-w * 0.62} ${w} 0q${w * 0.5} ${w * 0.62} ${w} 0`}
          {...line}
          strokeLinecap="round"
        />
      );
    default:
      return (
        <path d={`M${cx - w} ${y}q${w} ${w * 0.85} ${w * 2} 0`} {...line} strokeLinecap="round" />
      );
  }
}

// ---- bodies ---------------------------------------------------------------

interface Body {
  /** The silhouette, as one compound path so the rim runs around all of it. */
  d: string;
  /** The single light stroke. Always down the upper left. */
  sheen: string;
  /** Stems and leaves. Drawn under the silhouette so they tuck in behind it. */
  behind?: ReactNode;
  /** Seams, husks, kernels. Drawn on the body, under the face. */
  front?: ReactNode;
}

/** A stem, drawn the same way on everything that has one. */
function stem(from: number, to: number, width = 2.5) {
  return <path d={`M32 ${from}V${to}`} stroke={DEEP} strokeWidth={width} strokeLinecap="round" />;
}

/** Leaves, sprouts and husks. Filled in `shade`, rimmed like the body. */
function leaf(d: string) {
  return <path d={d} fill={SHADE} stroke={DEEP} strokeWidth="1.4" strokeLinejoin="round" />;
}

/** Seams and ribs. Never darker than the rim, never a second outline. */
function seam(d: string) {
  return (
    <path d={d} stroke={DEEP} strokeWidth="1.5" fill="none" opacity="0.32" strokeLinecap="round" />
  );
}

function drawBody(body: Body): ReactNode {
  return (
    <>
      {body.behind}
      <path
        d={body.d}
        fill={FILL}
        stroke={DEEP}
        strokeWidth={FORM.rim}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <path
        d={body.sheen}
        fill="none"
        stroke={LIGHT}
        strokeWidth={FORM.sheen}
        strokeLinecap="round"
        opacity="0.5"
      />
      {body.front}
    </>
  );
}

// ---- the cast -------------------------------------------------------------

export type CharacterGroup = "Recipe" | "Garden" | "Table";

export interface Character {
  key: string;
  label: string;
  group: CharacterGroup;
  body: Body;
  face: Face;
}

export const CHARACTERS: Character[] = [
  {
    key: "avocado",
    label: "Avocado",
    group: "Recipe",
    body: {
      d: "M32 13.4c-5.4 0-8.8 4.2-8.8 9 0 3.7.9 5.2-1.5 8.6-3.4 4.7-7.1 10-7.1 15.4 0 7.2 7.8 12.6 17.4 12.6s17.4-5.4 17.4-12.6c0-5.4-3.7-10.7-7.1-15.4-2.4-3.4-1.5-4.9-1.5-8.6 0-4.8-3.4-9-8.8-9z",
      sheen: "M23 28.6c-3.3 5-4.7 9.8-4.5 14.6",
      behind: stem(14, 6.8),
    },
    face: { y: 38.5, spread: 6.2, r: 4.4, mouth: "smile" },
  },
  {
    key: "lime",
    label: "Lime",
    group: "Recipe",
    body: {
      d: "M32 16.2C41.6 16.2 49.4 25.6 49.4 37.4S41.6 58.6 32 58.6 14.6 49.2 14.6 37.4 22.4 16.2 32 16.2z",
      sheen: "M21.2 26.8c-3.1 3.8-4.6 7.7-4.6 11.8",
      behind: (
        <>
          {stem(16.6, 9.6, 2.2)}
          {leaf("M33 11.4c4.6-3.4 9.4-2.6 9.4-2.6S39.2 14.4 33 11.4z")}
        </>
      ),
    },
    face: { y: 34.4, spread: 6.4, r: 4.6, mouth: "grin" },
  },
  {
    key: "tomato",
    label: "Tomato",
    group: "Recipe",
    body: {
      d: "M32 18.8c11.2 0 19.4 6.4 19.4 18.2 0 12.8-8.6 21.6-19.4 21.6S12.6 49.8 12.6 37c0-11.8 8.2-18.2 19.4-18.2z",
      sheen: "M20.8 27.6c-3.2 3.4-4.6 6.8-4.6 10.2",
      front: (
        <>
          {stem(19.4, 10.8, 2.4)}
          {leaf(
            "M32 19.8 20.6 14.2l3.3 8.1zM32 19.8 43.4 14.2l-3.3 8.1zM32 19.8l-7.3-7.3-.4 8.5zM32 19.8l7.3-7.3.4 8.5z",
          )}
        </>
      ),
    },
    face: { y: 37.2, spread: 6.6, r: 4.6, mouth: "smile" },
  },
  {
    key: "onion",
    label: "Onion",
    group: "Recipe",
    body: {
      d: "M32 14.6c-1.6 6.2-6.8 8.8-10.8 13.4C17.2 32.6 14.6 38.2 14.6 43.6c0 8.8 7.8 15.2 17.4 15.2s17.4-6.4 17.4-15.2c0-5.4-2.6-11-6.6-15.6-4-4.6-9.2-7.2-10.8-13.4z",
      sheen: "M22.6 30.6c-3.2 4.4-4.6 8.6-4.4 12.6",
      behind: (
        <>
          {stem(15, 6.6, 2.2)}
          {leaf(
            "M32 15c-1.6-4.8-6-7.8-6-7.8s.4 6.2 3.6 8.6zM32 15c1.4-5.4 6.6-8.2 6.6-8.2s-.6 6.8-4 9z",
          )}
        </>
      ),
      front: seam("M23.6 30.4c-2.6 5.6-3 12-1.2 17.4M40.4 30.4c2.6 5.6 3 12 1.2 17.4"),
    },
    face: { y: 36.6, spread: 6.2, r: 4.4, mouth: "wobble" },
  },
  {
    key: "garlic",
    label: "Garlic",
    group: "Recipe",
    body: {
      d: "M32 13c-1 6.8-6.2 10-10 15C18.2 32.8 15.8 38.6 15.8 43.8c0 8.6 7.2 15 16.2 15s16.2-6.4 16.2-15c0-5.2-2.4-11-6.2-15.8-3.8-5-9-8.2-10-15z",
      sheen: "M23.8 31.2c-3 4.4-4.4 8.6-4.2 12.6",
      behind: stem(13.4, 6.6, 2.4),
      front: seam("M24.8 31c-2.4 5.8-2.6 12.8-.6 18.4M39.2 31c2.4 5.8 2.6 12.8.6 18.4"),
    },
    face: { y: 37.8, spread: 6, r: 4.4, eyes: "sparkle", mouth: "tiny" },
  },
  {
    key: "chilli",
    label: "Chilli",
    group: "Recipe",
    body: {
      d: "M26.8 17.8c8.4-1.6 16.8 4.4 19 13.6 2.2 9.2-2.2 19.2-9.9 24.8-5.7 4.2-12.8 3.3-16.1-1.8-3.1-4.8-1.5-10.8 2.9-14.3 5.3-4.2 7.3-12.1 4.1-22.3z",
      sheen: "M25.6 25.4c-1.5 5.2-1.7 9.4-.6 12.9",
      behind: leaf("M27 18c-4.4-5-10.2-5.4-10.2-5.4s1 5.8 6.4 7.9z"),
    },
    face: { y: 34.6, spread: 5.6, r: 4.1, cx: 33, mouth: "smirk" },
  },
  {
    key: "cilantro",
    label: "Cilantro",
    group: "Recipe",
    body: {
      d: "M32 19.4c3.9-3 9.4-1.4 10.6 3 4.1-.9 7.6 2.7 6.5 6.7 3.5 2.1 3.4 7.6-.5 9.5 1.4 4.1-2.1 8.1-6.4 7.4-1.2 4.2-6.7 5.7-10.2 2.7-3.5 3-9 1.5-10.2-2.7-4.3.7-7.8-3.3-6.4-7.4-3.9-1.9-4-7.4-.5-9.5-1.1-4 2.4-7.6 6.5-6.7 1.2-4.4 6.7-6 10.6-3z M30.2 45.6h3.6v11.8h-3.6z",
      sheen: "M22 28.4c-1.4 2.6-1.6 5.2-.5 7.4",
    },
    face: { y: 31.4, spread: 6.2, r: 4.4, eyes: "shut", mouth: "smile" },
  },
  {
    key: "salt",
    label: "Salt",
    group: "Recipe",
    body: {
      d: "M32 15.2c-7 0-11.8 2.7-11.8 6.6 0 4 .9 6 .9 9.6 0 4-2.2 8.8-2.2 14.4 0 7.1 5.3 12.4 13.1 12.4s13.1-5.3 13.1-12.4c0-5.6-2.2-10.4-2.2-14.4 0-3.6.9-5.6.9-9.6 0-3.9-4.8-6.6-11.8-6.6z",
      sheen: "M22.8 31.2c-1.7 5.4-2.1 10.2-1.3 14",
      front: (
        <>
          {seam("M20.6 23.4h22.8")}
          <g fill={DEEP} opacity="0.45">
            <circle cx="28" cy="19.4" r="1.1" />
            <circle cx="32" cy="18.4" r="1.1" />
            <circle cx="36" cy="19.4" r="1.1" />
          </g>
        </>
      ),
    },
    face: { y: 38.6, spread: 6.2, r: 4.4, mouth: "smile" },
  },

  {
    key: "corn",
    label: "Corn",
    group: "Garden",
    body: {
      d: "M32 14.2c8.1 0 14 6.8 14 16.6 0 12.6-6.3 27.8-14 27.8s-14-15.2-14-27.8c0-9.8 5.9-16.6 14-16.6z",
      sheen: "M24.6 25c-2.4 5.2-3 10.2-2.2 14.2",
      behind: leaf(
        "M20.4 31c-4.8 2.2-7.4 8.2-7.4 8.2s5.8.9 9-3zM43.6 31c4.8 2.2 7.4 8.2 7.4 8.2s-5.8.9-9-3z",
      ),
      front: (
        <g fill={DEEP} opacity="0.2">
          <circle cx="27" cy="46" r="1.5" />
          <circle cx="32" cy="47.4" r="1.5" />
          <circle cx="37" cy="46" r="1.5" />
          <circle cx="29.4" cy="51.4" r="1.4" />
          <circle cx="34.6" cy="51.4" r="1.4" />
          <circle cx="32" cy="42" r="1.4" />
        </g>
      ),
    },
    face: { y: 32.6, spread: 5.6, r: 4.2, mouth: "grin" },
  },
  {
    key: "pepper",
    label: "Pepper",
    group: "Garden",
    body: {
      d: "M32 19c4.4-2.4 10.4-2 14 1.6 4.2 4.2 4.6 12 2.6 19.6-2 7.6-6.6 13.4-11.6 13.4-2.2 0-3.6-.8-5-.8s-2.8.8-5 .8c-5 0-9.6-5.8-11.6-13.4-2-7.6-1.6-15.4 2.6-19.6 3.6-3.6 9.6-4 14-1.6z",
      sheen: "M22.4 27.2c-2.4 5.2-2.8 10.6-1.6 15",
      behind: (
        <>
          {stem(19.4, 9.6, 2.8)}
          {leaf("M25 18.4c2.4-3.4 9.6-3.4 14 0-4 2.6-10 2.6-14 0z")}
        </>
      ),
      front: seam("M25.6 28.6c-1.2 6.8-.6 13.6 1.4 18.4M38.4 28.6c1.2 6.8.6 13.6-1.4 18.4"),
    },
    face: { y: 34.6, spread: 6.4, r: 4.6, eyes: "wink", mouth: "smirk" },
  },
  {
    key: "radish",
    label: "Radish",
    group: "Garden",
    body: {
      d: "M32 17.4c8.8 0 15.4 6.4 15.4 14.6 0 6.8-4.2 11.8-8.4 16.4-3 3.2-5.2 7-6 9-.4 1-1.6 1-2 0-.8-2-3-5.8-6-9-4.2-4.6-8.4-9.6-8.4-16.4 0-8.2 6.6-14.6 15.4-14.6z",
      sheen: "M23 25.6c-2.6 3.4-3.8 6.8-3.6 10.2",
      behind: leaf(
        "M32 17.8c-2.6-6.4-1.4-11.8-1.4-11.8s4.2 5 3.4 11.8zM32 18c4-5.6 10.2-8.6 10.2-8.6s-2.4 8.4-7.6 10.4zM32 18.2c-5-4-11.6-6.2-11.6-6.2s3.4 7.6 9.2 8.4z",
      ),
    },
    face: { y: 31.2, spread: 6, r: 4.4, mouth: "smile" },
  },
  {
    key: "carrot",
    label: "Carrot",
    group: "Garden",
    body: {
      d: "M32 18.6c7.5 0 12.5 4.3 12.5 9.2 0 3.6-1.8 7.8-4 12.8-2.4 5.4-4.8 10.6-5.6 14.2-.4 1.9-5.4 1.9-5.8 0-.8-3.6-3.2-8.8-5.6-14.2-2.2-5-4-9.2-4-12.8 0-4.9 5-9.2 12.5-9.2z",
      sheen: "M25.8 27.4c-1.2 4.8-1 9 .3 12.6",
      behind: leaf(
        "M32 18.8c-2.8-7-1.4-13.6-1.4-13.6s4.4 6.2 3.4 13.6zM32 19c4.4-5.8 10.6-9.2 10.6-9.2s-2.4 8.8-7.8 10.6zM32 19.2c-5.2-4-11.8-6.4-11.8-6.4s3.4 7.8 9.2 8.6z",
      ),
    },
    face: { y: 31.6, spread: 5.6, r: 4.1, mouth: "smile" },
  },
  {
    key: "mushroom",
    label: "Mushroom",
    group: "Garden",
    body: {
      d: "M32 15.4c11.4 0 19.4 8.4 19.4 16.8 0 3.4-2.6 5.2-6 5.2H18.6c-3.4 0-6-1.8-6-5.2 0-8.4 8-16.8 19.4-16.8z M25.6 37.4h12.8c0 6 1.4 12.2 1.4 16.2 0 3.6-3.2 5.2-7.8 5.2s-7.8-1.6-7.8-5.2c0-4 1.4-10.2 1.4-16.2z",
      sheen: "M21.6 24.6c-2.6 2.6-4 5.2-4.2 8",
      front: (
        <g fill={LIGHT} opacity="0.45">
          <circle cx="21.6" cy="30.6" r="2.2" />
          <circle cx="42.4" cy="30.6" r="1.9" />
          <circle cx="32" cy="19.4" r="1.7" />
        </g>
      ),
    },
    face: { y: 27.8, spread: 6, r: 4.2, mouth: "cat" },
  },
  {
    key: "squash",
    label: "Squash",
    group: "Garden",
    body: {
      d: "M29.2 34.8c-1.8-6.2-2.2-13.4-.6-18.4 1-3.2 5.8-3.4 6.8 0 1.4 4.8.8 11.6-.6 18.4 5.6 2.6 9.4 7.2 9.4 12.4 0 6.6-5.4 11.4-12.2 11.4s-12.2-4.8-12.2-11.4c0-5.2 3.8-9.8 9.4-12.4z",
      sheen: "M25.4 40.4c-2.4 3.4-3.2 6.8-2.6 10",
      behind: (
        <>
          {stem(15.4, 8.6, 2.2)}
          {leaf("M32.6 9.6c4.2-3.2 8.8-2.4 8.8-2.4s-3 5.2-8.8 2.4z")}
        </>
      ),
      front: seam("M28.6 21.4c-.8 4.4-.8 9 0 12.6"),
    },
    face: { y: 43.6, spread: 6, r: 4.3, mouth: "smile" },
  },
  {
    key: "eggplant",
    label: "Eggplant",
    group: "Garden",
    body: {
      d: "M32 11.8c4.2 0 7.2 2.2 7.2 5.6 0 2.5-.9 4.1-.9 6.5 0 3.3 2.3 5.5 3.8 8.6 1.3 2.7 2.3 5.6 2.3 8.8 0 8.9-5.6 15.4-12.4 15.4s-12.4-6.5-12.4-15.4c0-3.2 1-6.1 2.3-8.8 1.5-3.1 3.8-5.3 3.8-8.6 0-2.4-.9-4-.9-6.5 0-3.4 3-5.6 7.2-5.6z",
      sheen: "M23.8 36.4c-2.2 3.8-3 7.6-2.6 11",
      front: (
        <>
          {stem(13.4, 7.8, 2.6)}
          {leaf(
            "M32 15.6 23.8 15.2l2.6 6.4zM32 15.6l8.2-.4-2.6 6.4zM32 15.6l-4-4.2-2.4 6zM32 15.6l4-4.2 2.4 6z",
          )}
        </>
      ),
    },
    face: { y: 40, spread: 5.6, r: 4.2, eyes: "half", mouth: "smirk" },
  },

  {
    key: "chip",
    label: "Chip",
    group: "Table",
    body: {
      d: "M32 16.6c1.6 0 3 .8 3.8 2.2l15 27.6c1.8 3.4-.6 7.4-4.4 7.4H17.6c-3.8 0-6.2-4-4.4-7.4l15-27.6c.8-1.4 2.2-2.2 3.8-2.2z",
      sheen: "M22.8 44.6l5.8-10.8",
      front: (
        <g fill={LIGHT} opacity="0.5">
          <circle cx="40.6" cy="45.8" r="1.4" />
          <circle cx="23.6" cy="48.2" r="1.2" />
          <circle cx="34.4" cy="50" r="1.1" />
        </g>
      ),
    },
    face: { y: 40, spread: 5.4, r: 4.2, mouth: "grin" },
  },
  {
    key: "pit",
    label: "Pit",
    group: "Table",
    body: {
      d: "M32 14.8c9.8 0 17.2 9 17.2 21.2 0 13.4-7.4 22.6-17.2 22.6S14.8 49.4 14.8 36c0-12.2 7.4-21.2 17.2-21.2z",
      sheen: "M22.4 26.4c-2.8 4.2-4 8.4-3.8 12.4",
    },
    face: { y: 36.4, spread: 6.2, r: 4.4, eyes: "tall", mouth: "flat" },
  },
  {
    key: "mill",
    label: "Mill",
    group: "Table",
    body: {
      d: "M32 13.2c4.9 0 8.5 1.9 8.5 4.4 0 2.1-1.5 3.6-1.5 5.2 0 1.7 2 2.7 2 4.6s-2.3 3.1-2.3 4.8c0 2.6 5.6 8.4 5.6 15.6 0 6.3-5.2 11-12.3 11s-12.3-4.7-12.3-11c0-7.2 5.6-13 5.6-15.6 0-1.7-2.3-2.9-2.3-4.8s2-2.9 2-4.6c0-1.6-1.5-3.1-1.5-5.2 0-2.5 3.6-4.4 8.5-4.4z",
      sheen: "M24.6 36.6c-1.8 4.6-2.4 8.8-2 12.2",
      front: seam("M24.4 32.4h15.2"),
    },
    face: { y: 43, spread: 5.6, r: 4.1, eyes: "stern", mouth: "flat" },
  },
  {
    key: "molcajete",
    label: "Molcajete",
    group: "Table",
    body: {
      d: "M13.6 21.4c0-1.8 1.6-3 3.4-3h30c1.8 0 3.4 1.2 3.4 3 0 1.9-1.4 3.1-3.3 3.5-.8 6.6-2.6 11.6-5.4 14.8-2.5 2.9-5.8 4.4-9.7 4.4s-7.2-1.5-9.7-4.4c-2.8-3.2-4.6-8.2-5.4-14.8-1.9-.4-3.3-1.6-3.3-3.5z M24.2 41.6l-1.6 10.6c-.3 1.8-2 3-3.8 2.7s-3-2-2.7-3.8l1.6-10.6z M39.8 41.6l1.6 10.6c.3 1.8 2 3 3.8 2.7s3-2 2.7-3.8l-1.6-10.6z M28.6 43.4h6.8v8.6c0 1.9-1.5 3.4-3.4 3.4s-3.4-1.5-3.4-3.4z",
      sheen: "M18.6 26.4c1 6 2.6 10.6 4.8 13.6",
      front: seam("M17.4 24.8h29.2"),
    },
    face: { y: 29, spread: 6.4, r: 4.4, mouth: "grin" },
  },
  {
    key: "jar",
    label: "Jar",
    group: "Table",
    body: {
      d: "M19.8 25.4c0-2.6 2.1-4.6 4.6-4.6h15.2c2.5 0 4.6 2 4.6 4.6v26c0 2.6-2.1 4.6-4.6 4.6H24.4c-2.5 0-4.6-2-4.6-4.6z M21.8 13.4c0-1.4 1.1-2.4 2.5-2.4h15.4c1.4 0 2.5 1 2.5 2.4v5.2c0 1.4-1.1 2.4-2.5 2.4H24.3c-1.4 0-2.5-1-2.5-2.4z",
      sheen: "M23.6 29.4v18.6",
      front: seam("M20.2 26.8h23.6"),
    },
    face: { y: 36, spread: 6, r: 4.4, eyes: "tall", mouth: "flat" },
  },
  {
    key: "spoon",
    label: "Spoon",
    group: "Table",
    body: {
      d: "M28 11.6c5.3 0 9.6 4.7 9.6 10.6 0 3.6-1.6 6.8-4 8.6 3.2 3.6 9 12.8 11.6 17.8 1.4 2.7.4 5.6-2.2 6.8s-5.4 0-6.6-2.6c-2.4-5.2-7-13.4-9.8-17-4.8-1.2-8.4-6.6-8.4-13.4 0-6 4.3-10.8 9.6-10.8z",
      sheen: "M21.6 19.4c-1.3 3.2-1.5 6.2-.5 8.8",
      front: seam("M34.6 37.4c2.4 4 5 8.8 6.6 12.4"),
    },
    face: { y: 21, spread: 5.4, r: 4.1, cx: 28, mouth: "grin" },
  },
];

export const CHARACTER_GROUPS: CharacterGroup[] = ["Recipe", "Garden", "Table"];

const BY_KEY = new Map(CHARACTERS.map((c) => [c.key, c]));

export const DEFAULT_CHARACTER = "avocado";

/**
 * Keys from earlier sets, so an existing agent keeps a sensible character.
 *
 * Most of the original produce keys are real ingredients again, so they resolve
 * without an entry here. Only the sets that drifted away from food need one.
 */
const ALIASES: Record<string, string> = {
  // egg set: one body, told apart by face and prop
  plain: "avocado",
  cheerful: "tomato",
  curious: "lime",
  wink: "pepper",
  sleepy: "pit",
  stern: "mill",
  bright: "corn",
  blank: "onion",
  cat: "mushroom",
  tophat: "mill",
  cap: "chip",
  crown: "pepper",
  bowtie: "chip",
  necktie: "mill",
  scarf: "cilantro",
  glasses: "mushroom",
  monocle: "pit",
  headphones: "salt",
  antenna: "carrot",
  sprout: "cilantro",
  // hand-drawn creature set
  bean: "pit",
  fox: "carrot",
  owl: "mushroom",
  crab: "tomato",
  bird: "corn",
  bug: "radish",
  slime: "onion",
  bot: "salt",
  gear: "mill",
  ghost: "garlic",
  moon: "lime",
  star: "corn",
  cloud: "cilantro",
  // original emoji set
  robot: "salt",
  brain: "mushroom",
  penguin: "pit",
  butterfly: "cilantro",
  bee: "corn",
  rocket: "carrot",
  sun: "tomato",
  taco: "chip",
  octopus: "mushroom",
  frog: "pepper",
  snail: "onion",
  comet: "chip",
  fire: "chilli",
  bolt: "chip",
  satellite: "mill",
};

function hashToIndex(key: string): number {
  let hash = 0;
  for (let i = 0; i < key.length; i++) hash = (hash * 31 + key.charCodeAt(i)) >>> 0;
  return hash % CHARACTERS.length;
}

/**
 * Never returns undefined. An agent saved by any past or future build still has
 * to render today, so an unknown key gets a stable stand-in.
 */
export function lookupCharacter(key: string): Character {
  const direct = BY_KEY.get(key);
  if (direct) return direct;

  const aliased = ALIASES[key];
  if (aliased) {
    const found = BY_KEY.get(aliased);
    if (found) return found;
  }
  return CHARACTERS[hashToIndex(key)] ?? BY_KEY.get(DEFAULT_CHARACTER)!;
}

/** The whole drawing, in one order: what tucks behind, the body, then the face. */
export function drawCharacter(character: Character): ReactNode {
  return (
    <>
      {drawBody(character.body)}
      {drawEyes(character.face)}
      {drawMouth(character.face)}
    </>
  );
}

/**
 * Accent colors. Drawn from the same palette as the app chrome so a random
 * pick never clashes with the surface behind it.
 */
export const ACCENTS: { name: string; value: string }[] = [
  { name: "Flesh", value: "#c7d96b" },
  { name: "Rind", value: "#6faa5c" },
  { name: "Pit", value: "#b0784a" },
  { name: "Mint", value: "#7fd1a3" },
  { name: "Chilli", value: "#e2674a" },
  { name: "Tomato", value: "#d9534f" },
  { name: "Corn", value: "#e8b84b" },
  { name: "Sky", value: "#6aa9d9" },
  { name: "Iris", value: "#9b8ad4" },
  { name: "Rose", value: "#d97ea8" },
  { name: "Clay", value: "#c2926b" },
  { name: "Slate", value: "#8aa0a6" },
];

export const DEFAULT_ACCENT = ACCENTS[0]!.value;

/** Picks the accent least used so far, so a new crew is visually separable. */
export function suggestAccent(taken: string[]): string {
  const counts = new Map(ACCENTS.map((a) => [a.value, 0]));
  for (const color of taken) {
    const key = color.toLowerCase();
    if (counts.has(key)) counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  let best = DEFAULT_ACCENT;
  let lowest = Number.POSITIVE_INFINITY;
  for (const [value, count] of counts) {
    if (count < lowest) {
      lowest = count;
      best = value;
    }
  }
  return best;
}

/** Picks an unused character so two new agents do not look identical. */
export function suggestCharacter(taken: string[]): string {
  const used = new Set(taken);
  return (CHARACTERS.find((c) => !used.has(c.key)) ?? CHARACTERS[0]!).key;
}
