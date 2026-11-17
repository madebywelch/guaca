export type IconName =
  | "chevron-left"
  | "chevron-right"
  | "more"
  | "plus"
  | "search"
  | "people"
  | "settings"
  | "arrow-up";

const PATHS: Record<Exclude<IconName, "more">, string> = {
  "chevron-left": "M14.5 6l-6 6 6 6",
  "chevron-right": "M9.5 6l6 6-6 6",
  plus: "M12 5v14M5 12h14",
  search: "M10.5 4.5a6 6 0 1 0 0 12a6 6 0 1 0 0-12M15 15l4.5 4.5",
  people:
    "M9 5.5a3 3 0 1 0 0 6a3 3 0 1 0 0-6M3.5 19c.6-3 2.8-4.8 5.5-4.8s4.9 1.8 5.5 4.8M16 6a2.6 2.6 0 0 1 0 5.2M17.5 14.4c1.7.5 2.8 2 3 4.6",
  settings:
    "M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6M12 3v2.2M12 18.8V21M3 12h2.2M18.8 12H21M5.6 5.6l1.6 1.6M16.8 16.8l1.6 1.6M5.6 18.4l1.6-1.6M16.8 7.2l1.6-1.6",
  "arrow-up": "M12 19V5M6 11l6-6 6 6",
};

export function Icon({ name }: { name: IconName }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" data-icon={name}>
      {name === "more" ? (
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
