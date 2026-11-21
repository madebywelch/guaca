import type { ReactNode } from "react";
import { createPortal } from "react-dom";

export function Scrim({ children }: { children: ReactNode }) {
  return createPortal(<div className="scrim">{children}</div>, document.body);
}
