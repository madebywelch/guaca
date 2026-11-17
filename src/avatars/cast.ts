import { createContext } from "react";

import { type Cast, DEFAULT_PREFS } from "../lib/prefs";

export const CastContext = createContext<Cast>(DEFAULT_PREFS.cast);
