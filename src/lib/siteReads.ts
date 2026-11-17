type Read = (path: string) => string;

interface SiteRead {
  path: string;
  pattern?: RegExp;
  literal?: true;
}

export const SITE_READS: Record<string, SiteRead> = {
  "silhouette.ts": { path: "src/avatars/silhouette.ts" },
  "form.ts": { path: "src/avatars/form.ts" },
  "eyes.ts": { path: "src/avatars/eyes.ts" },
  "catalog.ts": { path: "src/avatars/catalog.ts" },
  "drawn.ts": { path: "src/avatars/drawn.ts" },
  "Skin.tsx": { path: "src/avatars/Skin.tsx" },
  "silhouette.test.ts": { path: "src/avatars/silhouette.test.ts" },
  "catalog.test.ts": { path: "src/avatars/catalog.test.ts" },
  "drawn.test.ts": { path: "src/avatars/drawn.test.ts" },
  "eyes.test.ts": { path: "src/avatars/eyes.test.ts" },
  "form.test.ts": { path: "src/avatars/form.test.ts" },
  "form.test.ts icon": {
    path: "src/avatars/form.test.ts",
    pattern: /\n(describe\("the icon's bodies"[\s\S]*?\n\}\);\n)/,
  },
  "moods.ts": { path: "src/avatars/moods.ts", pattern: /^([\s\S]*?)\nexport const PLEASED_MS/ },
  "moods.test.ts table": {
    path: "src/avatars/moods.test.ts",
    pattern: /\n(describe\("the table"[\s\S]*?\n\}\);\n)/,
  },
  HINT: {
    path: "src/avatars/CutArt.tsx",
    pattern: /\nconst HINT = (\{[^}]*\}) as const;/,
    literal: true,
  },
  AVATAR_PX: {
    path: "src/avatars/frame.ts",
    pattern: /\nexport const AVATAR_PX\b[^=]*= (\{[^}]*\});/,
    literal: true,
  },
  KNOCK: {
    path: "src/avatars/frame.ts",
    pattern: /\nexport const KNOCK\b[^=]*= (\{[\s\S]*?\n\});/,
    literal: true,
  },
  AIM: {
    path: "src/avatars/eyes.ts",
    pattern: /\nexport const AIM = (\{[^}]*\}) as const;/,
    literal: true,
  },
  "light tokens": { path: "src/styles.css", pattern: /\n:root \{\n([^{}]*)\n\}\n/ },
  "dark tokens": {
    path: "src/styles.css",
    pattern: /\n:root\[data-surface="dark"\] \{\n([^{}]*)\n\}\n/,
  },
  "avatar styles": {
    path: "src/styles.css",
    pattern: /\n(\.avatar \{\n[\s\S]*?)\n\.avatar__says \{\n/,
  },
  SCOPE: { path: "src-tauri/src/account.rs", pattern: /\npub const SCOPE: &str = "([^"]+)";/ },
  wordmark: { path: "design/wordmarks/eye-contact.svg" },
  "install.sh": { path: "deploy/box/install.sh" },
  "guacad.service": { path: "deploy/guacad.service" },
  "docker-compose.yml": { path: "docker-compose.yml" },
  "README.md": { path: "README.md" },
  "HOSTING.md": { path: "docs/HOSTING.md" },
  "RELEASING.md": { path: "docs/RELEASING.md" },
};

function entry(name: string): SiteRead {
  const read = SITE_READS[name];
  if (!read) {
    throw new Error(
      `guaca.bot asked for "${name}", which SITE_READS in Guaca's src/lib/siteReads.ts does not list`,
    );
  }
  return read;
}

export function siteText(name: string, read: Read): string {
  const { path, pattern } = entry(name);
  const source = read(path);
  if (!pattern) return source;
  const found = pattern.exec(source)?.[1];
  if (found === undefined) {
    throw new Error(
      `${path} no longer has "${name}" in the shape guaca.bot reads, ${pattern}. ` +
        "If it moved or changed shape, point its entry in src/lib/siteReads.ts at where it is now.",
    );
  }
  return found;
}

export function siteValue(name: string, read: Read): unknown {
  if (!entry(name).literal) throw new Error(`"${name}" is text, not a literal value`);
  const text = siteText(name, read);
  return JSON.parse(text.replace(/(\w+):/g, '"$1":').replace(/,(\s*\})/g, "$1"));
}
