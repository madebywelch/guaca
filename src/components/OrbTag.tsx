import type { CSSProperties } from "react";
import { useCallback, useState } from "react";

interface Props {
  name: string;
  note: string;
  at: number;
}

export function OrbTag({ name, note, at }: Props) {
  return (
    <span className="orb__tag" aria-hidden="true" style={{ "--tag-y": `${at}px` } as CSSProperties}>
      <span className="orb__tag-name">{name}</span>
      <span className="orb__tag-note">{note}</span>
    </span>
  );
}

export function useOrbTag(): {
  at: number | null;
  open: (event: { currentTarget: HTMLElement }) => void;
  close: () => void;
} {
  const [at, setAt] = useState<number | null>(null);

  const open = useCallback((event: { currentTarget: HTMLElement }) => {
    const box = event.currentTarget.getBoundingClientRect();
    setAt(box.top + box.height / 2);
  }, []);

  const close = useCallback(() => setAt(null), []);

  return { at, open, close };
}
