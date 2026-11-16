type Phase =
  | "notification.clicked"
  | "notification.missing"
  | "notification.remembered"
  | "reveal.agent"
  | "select.requested"
  | "select.invalid"
  | "select.committed";

const KEPT = 256;
const rows: { phase: Phase; id: number; value: number; ms: number }[] = [];

export function opaque(text: string): number {
  let hash = 2166136261;
  for (const byte of new TextEncoder().encode(text)) hash = Math.imul(hash ^ byte, 16777619) >>> 0;
  return hash;
}

export function diagnostic(phase: Phase, id: string | number | null, value = 0): void {
  if (import.meta.env.MODE !== "probe") return;
  if (rows.length === KEPT) rows.shift();
  rows.push({
    phase,
    id: typeof id === "string" ? opaque(id) : (id ?? 0),
    value,
    ms: performance.now(),
  });
}

export function diagnosticRows() {
  return { ms: performance.now(), rows: rows.slice() };
}
