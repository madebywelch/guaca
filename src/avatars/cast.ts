import { createContext } from "react";

import { type Cast, DEFAULT_PREFS } from "../lib/prefs";

/**
 * Which cast the avatars under it are drawn in.
 *
 * A context rather than a read of the store: an avatar is drawn inside fifteen
 * surfaces, and every one of their suites that stands a store in would
 * otherwise have to know that a preference about drawing exists. The root in
 * `main.tsx` provides the operator's choice, and anything drawn outside it, a
 * test or a preview, gets the default.
 */
export const CastContext = createContext<Cast>(DEFAULT_PREFS.cast);
