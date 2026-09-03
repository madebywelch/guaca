/**
 * A page for reviewing a pass on the characters: what shipped beside what is
 * proposed, drawn live by both versions of the drawing code.
 *
 *   ./scripts/make-caricature-review.sh
 *
 * `before/*` is `src/avatars` as it was at HEAD, snapshotted by the script and
 * bundled under that alias; `../src/avatars` is the working tree. Both are the
 * real modules, so nothing on the page is a sketch of the app: it is the app's
 * geometry, twice. The one thing ported rather than imported is the frame loop
 * from `AgentAvatar.tsx`, because that is a React component and this page has
 * no React, and because the two versions differ in it (the smoother moved into
 * `eyes.ts` and the eyes read it too, so they and the body move together).
 */

import * as beforeCatalog from "before/catalog";
import * as beforeEyes from "before/eyes";
import * as beforeForm from "before/form";
import * as beforeMoods from "before/moods";
import * as afterCatalog from "../src/avatars/catalog";
import { gaitOf } from "../src/avatars/clock";
import * as afterEyes from "../src/avatars/eyes";
import * as afterForm from "../src/avatars/form";
import * as afterMoods from "../src/avatars/moods";

type Point = [number, number];
type Mood = afterMoods.Mood;
type Eye = afterEyes.Eye;
type Look = "up" | "down" | null;
type Gesture = "send" | "receive" | null;

interface Mass {
  gaze: Point;
  vel: Point;
}

/** Everything a painter needs from one version of the drawing code. */
interface Kit {
  name: "before" | "after";
  FORM: typeof afterForm.FORM;
  MOODS: Record<Mood, afterMoods.Expression>;
  CHARACTERS: afterCatalog.Character[];
  bodyPoints: typeof afterForm.bodyPoints;
  blend: typeof afterForm.blend;
  outline: typeof afterForm.outline;
  eyesAt: typeof afterEyes.eyesAt;
  eyePath: typeof afterEyes.eyePath;
  gazeAt: typeof afterEyes.gazeAt;
  blendEyes: typeof afterEyes.blendEyes;
  aimedEye: typeof afterEyes.aimedEye;
  AIM: { up: number; down: number };
  markFor: typeof afterMoods.markFor;
  settle: (mass: Mass, target: Point, dt: number) => void;
  grip: (reach: number) => number;
  /** Whether the eyes read the smoothed look too, or only the body does. */
  together: boolean;
}

/* What `AgentAvatar.tsx` did before the spring moved into `eyes.ts`. */
const OLD_SPRING = 2.2 / beforeEyes.SETTLE;
function oldSettle(mass: Mass, target: Point, dt: number) {
  for (const i of [0, 1] as const) {
    const x = mass.gaze[i];
    const v = mass.vel[i];
    const accel = OLD_SPRING * OLD_SPRING * (target[i] - x) - 2 * OLD_SPRING * v;
    mass.vel[i] = v + accel * dt;
    mass.gaze[i] = x + mass.vel[i] * dt;
  }
}

const BEFORE: Kit = {
  name: "before",
  FORM: beforeForm.FORM,
  MOODS: beforeMoods.MOODS as Kit["MOODS"],
  CHARACTERS: beforeCatalog.CHARACTERS as Kit["CHARACTERS"],
  bodyPoints: beforeForm.bodyPoints as Kit["bodyPoints"],
  blend: beforeForm.blend,
  outline: beforeForm.outline,
  eyesAt: beforeEyes.eyesAt as Kit["eyesAt"],
  eyePath: beforeEyes.eyePath,
  gazeAt: beforeEyes.gazeAt as Kit["gazeAt"],
  blendEyes: beforeEyes.blendEyes as Kit["blendEyes"],
  aimedEye: beforeEyes.aimedEye as Kit["aimedEye"],
  AIM: beforeEyes.AIM,
  markFor: beforeMoods.markFor as Kit["markFor"],
  settle: oldSettle,
  grip: (r) => r,
  together: false,
};

const AFTER: Kit = {
  name: "after",
  FORM: afterForm.FORM,
  MOODS: afterMoods.MOODS,
  CHARACTERS: afterCatalog.CHARACTERS,
  bodyPoints: afterForm.bodyPoints,
  blend: afterForm.blend,
  outline: afterForm.outline,
  eyesAt: afterEyes.eyesAt,
  eyePath: afterEyes.eyePath,
  gazeAt: afterEyes.gazeAt,
  blendEyes: afterEyes.blendEyes,
  aimedEye: afterEyes.aimedEye,
  AIM: afterEyes.AIM,
  markFor: afterMoods.markFor,
  settle: afterEyes.settle,
  grip: afterForm.grip,
  together: true,
};

const KITS = [BEFORE, AFTER];

/* Ported from `AgentAvatar.tsx`, unchanged there. */
const MORPH = 0.6;
const KNOCK = {
  send: { amp: 0.26, hz: 1.5, decay: 0.28, life: 0.9 },
  receive: { amp: 0.3, hz: 2.2, decay: 0.3, life: 0.8 },
};

const ACCENT: Record<string, string> = {
  orb: "#8b8f45",
  husk: "#4f5f96",
  crumb: "#bf5f3c",
  wave: "#5e8158",
  pip: "#a8453a",
  slab: "#5a7d99",
  gourd: "#b3805c",
  pebble: "#6c6f70",
  tide: "#4d8b83",
  cell: "#7c5a8c",
  knot: "#c28c31",
  mote: "#b26f86",
};

const MOOD_KEYS = Object.keys(AFTER.MOODS) as Mood[];

const MOOD_NOTES: Record<Mood, string> = {
  idle: "Active with nothing in flight. Now: the body is still. No breath, no wobble. The eyes blink, look wider, and one look in three goes the whole way to a side, which is the only thing that moves the body, and then it moves it into a pear.",
  listening: "A message queued. Now: the dots lifted off their line, which is a pair of brows up.",
  thinking: "A turn between rounds. Now: one brow up and the other eye in a squint. The pair was a mirror and read as mild.",
  working: "A tool call in flight. Now: the dash tilted in a little further, scanning a little faster.",
  frustrated:
    "The last call back was refused or failed. Now: the eye has become the brow. Raised, thinned, tilted in until the stroke is a scowl, and it glares off to one side now and then, which drags the body after it.",
  blocked:
    "A turn parked on a person. Now: the squint at the badge cocks one brow as it goes, so it is looking at the badge and doubting it.",
  pleased: "Its reply landed in the last few seconds. Unchanged but for sitting a shade higher.",
  paused: "Lifecycle paused, or composted. Unchanged.",
  stuck:
    "An escalation of its own is open. Now: worried rather than blank. Inner ends up, one higher than the other, eyes darting where the body cannot go. This is the one semantic change: the old face read as sleepy beside paused.",
  surprised:
    "It has just been handed a message. Unchanged: the widest eyes on the table have a third of a unit of room over them when it looks up at whoever threw it, and a raise did not fit.",
};

/* --- the page's own clock ------------------------------------------------ */

let clock = 0;
let speed = 1;
let paused = false;
const painters: Array<(seconds: number) => void> = [];

let lastFrame = performance.now();
function frame(now: number) {
  requestAnimationFrame(frame);
  const dt = (now - lastFrame) / 1000;
  lastFrame = now;
  if (!paused) clock += Math.min(dt, 0.1) * speed;
  for (const paint of painters) paint(clock);
}

/* --- one creature ---------------------------------------------------------- */

const SVG = "http://www.w3.org/2000/svg";

class Creature {
  el: HTMLSpanElement;
  skin: SVGPathElement;
  eyes: SVGGElement;
  mark: SVGGElement;
  kit: Kit;
  lump: afterCatalog.Character;
  gait = { phase: 0, rate: 1 };

  want: Mood;
  mood: Mood;
  from: Mood;
  at = -MORPH;
  mass: Mass = { gaze: [0, 0], vel: [0, 0] };
  last = 0;
  look: Look = null;
  gesture: Gesture = null;
  gestureAt = 0;
  marked: Mood | null = null;
  /** A gaze the eyes hold instead of the mood's own. For the scrubber. */
  hold: Point | null = null;
  /** An eye that replaces the mood's, held still. For the vocabulary sheet. */
  pose: Eye | null = null;

  constructor(kit: Kit, key: string, mood: Mood, size: string, seed?: string) {
    this.kit = kit;
    const lump = kit.CHARACTERS.find((c) => c.key === key);
    if (!lump) throw new Error(`no character ${key}`);
    this.lump = lump;
    this.gait = gaitOf(seed ?? key);
    this.want = mood;
    this.mood = mood;
    this.from = mood;

    this.el = document.createElement("span");
    this.el.className = `avatar avatar--${size}`;
    this.el.style.setProperty("--accent", ACCENT[key] ?? "#8b8f45");
    this.el.style.setProperty("--gait", `${this.gait.phase.toFixed(2)}s`);
    this.el.title = `${lump.label} · ${kit.name}`;
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("viewBox", `0 0 ${kit.FORM.box} ${kit.FORM.box}`);
    svg.setAttribute("class", "avatar__body");
    this.skin = document.createElementNS(SVG, "path");
    this.skin.setAttribute("fill", "var(--accent)");
    this.eyes = document.createElementNS(SVG, "g");
    this.eyes.setAttribute("fill", "none");
    this.eyes.setAttribute("stroke", "var(--eye)");
    this.eyes.setAttribute("stroke-linecap", "round");
    this.eyes.append(document.createElementNS(SVG, "path"), document.createElementNS(SVG, "path"));
    this.mark = document.createElementNS(SVG, "g");
    svg.append(this.skin, this.eyes, this.mark);
    this.el.append(svg);
  }

  throw_(gesture: Gesture) {
    this.gesture = gesture;
    this.gestureAt = clock;
  }

  paint(seconds: number, live = true) {
    const kit = this.kit;
    const t = seconds * this.gait.rate + this.gait.phase;

    if (this.want !== this.mood) {
      this.from = this.mood;
      this.mood = this.want;
      this.at = seconds;
    }
    if (this.marked !== this.mood) {
      this.marked = this.mood;
      this.mark.innerHTML = kit.markFor(this.mood);
      this.el.setAttribute("data-mood", this.mood);
    }

    const to = kit.MOODS[this.mood];
    const from = kit.MOODS[this.from];
    const raw = live ? Math.min(1, (seconds - this.at) / MORPH) : 1;
    const u = raw * raw * (3 - 2 * raw);

    let eyeGaze: Point;
    if (this.look) {
      eyeGaze = [0, this.look === "up" ? -kit.AIM.up : kit.AIM.down];
    } else if (this.hold) {
      eyeGaze = [this.hold[0], this.hold[1]];
    } else if (!live) {
      eyeGaze = [0, 0];
    } else {
      const a = kit.gazeAt(t, from.watch?.gaze);
      const b = kit.gazeAt(t, to.watch?.gaze);
      eyeGaze = [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
    }

    const dt = live ? Math.min(0.05, Math.max(0, seconds - this.last)) : 0;
    this.last = seconds;
    if (live) kit.settle(this.mass, eyeGaze, dt);
    else this.mass = { gaze: [eyeGaze[0], eyeGaze[1]], vel: [0, 0] };
    if (kit.together) eyeGaze = [this.mass.gaze[0], this.mass.gaze[1]];
    const bodyGaze: Point = [this.mass.gaze[0], this.mass.gaze[1]];

    if (this.gesture) {
      const knock = KNOCK[this.gesture];
      const age = seconds - this.gestureAt;
      if (age > knock.life) {
        this.gesture = null;
      } else if (live) {
        const wave =
          this.gesture === "send"
            ? Math.sin(age * knock.hz * Math.PI * 2)
            : Math.cos(age * knock.hz * Math.PI * 2);
        const away = this.look === "down" ? -1 : 1;
        const push = away * knock.amp * wave * Math.exp(-age / knock.decay);
        bodyGaze[1] += push;
        eyeGaze[1] += push * 0.35;
      }
    }

    const shape = kit.bodyPoints(this.lump, to.shape, t, bodyGaze);
    const pts =
      u >= 1
        ? shape.pts
        : kit.blend(kit.bodyPoints(this.lump, from.shape, t, bodyGaze).pts, shape.pts, u);
    this.skin.setAttribute("d", kit.outline(pts));

    const blended = this.pose ?? (u >= 1 ? to.eye : kit.blendEyes(from.eye, to.eye, u));
    const eye = this.look ? kit.aimedEye(blended, this.look) : blended;
    const watch = this.pose ? { blink: false as const } : u > 0.5 ? to.watch : from.watch;
    const drawn = kit.eyesAt(this.lump, eye, watch, t, live, eyeGaze);
    const paths = this.eyes.children;
    for (let i = 0; i < paths.length; i++) {
      const path = paths[i] as SVGPathElement;
      const one = drawn[i];
      if (!one) {
        path.setAttribute("d", "");
        continue;
      }
      path.setAttribute("d", kit.eyePath(one));
      path.setAttribute("stroke-width", one.h.toFixed(2));
    }
  }
}

function live(c: Creature): Creature {
  painters.push((s) => c.paint(s));
  return c;
}

/* --- page furniture --------------------------------------------------------- */

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") el.className = v;
    else el.setAttribute(k, v);
  }
  el.append(...children);
  return el;
}

function section(title: string, ...lede: string[]): HTMLElement {
  const id = title.toLowerCase().replace(/[^a-z]+/g, "-").replace(/^-|-$/g, "");
  const s = h("section", { id }, h("h2", {}, title));
  for (const p of lede) {
    const el = h("p", { class: "lede" });
    el.innerHTML = p;
    s.append(el);
  }
  return s;
}

function labelled(kit: Kit, node: Node): HTMLElement {
  const after = kit.name === "after";
  return h(
    "div",
    { class: after ? "labelled labelled--after" : "labelled" },
    h("span", { class: "tag" }, after ? "Proposed" : "Shipped"),
    node,
  );
}

function pair(build: (kit: Kit) => Node): HTMLElement {
  return h("div", { class: "pair" }, labelled(BEFORE, build(BEFORE)), labelled(AFTER, build(AFTER)));
}

/* --- sections --------------------------------------------------------------- */

const RAIL = ["orb", "husk", "wave", "pip", "slab", "tide", "cell", "crumb"];

function idleRail(): HTMLElement {
  const s = section(
    "Idle. Eight of them.",
    "The complaint was that an idle rail would not sit still. The idle body is now still: no breath, no wobble, nothing but the eyes. And the look no longer moves the body at all under a glance: <code>grip</code> answers nothing under 0.06 body radii, everything past 0.26, and a smooth ramp between.",
    "What is new is that the eyes look further, hold for uneven lengths, and one look in three goes the whole way to one side. That look is what moves the body, and it pulls the body into a pear pointed at it as the eyes go, not after: the front is drawn out and narrowed with the eyes leading it, the back is left round, and the top cranes over on a planted base.",
  );
  s.append(
    pair((kit) => {
      const row = h("div", { class: "row" });
      for (const key of RAIL) row.append(live(new Creature(kit, key, "idle", "lg")).el);
      return row;
    }),
  );
  return s;
}

function lookDemo(): HTMLElement {
  const s = section(
    "The look moves the body. Nonlinearly.",
    "Drag the look. Shipped bulges toward a glance and a stare in proportion; proposed answers a glance with the eyes alone and a stare by stretching into a pear with the eyes at the narrow end. Press a flick to see the timing: shipped moves the eyes at once and the body half a second later; proposed moves both together, on one smoothed look, and a long look crosses more slowly than a glance.",
    "The body's answer is capped at 0.38 body radii of look, and the stretch is then cut to whatever room the outline has left, measured on the outline every frame. That last part fixes a hole that predates this pass: a puddle (<code>stuck</code>, <code>paused</code>) or a tall face (<code>surprised</code>) aimed downward at a peer already drew past <code>FORM.reach</code> by up to two units, and nothing sampled it.",
  );

  const subjects = KITS.map((kit) => new Creature(kit, "pebble", "idle", "xl"));
  for (const c of subjects) {
    c.hold = [0, 0];
    live(c);
  }

  const x = h("input", { type: "range", min: "-0.45", max: "0.45", step: "0.005", value: "0" });
  const y = h("input", { type: "range", min: "-0.3", max: "0.3", step: "0.005", value: "0" });
  const read = h("output", {}, "0.00, 0.00");
  const apply = () => {
    const gx = Number.parseFloat(x.value);
    const gy = Number.parseFloat(y.value);
    for (const c of subjects) c.hold = [gx, gy];
    read.textContent = `${gx.toFixed(2)}, ${gy.toFixed(2)} · grip ${AFTER.grip(Math.hypot(gx, gy)).toFixed(2)}`;
  };
  x.addEventListener("input", apply);
  y.addEventListener("input", apply);

  const flick = (gx: number, gy: number) => () => {
    x.value = String(gx);
    y.value = String(gy);
    apply();
  };
  const buttons = h(
    "div",
    { class: "buttons" },
    button("Flick left", flick(-0.4, 0)),
    button("Flick right", flick(0.4, 0)),
    button("Glance", flick(0.12, -0.04)),
    button("Up", flick(0, -0.22)),
    button("Down", flick(0, 0.3)),
    button("Center", flick(0, 0)),
  );

  let auto: number | null = null;
  const autoBox = h("input", { type: "checkbox" });
  const AUTO: [number, number, number][] = [
    [0, 0, 1.6],
    [0.12, -0.03, 1.4],
    [0, 0, 1.2],
    [0.4, 0, 1.8],
    [0, 0, 1.6],
    [-0.1, 0.04, 1.4],
    [-0.4, 0.05, 1.8],
    [0, 0, 1.2],
  ];
  autoBox.addEventListener("change", () => {
    if (auto !== null) {
      window.clearTimeout(auto);
      auto = null;
    }
    if (!autoBox.checked) return;
    let i = 0;
    const step = () => {
      const [gx, gy, hold] = AUTO[i % AUTO.length] as [number, number, number];
      flick(gx, gy)();
      i++;
      auto = window.setTimeout(step, (hold * 1000) / speed);
    };
    step();
  });

  const controls = h(
    "div",
    { class: "controls" },
    h("label", {}, "Across ", x),
    h("label", {}, "Up and down ", y),
    read,
    buttons,
    h("label", { class: "check" }, autoBox, " Glance, stare, glance, stare"),
  );

  s.append(
    h(
      "div",
      { class: "stage-pair" },
      labelled(BEFORE, h("div", { class: "stage" }, (subjects[0] as Creature).el)),
      labelled(AFTER, h("div", { class: "stage" }, (subjects[1] as Creature).el)),
      h("div", { class: "plot" }, gripPlot()),
    ),
    controls,
    lede(
      "The same look, held still, at four distances. Left to right: straight ahead, a glance, a look, and the edge of the idle range. Shipped moves the body from the first step; proposed moves it from the third.",
    ),
    pair((kit) => {
      const row = h("div", { class: "row" });
      for (const gx of [0, 0.12, 0.25, 0.4]) {
        const c = new Creature(kit, "pebble", "idle", "xl");
        c.hold = [gx, 0];
        painters.push(() => c.paint(0, false));
        row.append(h("div", { class: "still" }, c.el, h("small", {}, gx.toFixed(2))));
      }
      return row;
    }),
  );
  return s;
}

/** `grip` against the identity it replaced. */
function gripPlot(): SVGSVGElement {
  const W = 220;
  const H = 160;
  const pad = 26;
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.setAttribute("class", "grip");
  const sx = (r: number) => pad + (r / 0.5) * (W - pad - 8);
  const sy = (g: number) => H - pad - (g / 0.5) * (H - pad - 8);
  const line = (pts: string, cls: string) => {
    const p = document.createElementNS(SVG, "polyline");
    p.setAttribute("points", pts);
    p.setAttribute("class", cls);
    svg.append(p);
  };
  line(`${sx(0)},${sy(0)} ${sx(0.5)},${sy(0.5)}`, "grip__was");
  const pts: string[] = [];
  for (let r = 0; r <= 0.5001; r += 0.005) pts.push(`${sx(r)},${sy(AFTER.grip(r))}`);
  line(pts.join(" "), "grip__is");
  const mark = (r: number, label: string) => {
    const g = document.createElementNS(SVG, "line");
    g.setAttribute("x1", String(sx(r)));
    g.setAttribute("x2", String(sx(r)));
    g.setAttribute("y1", String(sy(0)));
    g.setAttribute("y2", String(sy(0.5)));
    g.setAttribute("class", "grip__tick");
    const t = document.createElementNS(SVG, "text");
    t.setAttribute("x", String(sx(r)));
    t.setAttribute("y", String(H - 8));
    t.setAttribute("text-anchor", "middle");
    t.textContent = label;
    svg.append(g, t);
  };
  const P = afterForm.PULL;
  mark(P.quiet, "quiet");
  mark(P.wide, "wide");
  mark(P.hold, "hold");
  const ax = document.createElementNS(SVG, "text");
  ax.setAttribute("x", "4");
  ax.setAttribute("y", "12");
  ax.textContent = "body answers";
  const ay = document.createElementNS(SVG, "text");
  ay.setAttribute("x", String(W - 8));
  ay.setAttribute("y", String(H - pad - 6));
  ay.setAttribute("text-anchor", "end");
  ay.textContent = "eyes went";
  svg.append(ax, ay);
  return svg;
}

const POSES: { name: string; note: string; eye: Eye }[] = [
  { name: "round", note: "idle. A dot.", eye: { w: 0.02, h: 2 } },
  { name: "wide", note: "surprised. The same dot, heavier.", eye: { w: 0.02, h: 2.85, sep: 0.4 } },
  { name: "raised", note: "listening. Lifted off the line: brows up.", eye: { w: 0.02, h: 2.45, sep: 0.3, dy: -0.5 } },
  {
    name: "cocked",
    note: "thinking. One brow up, the other eye squinting: skew and lop.",
    eye: { w: 0.4, h: 1.6, a: -12, skew: 0.42, lop: 0.4 },
  },
  { name: "dash", note: "working. A lid, tilted in.", eye: { w: 1.3, h: 0.85, a: 20, dy: 0.25 } },
  {
    name: "scowl",
    note: "frustrated. The brow is all that is left of the eye.",
    eye: { w: 1.65, h: 0.78, a: 40, c: -0.1, dy: -0.5 },
  },
  {
    name: "worried",
    note: "stuck. Inner ends up, uneven, low.",
    eye: { w: 1.1, h: 0.7, a: -28, c: 0.14, dy: 0.35, sep: -0.15, skew: 0.24 },
  },
  {
    name: "doubting",
    note: "blocked, at its badge. Squint, one brow up, one eye narrowed.",
    eye: { w: 0.82, h: 1.88, a: 20, sep: 0.2, skew: 0.36, lop: 0.3 },
  },
  { name: "smile", note: "pleased. The only bow that curves up.", eye: { w: 1.05, h: 0.8, c: -0.8, dy: -0.2 } },
  { name: "shut", note: "paused. Bowed down, no weight.", eye: { w: 1.35, h: 0.5, c: 0.16 } },
];

function vocabulary(): HTMLElement {
  const s = section(
    "The eye is still one stroke.",
    "There is no second object. An eye is an arc with four numbers on it (length, weight, bow, tilt) plus two now that let the pair disagree: <code>skew</code> lifts one over the other and <code>lop</code> narrows one against the other. A brow is what the stroke becomes when it is raised, thinned and tilted until nothing of the eye is left, and because nothing is swapped, a face can sit anywhere between any two of these. Drawn on Orb, held still, with the proposed code.",
  );
  const sheet = h("div", { class: "sheet" });
  const stills: Creature[] = [];
  for (const pose of POSES) {
    const c = new Creature(AFTER, "orb", "idle", "lg");
    c.pose = pose.eye;
    c.hold = [0, 0];
    stills.push(c);
    sheet.append(
      h("div", { class: "still" }, c.el, h("b", {}, pose.name), h("small", {}, pose.note)),
    );
  }
  s.append(sheet);

  const from = select(POSES.map((p) => p.name), "round");
  const to = select(POSES.map((p) => p.name), "scowl");
  const strip = h("div", { class: "strip" });
  const morphs: Creature[] = [];
  const STEPS = 7;
  for (let i = 0; i < STEPS; i++) {
    const c = new Creature(AFTER, "orb", "idle", "md");
    c.hold = [0, 0];
    morphs.push(c);
    strip.append(c.el);
  }
  const redraw = () => {
    const a = POSES.find((p) => p.name === from.value)?.eye ?? POSES[0]?.eye;
    const b = POSES.find((p) => p.name === to.value)?.eye ?? POSES[0]?.eye;
    if (!a || !b) return;
    morphs.forEach((c, i) => {
      c.pose = AFTER.blendEyes(a, b, i / (STEPS - 1));
      c.paint(0, false);
    });
  };
  from.addEventListener("change", redraw);
  to.addEventListener("change", redraw);
  s.append(
    h("p", { class: "lede" }, "Halfway between any two, by lerping the numbers. Pick a pair."),
    h("div", { class: "controls" }, h("label", {}, "From ", from), h("label", {}, "to ", to)),
    strip,
  );
  painters.push(() => {
    for (const c of stills) c.paint(0, false);
  });
  redraw();
  return s;
}

const TRIO = ["orb", "slab", "tide"];

function moods(): HTMLElement {
  const s = section(
    "Ten moods, three shapes.",
    "One card per row of the table in <code>docs/CHARACTERS.md</code>. Circle, square and cloud, because the shear and the crane read differently on a flat side than on a round one, and the cloud is the one that is not convex.",
  );
  const grid = h("div", { class: "cards" });
  for (const key of MOOD_KEYS) {
    const card = h("div", { class: "card" }, h("h3", {}, key), h("p", {}, MOOD_NOTES[key]));
    card.append(
      pair((kit) => {
        const row = h("div", { class: "row row--tight" });
        for (const who of TRIO) row.append(live(new Creature(kit, who, key, "lg", `${who}-${key}`)).el);
        return row;
      }),
    );
    grid.append(card);
  }
  s.append(grid);
  return s;
}

const MORNING: { mood: Mood; look?: Look; gesture?: Gesture; dur: number; say: string }[] = [
  { mood: "idle", dur: 5, say: "nothing in flight" },
  { mood: "listening", dur: 2.4, say: "a message queued" },
  { mood: "thinking", dur: 4, say: "between rounds" },
  { mood: "working", dur: 4, say: "a tool call in flight" },
  { mood: "frustrated", dur: 3, say: "the call back was refused" },
  { mood: "working", dur: 2.4, say: "trying again" },
  { mood: "pleased", dur: 2.6, say: "the reply landed" },
  { mood: "idle", dur: 3, say: "nothing in flight" },
  { mood: "surprised", look: "down", gesture: "receive", dur: 0.9, say: "hit from below" },
  { mood: "thinking", look: "down", dur: 1.6, say: "still looking at who threw it" },
  { mood: "thinking", dur: 2, say: "between rounds" },
  { mood: "thinking", look: "up", gesture: "send", dur: 1.4, say: "sending upward" },
  { mood: "blocked", dur: 5, say: "parked on a person" },
  { mood: "stuck", dur: 5, say: "an escalation open" },
  { mood: "paused", dur: 4, say: "paused" },
];

function morning(): HTMLElement {
  const s = section(
    "A morning, twice.",
    "The same script of signals run through both versions: every mood the app can reach, in an order a real agent might, with a message caught from below and one thrown upward. The label is the signal; the face is what each version makes of it.",
  );
  const subjects = KITS.map((kit) => new Creature(kit, "gourd", "idle", "xl", "morning"));
  for (const c of subjects) live(c);
  const label = h("div", { class: "cue" }, "");
  let i = -1;
  let until = 0;
  painters.push((seconds) => {
    if (seconds < until) return;
    i = (i + 1) % MORNING.length;
    const step = MORNING[i] as (typeof MORNING)[number];
    until = seconds + step.dur;
    for (const c of subjects) {
      c.want = step.mood;
      c.look = step.look ?? null;
      if (step.gesture) c.throw_(step.gesture);
    }
    label.textContent = `${step.mood} · ${step.say}`;
  });
  s.append(
    h(
      "div",
      { class: "stage-pair" },
      labelled(BEFORE, h("div", { class: "stage" }, (subjects[0] as Creature).el)),
      labelled(AFTER, h("div", { class: "stage" }, (subjects[1] as Creature).el)),
    ),
    label,
  );
  return s;
}

function numbers(): HTMLElement {
  const s = section(
    "What moved, in numbers.",
    "Everything is a constant in <code>src/avatars</code>. Nothing on this page was drawn by hand.",
  );
  const rows: [string, string, string][] = [
    ["idle breath", "0.03 at 0.22 Hz", "none: the idle body is still"],
    ["idle outline wobble", "0.008, 0.006", "none"],
    ["idle look range", "0.28 body radii, uniform", "0.40, 30% of looks to the full side, the rest inside 0.6 of it"],
    ["saccade timing", "on the beat", "in the first half of each slot, varied"],
    ["body's answer to a look", "linear, 0.8 swell", "ramp: 0 under 0.06, full past 0.26, capped at 0.38"],
    ["what the answer does", "a bulge on the near side, a flatten on the far", "a pear: front drawn out by 1.2 and narrowed by 1.1 per body radius of look, back left round"],
    ["eye travel across", "1.0 of the look", "up to 1.6 as the body answers, so the eyes lead the snout"],
    ["lean of the center", "0.10", "0.10, plus a shear of 0.20 about a pivot 0.45 under the center"],
    ["crane", "none", "1% taller per 0.1 of look, half that thinner"],
    ["when the body moves", "0.44 s after the eyes, on a spring", "with the eyes, on one 0.12 s smoother in front of both"],
    ["how long a look takes", "cross, whatever the distance", "cross for a glance, up to twice it for a look the whole way"],
    ["stretch against the reach", "tuned, worst frame 0.02 under", "cut on the outline each frame, tested in 16 directions past the cap"],
    ["eye numbers", "w, h, c, a, sep, dx, dy", "and skew, and lop"],
    ["far eye on a sideways look", "same size", "30% smaller per body radius of look, near eye larger"],
    ["stuck", "small dots, staring", "worried brows, darting"],
    ["tests in form.test.ts", "15", "28"],
  ];
  const table = h("table", {}, h("tr", {}, h("th", {}, ""), h("th", {}, "Shipped"), h("th", {}, "Proposed")));
  for (const [what, was, is] of rows) {
    table.append(h("tr", {}, h("td", {}, what), h("td", {}, was), h("td", {}, is)));
  }
  s.append(table);
  return s;
}

/* --- controls ---------------------------------------------------------------- */

function button(label: string, onClick: () => void, pressed?: boolean): HTMLButtonElement {
  const b = h("button", { type: "button" }, label);
  if (pressed !== undefined) b.setAttribute("aria-pressed", String(pressed));
  b.addEventListener("click", onClick);
  return b;
}

function select(options: string[], value: string): HTMLSelectElement {
  const s = h("select");
  for (const o of options) s.append(h("option", { value: o }, o));
  s.value = value;
  return s;
}

function group(
  name: string,
  options: { label: string; value: string }[],
  current: string,
  onPick: (value: string) => void,
): HTMLElement {
  const box = h("span", { class: "group" });
  const buttons = options.map((o) =>
    button(
      o.label,
      () => {
        for (const b of buttons) b.setAttribute("aria-pressed", "false");
        const me = buttons[options.indexOf(o)];
        me?.setAttribute("aria-pressed", "true");
        onPick(o.value);
      },
      o.value === current,
    ),
  );
  box.append(...buttons);
  return h("label", {}, `${name} `, box);
}

function bar(): HTMLElement {
  const play = button("Pause", () => {
    paused = !paused;
    play.textContent = paused ? "Play" : "Pause";
  });
  return h(
    "div",
    { class: "bar" },
    play,
    group(
      "Speed",
      [
        { label: "½", value: "0.5" },
        { label: "1", value: "1" },
        { label: "2", value: "2" },
      ],
      "1",
      (v) => {
        speed = Number.parseFloat(v);
      },
    ),
    group(
      "Behind",
      [
        { label: "Paper", value: "paper" },
        { label: "Ink", value: "ink" },
      ],
      "paper",
      (v) => document.body.setAttribute("data-surface", v),
    ),
  );
}

/* --- the page ----------------------------------------------------------------- */

const CSS = `
:root {
  --ink: #0b0b0a; --paper: #ffffff; --rail: #f5f3ee; --edge: #e2e0da;
  --muted: #54524d; --faint: #8a877f; --amber: #b4530a; --flesh: #b4530a;
}
* { box-sizing: border-box; }
body { margin: 0; font: 15px/1.6 ui-sans-serif, -apple-system, "Segoe UI", sans-serif; color: var(--ink); background: var(--rail); }
main { max-width: 1180px; margin: 0 auto; padding: 56px 32px 120px; }
h1 { font-size: 34px; letter-spacing: -0.02em; margin: 0 0 4px; }
h2 { font-size: 11px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--faint); margin: 56px 0 14px; font-weight: 600; }
h3 { margin: 0; font-size: 17px; letter-spacing: -0.01em; }
.lede { max-width: 72ch; color: var(--muted); margin: 0 0 10px; }
.lede strong { color: var(--ink); font-weight: 600; }
code { font: 12px ui-monospace, monospace; background: var(--rail); border: 1px solid var(--edge); border-radius: 4px; padding: 1px 5px; }
.bar { position: sticky; top: 0; z-index: 10; display: flex; gap: 22px; flex-wrap: wrap; align-items: center; margin: 28px 0 0; padding: 12px 16px; background: rgba(245,243,238,0.93); backdrop-filter: blur(8px); border: 1px solid var(--edge); border-radius: 10px; }
.bar label { font-size: 12px; color: var(--muted); display: flex; gap: 7px; align-items: center; }
.group { display: flex; gap: 2px; background: #fff; border: 1px solid var(--edge); border-radius: 7px; padding: 2px; }
button { font: inherit; font-size: 12px; border: 1px solid var(--edge); background: #fff; color: var(--muted); padding: 4px 10px; border-radius: 6px; cursor: pointer; }
.group button { border: 0; background: transparent; border-radius: 5px; }
button[aria-pressed="true"] { background: var(--ink); color: #fff; }
select { font: inherit; font-size: 12px; padding: 3px 6px; border: 1px solid var(--edge); border-radius: 6px; background: #fff; }
.pair { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.labelled { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.tag { font-size: 9px; letter-spacing: 0.1em; text-transform: uppercase; font-weight: 700; color: var(--faint); }
.labelled--after .tag { color: var(--amber); }
.row { display: flex; flex-wrap: wrap; gap: 18px; padding: 22px; border-radius: 12px; align-items: center; }
.row--tight { gap: 10px; padding: 14px; }
.stage-pair { display: grid; grid-template-columns: 1fr 1fr auto; gap: 16px; align-items: end; }
.stage { display: flex; justify-content: center; align-items: center; padding: 28px; border-radius: 12px; min-height: 200px; }
.plot { align-self: stretch; display: flex; align-items: flex-end; }
body[data-surface="paper"] .row, body[data-surface="paper"] .stage, body[data-surface="paper"] .sheet, body[data-surface="paper"] .strip { background: #fff; box-shadow: inset 0 0 0 1px var(--edge); }
body[data-surface="ink"] .row, body[data-surface="ink"] .stage, body[data-surface="ink"] .sheet, body[data-surface="ink"] .strip { background: #17171a; }
body[data-surface="ink"] .still b, body[data-surface="ink"] .still small { color: rgba(255,255,255,0.75); }
.controls { display: flex; flex-wrap: wrap; gap: 16px; align-items: center; margin: 14px 0 0; font-size: 12px; color: var(--muted); }
.controls label { display: flex; gap: 8px; align-items: center; }
.controls input[type="range"] { width: 180px; }
.controls output { font: 12px ui-monospace, monospace; }
.buttons { display: flex; gap: 6px; flex-wrap: wrap; }
.check { gap: 6px; }
.grip { width: 220px; height: 160px; font: 10px ui-monospace, monospace; fill: var(--faint); }
.grip .grip__was { fill: none; stroke: var(--edge); stroke-width: 1.5; }
.grip .grip__is { fill: none; stroke: var(--amber); stroke-width: 2; }
.grip .grip__tick { stroke: var(--edge); stroke-dasharray: 2 3; }
.sheet { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 14px; padding: 18px; border-radius: 12px; }
.still { display: flex; flex-direction: column; align-items: center; gap: 4px; text-align: center; }
.still b { font-size: 12px; }
.still small { font-size: 11px; color: var(--muted); line-height: 1.35; }
.strip { display: flex; gap: 14px; padding: 16px 20px; border-radius: 12px; margin-top: 10px; align-items: center; }
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(520px, 1fr)); gap: 18px; }
.card { background: #fff; border: 1px solid var(--edge); border-radius: 12px; padding: 18px; display: flex; flex-direction: column; gap: 10px; }
.card p { margin: 0; font-size: 13px; color: var(--muted); min-height: 3.6em; }
.card .row { padding: 12px; }
.cue { margin-top: 12px; font: 13px ui-monospace, monospace; color: var(--muted); }
table { border-collapse: collapse; font-size: 13px; max-width: 900px; }
th, td { text-align: left; padding: 6px 12px 6px 0; border-bottom: 1px solid var(--edge); vertical-align: top; }
th { font-size: 10px; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); }
td:first-child { color: var(--muted); white-space: nowrap; }
footer { margin-top: 64px; font-size: 13px; color: var(--muted); max-width: 72ch; }

/* .avatar, as in src/styles.css */
.avatar { --eye: color-mix(in oklab, var(--accent, var(--flesh)) 34%, #171410); display: inline-grid; place-items: center; flex: none; position: relative; }
.avatar--md { width: 2.75rem; height: 2.75rem; }
.avatar--lg { width: 4.25rem; height: 4.25rem; }
.avatar--xl { width: 9rem; height: 9rem; }
.avatar__body { width: 100%; height: 100%; overflow: visible; }
.avatar[data-mood="paused"] .avatar__body { filter: grayscale(0.72); opacity: 0.5; }
.avatar__dots circle { animation: think-dot 1.5s ease-in-out infinite; animation-delay: calc(var(--gait, 0s) * -1); }
.avatar__dots circle:nth-child(2) { animation-delay: calc(var(--gait, 0s) * -1 + 0.18s); }
.avatar__dots circle:nth-child(3) { animation-delay: calc(var(--gait, 0s) * -1 + 0.36s); }
@keyframes think-dot { 0%, 100% { opacity: 0.15; } 50% { opacity: 0.9; } }
.avatar__halo { transform-box: fill-box; transform-origin: center; animation: attention 1.9s ease-in-out infinite; animation-delay: calc(var(--gait, 0s) * -1); }
@keyframes attention { 0% { opacity: 0.5; transform: scale(0.85); } 100% { opacity: 0; transform: scale(1.5); } }
.avatar__z { animation: snooze 3.4s ease-in-out infinite; animation-delay: calc(var(--gait, 0s) * -1); }
@keyframes snooze { 0% { opacity: 0; transform: translate(0, 1px) scale(0.75); } 30% { opacity: 0.75; } 100% { opacity: 0; transform: translate(4px, -7px) scale(1.15); } }
`;

function page() {
  document.head.append(h("style", {}, CSS));
  document.title = "Guaca characters: the caricature pass";
  document.body.setAttribute("data-surface", "paper");
  const main = h("main");
  main.append(
    h("h1", {}, "Characters, caricatured"),
    lede(
      "Less body when idle, more body when the eyes go to the edge, more from the eyes all round, and the eye itself becoming the brow. Every creature on this page is drawn by the app's own code: the left of each pair by <code>src/avatars</code> at HEAD, the right by the working tree. Same clock, same seeds, same script.",
    ),
    lede(
      "What to look for, in order: whether the idle rail sits still; whether a hard sideways look now reads as the creature turning toward it rather than a mark sliding; whether the brows read at 44px, which is the rail's size; and whether <code>stuck</code> should change meaning, which is the one call on this page that is not about drawing.",
    ),
    bar(),
    idleRail(),
    lookDemo(),
    vocabulary(),
    moods(),
    morning(),
    numbers(),
    h(
      "footer",
      {},
      h(
        "p",
        {},
        "Gates: form.test.ts holds the outline to FORM.reach in every direction past the cap and both eyes inside the body in every mood and under both aimed looks. moods.test.ts still proves every mood is reachable from a real signal. No new mood, no new shape, no second object on a face, no transform on a drawing.",
      ),
    ),
  );
  document.body.append(main);
  /* `?only=<section id>` leaves one section on the page, which is how a still
     of it is taken without scrolling a headless browser. */
  const only = new URLSearchParams(location.search).get("only");
  if (only) for (const el of main.querySelectorAll("section")) if (el.id !== only) el.remove();
  if (location.hash) document.querySelector(location.hash)?.scrollIntoView();
  requestAnimationFrame(frame);
}

function lede(html: string): HTMLElement {
  const p = h("p", { class: "lede" });
  p.innerHTML = html;
  return p;
}

page();
