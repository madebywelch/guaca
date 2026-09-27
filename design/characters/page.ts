/**
 * The review page: the cast as it ships beside three redesigns, all live.
 *
 *   ./design/make-characters.sh
 */

import { ACCENTS, CHARACTERS } from "../../src/avatars/catalog";
import { baseline } from "./baseline";
import { gel } from "./gel";
import { ink } from "./ink";
import { lids } from "./lids";
import {
  type Design,
  type Handle,
  type Look,
  MOOD_LIST,
  MOOD_SIGNAL,
  type Mood,
  mount,
  onTick,
  world,
} from "./shared";

const DESIGNS = [baseline, lids, gel, ink];

const PIGMENT = Object.fromEntries(ACCENTS.map((a) => [a.name, a.value])) as Record<string, string>;

/** One character per mood, the same in every design, so a column compares like with like. */
const BY_MOOD: Record<Mood, { who: string; color: string }> = {
  idle: { who: "orb", color: PIGMENT.Olive as string },
  listening: { who: "bead", color: PIGMENT.Rose as string },
  thinking: { who: "husk", color: PIGMENT.Indigo as string },
  working: { who: "crumb", color: PIGMENT.Terracotta as string },
  frustrated: { who: "pip", color: PIGMENT.Madder as string },
  blocked: { who: "slab", color: PIGMENT.Slate as string },
  pleased: { who: "wave", color: PIGMENT.Moss as string },
  paused: { who: "pebble", color: PIGMENT.Graphite as string },
  stuck: { who: "gourd", color: PIGMENT.Clay as string },
  surprised: { who: "lobe", color: PIGMENT.Verdigris as string },
};

/** What the stage offers: one of each shape, and one with a single eye. */
const STAGE_CAST: { who: string; color: string; label: string }[] = [
  { who: "orb", color: PIGMENT.Olive as string, label: "circle" },
  { who: "puck", color: PIGMENT.Indigo as string, label: "octagon" },
  { who: "slab", color: PIGMENT.Slate as string, label: "square" },
  { who: "drop", color: PIGMENT.Madder as string, label: "drop" },
  { who: "lobe", color: PIGMENT.Verdigris as string, label: "cloud" },
  { who: "cell", color: PIGMENT.Ochre as string, label: "one eye" },
];

interface Note {
  title: string;
  body: string;
}

interface Copy {
  eyebrow: string;
  title: string;
  thesis: string;
  notes: Note[];
}

const COPY: Record<string, Copy> = {
  now: {
    eyebrow: "Baseline",
    title: "Now",
    thesis:
      "Five paper silhouettes and one ink stroke per eye, drawn here by src/avatars itself. The silhouettes carry identity well. The eyes are asked to carry everything else, and at 24px they cannot.",
    notes: [
      {
        title: "No pupil",
        body: "A look is the whole eye sliding across the face, so every glance moves the face and every stare spends outline. Most of the FORM.reach budget pays for eyes that have to travel.",
      },
      {
        title: "Four numbers for ten moods",
        body: "Working, frustrated, paused and stuck are all dashes. At 24px they differ by the tilt of a line two pixels long.",
      },
      {
        title: "No lid",
        body: "Looking down at a peer is faked by molding the stroke toward a line (aimedEye). It reads as a squint, not as looking at someone below.",
      },
      {
        title: "Kinematic motion",
        body: "A mood change is a 0.6s smoothstep. Nothing overshoots, nothing settles, nothing has weight. The only spring on the body is the knock.",
      },
      {
        title: "Blinks on a timer",
        body: "A face blinks into a large saccade. These blink every four seconds or so, whatever the eyes are doing.",
      },
      {
        title: "No light",
        body: "The relief is a fixed offset. A lean reads as a skew rather than a turn, and nothing stands on anything.",
      },
    ],
  },
  lids: {
    eyebrow: "Direction A",
    title: "Lids",
    thesis:
      "Same body. A real eye: white, pupil, two lids. The pupil takes the glance and the lids take the emotion, which is how an animator divides a face.",
    notes: [
      {
        title: "Shape",
        body: "Unchanged, on purpose. The silhouette carries identity well and the eye carries emotion badly, so this fixes the eye. The body here is bodyPoints and MOODS[m].shape, untouched.",
      },
      {
        title: "Eyes",
        body: "The pupil moves inside the white, so a glance costs the face nothing and only a stare moves the eyes and the body. That is what grip approximates today, now true by construction. The upper lid rides the pupil down, which is what makes a look at a peer below read.",
      },
      {
        title: "Motion",
        body: "Lids lead a large saccade with a blink, two times in three, and blink into every aimed look. Pupils open a tenth while the creature watches you. Working reads: short steps right along a line, one long return.",
      },
      {
        title: "Emotion",
        body: "Five numbers on the lids. Inner ends down is cross, inner ends up is worry, a raised lower lid is a smile, retracted lids round pinned pupils is shock, one lid lower than the other is doubt. None of the ten is a dash.",
      },
      {
        title: "Cost",
        body: "Whites read younger than ink dots. The eye grows by half to keep a pupil at 24px, so form.test.ts has to re-measure the ink against the outline. Porting it is eyes.ts and the paths AgentAvatar writes; the body, the moods' shapes and the clock do not change.",
      },
    ],
  },
  gel: {
    eyebrow: "Direction B",
    title: "Gel",
    thesis:
      "Motion is simulated, not scripted. A lit soft body standing on a floor: thirty-two masses on springs, a pressure term, a contact shadow. Weight falls out of the physics.",
    notes: [
      {
        title: "Shape",
        body: "A gel cannot hold a corner, so the square is a pillow and the octagon a gumdrop, each sized to the circle's area. Thinking and blocked tilt the rest shape about its base: a head tilt with the feet planted.",
      },
      {
        title: "Material",
        body: "Lit from the top left: gradient, inner rim, specular, and a contact shadow that widens as the body squashes. The shadow is the most useful thing in this direction. It plants the creature, so a squash reads as weight rather than as a smaller drawing.",
      },
      {
        title: "Motion",
        body: "A mood is a rest shape and a tension. Changing mood changes the rest shape and the body flows into it, with overshoot and settle nobody keyframed. A message landing is an impulse. A look pulls the goal into the pear form.ts already uses, so the mass arrives a few frames late.",
      },
      {
        title: "Eyes",
        body: "Glossy beads with a catch-light that belongs to the light, not the eye: when the bead turns, the glint slides the other way across it. Cuts across the bead are the brow. Stuck gets a second, lower glint, which is a wet eye.",
      },
      {
        title: "Emotion",
        body: "Tension is the new channel. Frustrated is stiff and trembling, paused is slack and sagging, stuck heaves up and falls back, surprised jolts tall and wobbles down.",
      },
      {
        title: "Cost",
        body: "A lit gel reads as a toy beside long-form text. State per creature: a still frame is a settled simulation, not a function of time. And it is the body doing the acting, which CHARACTERS.md warns against; the amplitudes here are the ceiling, not a target.",
      },
    ],
  },
  ink: {
    eyebrow: "Direction C",
    title: "Ink",
    thesis:
      "Drawn, not cut. One brush stroke over a pigment printed out of register, twelve drawings a second, and the line itself as a channel for mood.",
    notes: [
      {
        title: "Shape",
        body: "Today's five silhouettes and today's body code, drawn as one stroke that presses at the bottom and overlaps where it began. Each character wears a mark on its head (a curl, a leaf, a tick, or nothing) that lags the body: the only secondary motion in the set, and a second identity cue at 24px.",
      },
      {
        title: "Timing",
        body: "On twos. Eyes snap and hold. A large look is hidden behind a blink, the way an animator cuts a head turn. A mood change is five drawings with one past the target. It also paints a fifth as often, which matters at sixty faces.",
      },
      {
        title: "Emotion",
        body: "Three channels. Dot eyes under ink brows. A mouth, only at 40px and up. And the line: calm moods hold a clean line, work boils it, frustration scratches it, a paused creature's line runs dry.",
      },
      {
        title: "Marks",
        body: "Comics already solved emotion at small sizes: a scribble is cross, a drop of sweat is worry, lines round the head are shock, a star is pleased. Amber is still spent on blocked and nothing else.",
      },
      {
        title: "Cost",
        body: "Brows and a mouth are back. They are computed from the eye, so register is not the failure it was, but the mouth is gated by size, so a face changes between the rail and a header. Boil in a rail of sixty is noise; it is spent only on work and still reads busier than A.",
      },
    ],
  },
};

/* --- building blocks ---------------------------------------------------------- */

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else node.setAttribute(k, v);
  }
  for (const c of children) node.append(c);
  return node;
}

function spot(design: Design, who: string, color: string, px: number, mood: Mood, follow = false, seed?: string) {
  const host = h("span");
  const handle = mount(host, design, { who, color, px, mood, follow, seed });
  return { host, handle };
}

/* --- the stage ----------------------------------------------------------------- */

const LADDER = [68, 44, 32, 24];

function stage(design: Design): HTMLElement {
  let cast = STAGE_CAST[0] as (typeof STAGE_CAST)[number];
  let mood: Mood = "idle";
  let look: Look = null;
  let cycling = true;
  let cycleAt = performance.now() / 1000;
  let handles: Handle[] = [];

  const hero = h("div", { class: "stage__hero" });
  const ladder = h("div", { class: "stage__ladder" });
  const moodLabel = h("div", { class: "stage__mood" });

  const build = () => {
    for (const x of handles) x.destroy();
    handles = [];
    hero.replaceChildren();
    ladder.replaceChildren();
    const big = spot(design, cast.who, cast.color, 232, mood, true, `stage-${design.key}`);
    hero.append(big.host);
    handles.push(big.handle);
    for (const px of LADDER) {
      const small = spot(design, cast.who, cast.color, px, mood, true, `stage-${design.key}`);
      ladder.append(h("div", { class: "stage__rung" }, small.host, h("span", {}, `${px}px`)));
      handles.push(small.handle);
    }
    for (const x of handles) x.setLook(look);
  };

  const setMood = (m: Mood) => {
    mood = m;
    for (const x of handles) x.setMood(m);
    moodLabel.replaceChildren(h("strong", {}, m), h("span", {}, MOOD_SIGNAL[m]));
    for (const chip of moodChips.children) chip.classList.toggle("is-on", (chip as HTMLElement).dataset.mood === m);
  };

  const moodChips = h("div", { class: "chips" });
  for (const m of MOOD_LIST) {
    const chip = h("button", { class: "chip", "data-mood": m, type: "button" }, m);
    chip.addEventListener("click", () => {
      cycling = false;
      cycleChip.classList.remove("is-on");
      setMood(m);
    });
    moodChips.append(chip);
  }
  const cycleChip = h("button", { class: "chip chip--quiet is-on", type: "button" }, "cycle");
  cycleChip.addEventListener("click", () => {
    cycling = !cycling;
    cycleChip.classList.toggle("is-on", cycling);
    cycleAt = performance.now() / 1000;
  });
  moodChips.append(cycleChip);

  const castChips = h("div", { class: "chips" });
  for (const c of STAGE_CAST) {
    const chip = h("button", { class: `chip${c === cast ? " is-on" : ""}`, type: "button" }, c.label);
    chip.addEventListener("click", () => {
      cast = c;
      for (const other of castChips.children) other.classList.toggle("is-on", other === chip);
      build();
    });
    castChips.append(chip);
  }

  const actChips = h("div", { class: "chips" });
  const act = (label: string, fn: () => void) => {
    const chip = h("button", { class: "chip", type: "button" }, label);
    chip.addEventListener("click", fn);
    actChips.append(chip);
    return chip;
  };
  const lookUp = act("look up", () => aim(look === "up" ? null : "up"));
  const lookDown = act("look down", () => aim(look === "down" ? null : "down"));
  act("send", () => {
    if (!look) aim("down");
    for (const x of handles) x.gesture("send");
  });
  act("receive", () => {
    if (!look) aim("up");
    for (const x of handles) x.gesture("receive");
  });
  const aim = (l: Look) => {
    look = l;
    for (const x of handles) x.setLook(l);
    lookUp.classList.toggle("is-on", l === "up");
    lookDown.classList.toggle("is-on", l === "down");
  };

  onTick((now) => {
    if (!cycling || now - cycleAt < 3.4) return;
    cycleAt = now;
    setMood(MOOD_LIST[(MOOD_LIST.indexOf(mood) + 1) % MOOD_LIST.length] as Mood);
  });

  build();
  setMood(mood);

  return h(
    "div",
    { class: "stage" },
    h("div", { class: "stage__floor" }, hero, moodLabel),
    ladder,
    h(
      "div",
      { class: "stage__controls" },
      h("div", { class: "control" }, h("span", { class: "control__label" }, "Mood"), moodChips),
      h("div", { class: "control" }, h("span", { class: "control__label" }, "Shape"), castChips),
      h("div", { class: "control" }, h("span", { class: "control__label" }, "Do"), actChips),
    ),
  );
}

/* --- ten moods ------------------------------------------------------------------ */

function moods(design: Design): HTMLElement {
  const big = h("div", { class: "moods" });
  const small = h("div", { class: "moods moods--small" });
  for (const m of MOOD_LIST) {
    const { who, color } = BY_MOOD[m];
    big.append(
      h(
        "figure",
        { class: "mood" },
        spot(design, who, color, 68, m, false, `mood-${m}`).host,
        h("figcaption", {}, h("strong", {}, m), h("span", {}, MOOD_SIGNAL[m])),
      ),
    );
    small.append(h("div", { class: "mood mood--small" }, spot(design, who, color, 24, m, false, `mood-${m}`).host));
  }
  return h("div", { class: "block" }, h("h3", {}, "Ten moods, at 68px and at 24px"), big, small);
}

/* --- a rail, with a conversation in it ------------------------------------------ */

const CREW: { who: string; color: string; name: string }[] = [
  { who: "husk", color: PIGMENT.Indigo as string, name: "Planner" },
  { who: "orb", color: PIGMENT.Olive as string, name: "Researcher" },
  { who: "crumb", color: PIGMENT.Terracotta as string, name: "Builder" },
  { who: "slab", color: PIGMENT.Slate as string, name: "Reviewer" },
  { who: "gourd", color: PIGMENT.Clay as string, name: "Archivist" },
];

type Beat = [number, number, { mood?: Mood; look?: Look; gesture?: "send" | "receive"; status?: string; parcel?: number }];

const LAP = 15;
const SCRIPT: Beat[] = [
  [0, 0, { mood: "thinking", look: null, status: "between rounds" }],
  [0, 1, { mood: "idle", look: null, status: "" }],
  [0, 2, { mood: "idle", look: null, status: "" }],
  [0, 3, { mood: "blocked", look: null, status: "wants to push" }],
  [0, 4, { mood: "paused", look: null, status: "paused" }],
  [2.2, 0, { look: "down", gesture: "send", status: "to Builder", parcel: 2 }],
  [2.5, 2, { look: "up" }],
  [2.8, 2, { gesture: "receive", mood: "listening", status: "from Planner" }],
  [3.4, 0, { look: null, mood: "idle", status: "" }],
  [3.9, 2, { look: null, mood: "thinking", status: "between rounds" }],
  [5.2, 2, { mood: "working", status: "shell: pnpm test" }],
  [7, 2, { mood: "frustrated", status: "shell refused" }],
  [8.3, 2, { mood: "working", status: "shell: pnpm test" }],
  [9.6, 2, { mood: "pleased", look: "up", gesture: "send", status: "to Planner", parcel: 0 }],
  [9.9, 0, { look: "down" }],
  [10.2, 0, { gesture: "receive", mood: "listening", status: "from Builder" }],
  [10.6, 2, { look: null }],
  [11, 0, { look: null, mood: "thinking", status: "between rounds" }],
  [11.2, 1, { mood: "stuck", status: "escalated" }],
  [12.4, 2, { mood: "idle", status: "" }],
];

function rail(design: Design): HTMLElement {
  const panel = h("div", { class: "rail" });
  const parcel = h("span", { class: "rail__parcel" });
  const rows = CREW.map((c, i) => {
    const { host, handle } = spot(design, c.who, c.color, 32, "idle", false, `rail-${i}`);
    const status = h("span", { class: "rail__status" });
    const row = h("div", { class: "rail__row" }, host, h("span", { class: "rail__who" }, h("span", {}, c.name), status));
    panel.append(row);
    return { host, handle, status, row };
  });
  panel.append(parcel);

  let start = performance.now() / 1000;
  let fired = new Set<number>();
  let flight: { from: number; to: number; at: number } | null = null;
  onTick((now) => {
    let t = now - start;
    if (t >= LAP) {
      start = now;
      t = 0;
      fired = new Set();
    }
    SCRIPT.forEach(([at, who, beat], i) => {
      if (fired.has(i) || t < at) return;
      fired.add(i);
      const row = rows[who];
      if (!row) return;
      if (beat.mood) row.handle.setMood(beat.mood);
      if (beat.look !== undefined) row.handle.setLook(beat.look);
      if (beat.gesture) row.handle.gesture(beat.gesture);
      if (beat.status !== undefined) row.status.textContent = beat.status;
      if (beat.parcel !== undefined) flight = { from: who, to: beat.parcel, at: now };
    });
    if (flight) {
      const p = (now - flight.at - 0.12) / 0.5;
      const a = rows[flight.from]?.host;
      const b = rows[flight.to]?.host;
      if (p > 1 || !a || !b) {
        flight = null;
        parcel.style.opacity = "0";
      } else if (p >= 0) {
        const box = panel.getBoundingClientRect();
        const ra = a.getBoundingClientRect();
        const rb = b.getBoundingClientRect();
        const ease = p * p * (3 - 2 * p);
        const y = ra.top + ra.height / 2 + (rb.top - ra.top) * ease - box.top;
        const x = ra.left + ra.width / 2 - box.left + Math.sin(p * Math.PI) * 22;
        parcel.style.opacity = "1";
        parcel.style.background = CREW[flight.from]?.color ?? "";
        parcel.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px)`;
      }
    }
  });

  const crowd = h("div", { class: "crowd" });
  CHARACTERS.forEach((c, i) => {
    const color = (ACCENTS[(i * 5) % ACCENTS.length] as { value: string }).value;
    crowd.append(h("span", { class: "crowd__one", title: c.label }, spot(design, c.key, color, 32, "idle", false, `crowd-${c.key}`).host));
  });

  return h(
    "div",
    { class: "block block--pair" },
    h("div", {}, h("h3", {}, "A rail, with a conversation in it"), panel),
    h("div", {}, h("h3", {}, "The whole cast, idle, at 32px"), crowd),
  );
}

/* --- sections ------------------------------------------------------------------- */

function section(design: Design): HTMLElement {
  const copy = COPY[design.key] as Copy;
  const notes = h("div", { class: "notes" });
  for (const n of copy.notes) notes.append(h("div", { class: "note" }, h("h4", {}, n.title), h("p", {}, n.body)));
  return h(
    "section",
    { class: "design", id: design.key },
    h(
      "header",
      { class: "design__head" },
      h("span", { class: "eyebrow" }, copy.eyebrow),
      h("h2", {}, copy.title),
      h("p", { class: "thesis" }, copy.thesis),
    ),
    stage(design),
    notes,
    moods(design),
    rail(design),
  );
}

function compare(): HTMLElement {
  const grid = (px: number) => {
    const table = h("div", { class: `grid grid--${px}` });
    table.append(h("span", {}));
    for (const m of MOOD_LIST) table.append(h("span", { class: "grid__mood" }, m));
    for (const d of DESIGNS) {
      table.append(h("span", { class: "grid__design" }, (COPY[d.key] as Copy).title));
      for (const m of MOOD_LIST) {
        const { who, color } = BY_MOOD[m];
        table.append(h("span", { class: "grid__cell" }, spot(d, who, color, px, m, false, `grid-${m}`).host));
      }
    }
    return table;
  };
  return h(
    "section",
    { class: "design", id: "compare" },
    h(
      "header",
      { class: "design__head" },
      h("span", { class: "eyebrow" }, "Side by side"),
      h("h2", {}, "Every mood, every design"),
      h(
        "p",
        { class: "thesis" },
        "Same character per column, same moment on the clock. The first grid is 44px, a row in the rail. The second is 24px, the smallest the app draws, and the size the decision should be made at.",
      ),
    ),
    grid(44),
    grid(24),
    h(
      "div",
      { class: "notes notes--kept" },
      h(
        "div",
        { class: "note" },
        h("h4", {}, "Kept in every direction"),
        h(
          "p",
          {},
          "Moods as one table and moodFor as the only door into it. One clock, and a gait per creature so no two keep time. Nothing drawn past FORM.reach. Amber on blocked and nowhere else. Agents store a key, so any of these ships without a migration.",
        ),
      ),
    ),
  );
}

function toggle(label: string, options: [string, () => void][]): HTMLElement {
  const group = h("div", { class: "toggle" }, h("span", { class: "toggle__label" }, label));
  options.forEach(([name, fn], i) => {
    const b = h("button", { type: "button", class: i === 0 ? "is-on" : "" }, name);
    b.addEventListener("click", () => {
      for (const other of group.querySelectorAll("button")) other.classList.toggle("is-on", other === b);
      fn();
    });
    group.append(b);
  });
  return group;
}

function page() {
  const bar = h(
    "header",
    { class: "bar" },
    h("span", { class: "bar__title" }, "Guaca", h("span", {}, "characters")),
    h(
      "nav",
      { class: "bar__nav" },
      ...DESIGNS.map((d) => h("a", { href: `#${d.key}` }, (COPY[d.key] as Copy).title)),
      h("a", { href: "#compare" }, "Side by side"),
    ),
    h(
      "div",
      { class: "bar__toggles" },
      toggle("Surface", [
        ["Paper", () => delete document.documentElement.dataset.surface],
        ["Dark", () => (document.documentElement.dataset.surface = "dark")],
      ]),
      toggle("Motion", [
        ["Live", () => (world.still = false)],
        ["Still", () => (world.still = true)],
      ]),
      toggle("Pointer", [
        ["Watched", () => (world.followAll = true)],
        ["Ignored", () => (world.followAll = false)],
      ]),
    ),
  );

  const intro = h(
    "section",
    { class: "intro" },
    h("h1", {}, "Characters, three ways"),
    h(
      "p",
      { class: "lede" },
      "The cast as it ships, drawn by src/avatars, beside three redesigns. Every creature here is live: ten moods, the aimed look, both gestures, a rail with a conversation in it. Point anywhere and the creatures on a stage look at you. Still shows what reduced motion gets.",
    ),
    h(
      "div",
      { class: "verdict" },
      h("h4", {}, "Recommendation"),
      h(
        "p",
        {},
        "Ship A's eye on today's body, and take two things from B: the contact shadow, and a spring on mood changes in place of the smoothstep. A fixes the part that is failing and keeps every rule in CHARACTERS.md. B has the best motion on the page and the wrong material for a column of text. C is the most distinctive and the noisiest rail; its emanata are worth stealing for stuck and frustrated if 24px is still not enough.",
      ),
    ),
    h(
      "div",
      { class: "axes" },
      ...[
        ["A. Lids", "Anatomical", "The eye gets a pupil and two lids. Body unchanged."],
        ["B. Gel", "Physical", "The body is simulated. Light, weight, a floor."],
        ["C. Ink", "Graphic", "One brush stroke, on twos. The line carries mood."],
      ].map(([t, k, d]) => h("div", { class: "axis" }, h("strong", {}, t as string), h("span", {}, k as string), h("p", {}, d as string))),
    ),
  );

  document.body.append(bar, h("main", {}, intro, ...DESIGNS.map(section), compare()));
}

page();
