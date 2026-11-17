export const SECTIONS = [
  "general",
  "workspace",
  "provider",
  "limits",
  "permissions",
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
  permissions: "Permissions",
  machines: "Machines",
  account: "Account",
  appearance: "Appearance",
  notifications: "Notifications",
  shortcuts: "Shortcuts",
  skills: "Skills",
  compost: "Compost",
  about: "About",
};

export function asSection(name: string | null | undefined): Section | undefined {
  return SECTIONS.find((section) => section === name);
}

export function paneName(section: string | null | undefined): string | null {
  if (!section) return null;
  return SECTION_LABELS[section as Section] ?? section;
}
