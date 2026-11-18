export const ICON_NAMES = [
  "agent",
  "alert",
  "arrow-right",
  "arrow-up",
  "artifact",
  "attach",
  "blocked",
  "calendar",
  "check",
  "chevron-down",
  "chevron-left",
  "chevron-right",
  "clock",
  "close",
  "command",
  "external",
  "file",
  "for-you",
  "gear",
  "group",
  "host",
  "lock",
  "message",
  "more",
  "pin",
  "plus",
  "return",
  "search",
  "shield",
  "stop",
  "swap",
  "undo",
] as const;

export type IconName = (typeof ICON_NAMES)[number];

export type IconSize = 16 | 18;

export const PATHS: Record<IconName, string> = {
  agent:
    "M12 4A4 4 0 1 0 12 12A4 4 0 1 0 12 4ZM5 20C5.8 16.6 8.6 14.5 12 14.5C15.4 14.5 18.2 16.6 19 20",
  alert: "M12 3A9 9 0 1 0 12 21A9 9 0 1 0 12 3ZM12 7.5V12.5M12 16.5V16.51",
  "arrow-right": "M5 12H19M13 6L19 12L13 18",
  "arrow-up": "M12 19V5M6 11L12 5L18 11",
  artifact:
    "M14 3H7A2 2 0 0 0 5 5V19A2 2 0 0 0 7 21H17A2 2 0 0 0 19 19V8ZM14 3V8H19M9 13H15M9 17H13",
  attach:
    "M15.5 7.5L8.6 14.4A2 2 0 0 0 11.4 17.2L18.3 10.3A4 4 0 0 0 12.6 4.6L5.7 11.5A6 6 0 0 0 14.2 20L20 14.2",
  blocked: "M12 3A9 9 0 1 0 12 21A9 9 0 1 0 12 3ZM5.6 5.6L18.4 18.4",
  calendar:
    "M6 5H18A2 2 0 0 1 20 7V18A2 2 0 0 1 18 20H6A2 2 0 0 1 4 18V7A2 2 0 0 1 6 5ZM4 10H20M8 3V7M16 3V7",
  check: "M5 12.5L10 17.5L19 7",
  "chevron-down": "M6 9L12 15L18 9",
  "chevron-left": "M15 6L9 12L15 18",
  "chevron-right": "M9 6L15 12L9 18",
  clock: "M12 3A9 9 0 1 0 12 21A9 9 0 1 0 12 3ZM12 7V12L15.5 14",
  close: "M6 6L18 18M18 6L6 18",
  command:
    "M9 9H15V15H9ZM9 9V6A3 3 0 1 0 6 9H18A3 3 0 1 0 15 6V18A3 3 0 1 0 18 15H6A3 3 0 1 0 9 18Z",
  external: "M7 17L17 7M8 7H17V16",
  file: "M14 3H7A2 2 0 0 0 5 5V19A2 2 0 0 0 7 21H17A2 2 0 0 0 19 19V8ZM14 3V8H19",
  "for-you": "M4 13H8.5L10 16H14L15.5 13H20M7 4H17L20 13V18A2 2 0 0 1 18 20H6A2 2 0 0 1 4 18V13Z",
  gear: "M10.38 4.98L10.68 2.59L13.32 2.59L13.62 4.98A7.2 7.2 0 0 1 15.82 5.89L17.72 4.41L19.59 6.28L18.11 8.18A7.2 7.2 0 0 1 19.02 10.38L21.41 10.68L21.41 13.32L19.02 13.62A7.2 7.2 0 0 1 18.11 15.82L19.59 17.72L17.72 19.59L15.82 18.11A7.2 7.2 0 0 1 13.62 19.02L13.32 21.41L10.68 21.41L10.38 19.02A7.2 7.2 0 0 1 8.18 18.11L6.28 19.59L4.41 17.72L5.89 15.82A7.2 7.2 0 0 1 4.98 13.62L2.59 13.32L2.59 10.68L4.98 10.38A7.2 7.2 0 0 1 5.89 8.18L4.41 6.28L6.28 4.41L8.18 5.89A7.2 7.2 0 0 1 10.38 4.98ZM12 9A3 3 0 1 0 12 15A3 3 0 1 0 12 9Z",
  group:
    "M9 5.5A3 3 0 1 0 9 11.5A3 3 0 1 0 9 5.5ZM3.5 19C4.1 16 6.3 14.2 9 14.2C11.7 14.2 13.9 16 14.5 19M16 6A2.6 2.6 0 0 1 16 11.2M17.5 14.4C19.2 14.9 20.3 16.4 20.5 19",
  host: "M6 4H18A2 2 0 0 1 20 6V9A2 2 0 0 1 18 11H6A2 2 0 0 1 4 9V6A2 2 0 0 1 6 4ZM6 13H18A2 2 0 0 1 20 15V18A2 2 0 0 1 18 20H6A2 2 0 0 1 4 18V15A2 2 0 0 1 6 13ZM8 7.5H8.01M8 16.5H8.01",
  lock: "M8 11V8A4 4 0 0 1 16 8V11M7 11H17A2 2 0 0 1 19 13V18A2 2 0 0 1 17 20H7A2 2 0 0 1 5 18V13A2 2 0 0 1 7 11Z",
  message: "M7 5H17A3 3 0 0 1 20 8V13A3 3 0 0 1 17 16H11L7 19.5V16A3 3 0 0 1 4 13V8A3 3 0 0 1 7 5Z",
  more: "M4 12A1.5 1.5 0 1 0 7 12A1.5 1.5 0 1 0 4 12ZM10.5 12A1.5 1.5 0 1 0 13.5 12A1.5 1.5 0 1 0 10.5 12ZM17 12A1.5 1.5 0 1 0 20 12A1.5 1.5 0 1 0 17 12Z",
  pin: "M12 16V21M9 3H15M10 3V9L6.5 13V16H17.5V13L14 9V3",
  plus: "M12 5V19M5 12H19",
  return: "M19 5V11A3 3 0 0 1 16 14H5M9 10L5 14L9 18",
  search: "M11 4A7 7 0 1 0 11 18A7 7 0 1 0 11 4ZM16 16L20 20",
  shield: "M12 3L19 6V11.5C19 15.5 16 18.8 12 20.5C8 18.8 5 15.5 5 11.5V6Z",
  stop: "M8 6H16A2 2 0 0 1 18 8V16A2 2 0 0 1 16 18H8A2 2 0 0 1 6 16V8A2 2 0 0 1 8 6Z",
  swap: "M4 8H18M14 4L18 8L14 12M20 16H6M10 12L6 16L10 20",
  undo: "M9 14L4 9L9 4M4 9H14.5A5.5 5.5 0 0 1 14.5 20H11",
};

export function Icon({ name, size = 18 }: { name: IconName; size?: IconSize }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" data-icon={name} data-size={size}>
      <path d={PATHS[name]} />
    </svg>
  );
}
