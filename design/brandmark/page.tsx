/**
 * The review page: three ways the name gives the corner back to the cast.
 *
 *   ./design/brandmark/make.sh
 *
 * Every creature on it is the app's own, drawn by the painters in
 * src/avatars; the rows under each title bar are `AgentAvatar` itself.
 */

import { type MutableRefObject, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";

import { AgentAvatar } from "../../src/avatars/AgentAvatar";
import { prefersReducedMotion } from "../../src/lib/motion";
import { Actor, type Cue, REST_CUE } from "./actor";
import { DIRECTIONS, type Direction, type Flags, HOLD, type Happening, PIGMENT, pair } from "./scenes";

type Cast = "cut" | "drawn";

interface View {
  cues: MutableRefObject<Cue>[];
  name(shown: number): void;
  say(text: string): void;
}

const now = () => performance.now() / 1000;
const fresh = (): Flags => ({
  condensedAt: Number.POSITIVE_INFINITY,
  hover: false,
  hoverAt: Number.NEGATIVE_INFINITY,
  visits: 0,
  happening: "none",
  happeningAt: 0,
});

/** The mark: the cast, and the name beside it or in a bubble over it. */
function Mark({
  dir,
  px,
  type,
  bubble: shout,
  cast,
  views,
  hover,
}: {
  dir: Direction;
  px: number;
  /** The name's size, and the bubble's, in CSS pixels. */
  type: number;
  bubble: number;
  cast: Cast;
  views: MutableRefObject<Set<View>>;
  hover: (on: boolean) => void;
}) {
  const cues = dir.cast.map(() => useRef<Cue>({ ...REST_CUE, shown: false }));
  const name = useRef<HTMLSpanElement>(null);
  const word = useRef<HTMLSpanElement>(null);
  const bubble = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const view: View = {
      cues,
      name(shown) {
        const outer = name.current;
        const inner = word.current;
        if (!outer || !inner) return;
        outer.style.width = `${(shown * inner.offsetWidth).toFixed(2)}px`;
        inner.style.clipPath = `inset(0 ${((1 - shown) * 100).toFixed(2)}% 0 0)`;
      },
      say(text) {
        const el = bubble.current;
        if (!el) return;
        el.textContent = text;
        el.classList.remove("on");
        void el.offsetWidth;
        el.classList.add("on");
      },
    };
    views.current.add(view);
    return () => {
      views.current.delete(view);
    };
  });

  /* A box is a fifth empty on either side of the body, which is room for a
     mood to swell into. A pair stands closer than its boxes, as the icon's
     does, the second a little higher; the name stands off the body, not the box. */
  const overlap = Math.round(px * 0.22);
  return (
    <span
      className="bm-mark"
      style={{ gap: Math.round(px * 0.04) }}
      onPointerEnter={(e) => {
        e.currentTarget.dataset.hover = "";
        hover(true);
      }}
      onPointerLeave={(e) => {
        delete e.currentTarget.dataset.hover;
        hover(false);
      }}
    >
      <span className="bm-mark__cast">
        {dir.cast.map((c, i) => (
          <span
            key={c.seed}
            style={{
              display: "inline-flex",
              marginLeft: i > 0 ? -overlap : 0,
              marginBottom: i === 1 ? Math.round(px * 0.12) : 0,
            }}
          >
            <Actor lump={c.lump} color={c.color} px={px} cast={cast} cue={cues[i] as MutableRefObject<Cue>} seed={c.seed} />
          </span>
        ))}
        {dir.voice === "said" && (
          <span
            ref={bubble}
            className="bm-mark__says"
            style={{ fontSize: shout, background: dir.cast[0]?.color }}
          />
        )}
      </span>
      {dir.voice === "set" && (
        <span ref={name} className="bm-mark__name" style={{ width: 0 }}>
          <span ref={word} className="bm-mark__word" style={{ fontSize: type }}>
            Guaca
          </span>
        </span>
      )}
    </span>
  );
}

const CREW = [
  { name: "Scout", avatar: "bean", color: PIGMENT.Moss as string },
  { name: "Ledger", avatar: "knot", color: PIGMENT.Indigo as string },
  { name: "Quill", avatar: "wave", color: PIGMENT.Rose as string },
];

/**
 * The top of the rail, drawn to the app's own measurements: the window's
 * buttons over the corner, the 36px the rail leaves for them, then the search
 * field and a crew. `now` is what ships; `mark` is a mark in the title bar
 * and no brand row under it.
 */
function RailTop({ mark }: { mark?: React.ReactNode }) {
  return (
    <div className="bm-railtop">
      <div className="bm-railtop__band">
        <span className="bm-lights" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
        {mark && <span className="bm-railtop__mark">{mark}</span>}
        {mark && <span className="bm-plus">+</span>}
      </div>
      {!mark && (
        <div className="bm-railtop__brand">
          <span className="bm-railtop__wordmark">GUACA</span>
          <span className="bm-plus">+</span>
        </div>
      )}
      <div className="bm-railtop__search">Search</div>
      <div className="bm-railtop__crew">Research crew</div>
      {CREW.map((a) => (
        <div key={a.name} className="bm-railtop__row">
          <AgentAvatar avatar={a.avatar} color={a.color} size="xs" seed={a.name} mood="idle" />
          <span>{a.name}</span>
        </div>
      ))}
    </div>
  );
}

const HAPPENINGS: [Happening, string][] = [
  ["message", "A message lands"],
  ["working", "Agents working"],
  ["waiting", "Waiting on you"],
  ["asleep", "Quiet for an hour"],
];

function Section({ dir, index }: { dir: Direction; index: number }) {
  const [cast, setCast] = useState<Cast>("cut");
  const [state, setState] = useState<Happening>("none");
  const views = useRef(new Set<View>());
  const flags = useRef<Flags>(fresh());
  const start = useRef(now());
  const shown = useRef(0);
  const said = useRef(Number.NaN);
  /* Reduced motion skips the performance: the mark is already condensed and
     at rest, and the name appears on a hover rather than folding out. */
  const at = () => now() - start.current + (prefersReducedMotion() ? HOLD + 60 : 0);

  useEffect(() => {
    let frame = 0;
    let last = now();
    const tick = () => {
      frame = requestAnimationFrame(tick);
      const still = prefersReducedMotion();
      const clock = now();
      const dt = Math.min(0.05, clock - last);
      last = clock;
      const t = at();
      const f = flags.current;
      if (still) f.condensedAt = 0;
      else if (t >= HOLD && f.condensedAt === Number.POSITIVE_INFINITY) f.condensedAt = t;
      const scene = dir.at(t, f);
      /* The launch writes the name at its own pace; after that the name eases
         toward in or out, so a condense and a hover are a fold, not a cut. */
      shown.current =
        still || t < HOLD - 1
          ? scene.name
          : shown.current + (scene.name - shown.current) * Math.min(1, dt * 12);
      if (scene.says && scene.says.at !== said.current) {
        said.current = scene.says.at;
        for (const v of views.current) v.say(scene.says.text);
      }
      for (const v of views.current) {
        scene.cues.forEach((cue, i) => {
          const ref = v.cues[i];
          if (ref) ref.current = cue;
        });
        v.name(shown.current);
      }
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [dir]);

  const hover = (on: boolean) => {
    const f = flags.current;
    if (on && !f.hover) {
      /* Still, a visit lands already finished: the name is out and the
         shapeshifter has become the next one, with nothing in between. */
      f.hoverAt = at() - (prefersReducedMotion() ? 1 : 0);
      f.visits += 1;
    }
    f.hover = on;
  };
  const replay = () => {
    start.current = now();
    flags.current = fresh();
    shown.current = 0;
    said.current = Number.NaN;
    setState("none");
  };
  const condense = () => {
    const f = flags.current;
    if (f.condensedAt === Number.POSITIVE_INFINITY) f.condensedAt = Math.max(at(), 4);
  };
  const happen = (h: Happening) => {
    flags.current.happening = h;
    flags.current.happeningAt = at();
    setState(h);
  };

  return (
    <section id={dir.key}>
      <header>
        <span>GUACA / BRAND MARK</span>
        <span>
          {String(index + 2).padStart(2, "0")}&nbsp;&nbsp;&nbsp;{dir.name}
        </span>
      </header>
      <div className="bm-stage">
        <Mark dir={dir} px={120} type={76} bubble={40} cast={cast} views={views} hover={hover} />
      </div>
      <div className="bm-controls">
        <button type="button" onClick={replay}>
          Replay the launch
        </button>
        <button type="button" onClick={condense}>
          Condense now
        </button>
        <span className="bm-controls__rule" />
        {HAPPENINGS.map(([h, label]) => (
          <button key={h} type="button" aria-pressed={state === h} onClick={() => happen(state === h ? "none" : h)}>
            {label}
          </button>
        ))}
        <span className="bm-controls__rule" />
        {(["cut", "drawn"] as Cast[]).map((c) => (
          <button key={c} type="button" aria-pressed={cast === c} onClick={() => setCast(c)}>
            {c === "cut" ? "Cut cast" : "Drawn cast"}
          </button>
        ))}
      </div>
      <div className="bm-foot">
        <div className="bm-why">
          <h2>{dir.title}</h2>
          <p>{dir.why}</p>
          <ol className="bm-beats">
            {dir.beats.map(([t, what]) => (
              <li key={t}>
                <span>{t}s</span>
                {what}
              </li>
            ))}
          </ol>
        </div>
        <div className="bm-in-rail">
          <h3>IN THE TITLE BAR, SHOWN AT 2X</h3>
          <div className="bm-zoom">
            <RailTop mark={<Mark dir={dir} px={28} type={17} bubble={12} cast={cast} views={views} hover={hover} />} />
          </div>
        </div>
      </div>
    </section>
  );
}

/** Now and proposed, side by side at 2x, with the pair standing in for all three. */
function Corner() {
  const views = useRef(new Set<View>());
  useEffect(() => {
    let frame = 0;
    const tick = () => {
      frame = requestAnimationFrame(tick);
      const scene = pair.at(60, { ...fresh(), condensedAt: 0 });
      for (const v of views.current) {
        scene.cues.forEach((cue, i) => {
          const ref = v.cues[i];
          if (ref) ref.current = cue;
        });
        v.name(0);
      }
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, []);
  return (
    <section id="corner">
      <header>
        <span>GUACA / BRAND MARK</span>
        <span>01&nbsp;&nbsp;&nbsp;The corner</span>
      </header>
      <div className="bm-corner">
        <div className="bm-why">
          <h2>The name gives the corner back.</h2>
          <p>
            The rail spends its first 81 pixels on the window's buttons and a wordmark. The buttons
            need 36 of them, and the strip beside the buttons is empty. So the mark moves up into that
            strip with the plus beside it, the brand row goes, and the crew starts 45 pixels higher.
          </p>
          <p>
            The full name is shown once, when the app opens. After a few seconds it folds away and only
            the cast is left; pointing at the corner brings the name back. The type is deliberately
            plain: Instrument Sans at 600, sentence case, no tracking. The characters are the brand, and
            the name is a label for them.
          </p>
          <p>
            Every creature on this page is the app's own, drawn by src/avatars. The motion honors reduced
            motion the way the rail does: a still, condensed mark.
          </p>
        </div>
        <div className="bm-corner__pair">
          <figure>
            <figcaption>Now</figcaption>
            <div className="bm-zoom">
              <RailTop />
            </div>
          </figure>
          <figure>
            <figcaption>Proposed, condensed</figcaption>
            <div className="bm-zoom">
              <RailTop
                mark={<Mark dir={pair} px={28} type={17} bubble={12} cast="cut" views={views} hover={() => {}} />}
              />
            </div>
          </figure>
        </div>
      </div>
    </section>
  );
}

function Page() {
  return (
    <>
      <Corner />
      {DIRECTIONS.map((dir, i) => (
        <Section key={dir.key} dir={dir} index={i} />
      ))}
    </>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(<Page />);
