/**
 * The panes Settings draws: the id a route or a quick action names, and the
 * words the pane is drawn with.
 *
 * The id is what crosses the wire, because it is one lowercase word a page can
 * send and the runtime can keep. The label is what a person reads and what an
 * agent is told, so the two are one table rather than three — the dialog's nav,
 * the sentence a quick action's tooltip shows, and the sentence an agent reads
 * about the screen. `manual.test.ts` holds the agent's copy in
 * `domain/view.rs` to these words, the way it holds `LIMITS` to `config.rs`.
 */

export const SECTIONS = [
  "general",
  "workspace",
  "provider",
  "limits",
  "machines",
  "account",
  "appearance",
  "notifications",
  "shortcuts",
  "skills",
  "compost",
  "about",
] as const;

export type Section = (typeof SECTIONS)[number];

export const SECTION_LABELS: Record<Section, string> = {
  general: "General",
  workspace: "Host",
  provider: "Provider",
  limits: "Limits",
  machines: "Machines",
  account: "Account",
  appearance: "Appearance",
  notifications: "Notifications",
  shortcuts: "Shortcuts",
  skills: "Skills",
  compost: "Compost",
  about: "About",
};

/** A pane name from elsewhere (a quick action, a route), if it is one. */
export function asSection(name: string | null | undefined): Section | undefined {
  return SECTIONS.find((section) => section === name);
}

/**
 * What Settings calls a pane, for a section id that came from anywhere.
 *
 * A page sends an id and a stored quick action carries one, so an id this
 * build no longer draws arrives here too. It is named back as it came rather
 * than swallowed: a word somebody wrote is still a place they meant.
 */
export function paneName(section: string | null | undefined): string | null {
  if (!section) return null;
  return SECTION_LABELS[section as Section] ?? section;
}
