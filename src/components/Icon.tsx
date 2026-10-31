/**
 * The few marks the interface draws instead of typing.
 *
 * `«`, `»`, `‹`, `⚙` and `⋯` were typed characters in buttons, each at whatever
 * weight and baseline the font happened to give it, beside a footer drawn in
 * one stroke. A control's mark is drawn here, in the stroke the footer set,
 * and sized by `.icon` so every one of them sits the same in a button.
 */
export type IconName = "chevron-left" | "chevron-right" | "more" | "plus";

const PATHS: Record<Exclude<IconName, "more">, string> = {
  "chevron-left": "M14.5 6l-6 6 6 6",
  "chevron-right": "M9.5 6l6 6-6 6",
  plus: "M12 5v14M5 12h14",
};

export function Icon({ name }: { name: IconName }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" data-icon={name}>
      {name === "more" ? (
        // Three dots are filled, not stroked: at the stroke's width a dot is
        // a speck, and the one glyph that stands for "everything else" has to
        // be found before it can be pressed.
        <>
          <circle className="icon__dot" cx="5.5" cy="12" r="1.7" />
          <circle className="icon__dot" cx="12" cy="12" r="1.7" />
          <circle className="icon__dot" cx="18.5" cy="12" r="1.7" />
        </>
      ) : (
        <path d={PATHS[name]} />
      )}
    </svg>
  );
}
