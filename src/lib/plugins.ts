/**
 * How each plugin is drawn. Its mark, and nothing else.
 *
 * Everything a plugin *is* — its name, what it offers, where it signs in — comes
 * from Rust, because Rust is what dials the endpoint. A second copy of that here
 * would be a second place for it to be wrong. What is left on this side is the
 * part the runtime has no use for: a logo and a colour.
 *
 * The marks are Simple Icons (https://simpleicons.org), CC0 1.0, copied in as
 * path data rather than pulled as a dependency: a handful of glyphs do not
 * justify a package carrying three thousand. Trademarks belong to their owners.
 *
 * AgentMail is the exception and is drawn here, because it is not in that set.
 * A generic envelope rather than an approximation of their logo: a mark nobody
 * can check against the original is a mark that quietly becomes wrong.
 */

import type { PluginKind } from "./types";

export interface Brand {
  /** The `d` of a single path on a 24x24 viewBox, verbatim from Simple Icons. */
  path: string;
  /** The brand's own colour, which is what makes the row scannable. */
  color: string;
}

export const BRANDS: Record<PluginKind, Brand> = {
  neon: {
    path: "M24 0V24l-9.365-8.045V24H0V0ZM2.942 21.087h8.751V9.563l9.365 8.204V2.919L2.942 2.914Z",
    color: "#34d59a",
  },
  cloudflare: {
    path: "M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268zm2.7568-5.5634c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727",
    color: "#f38020",
  },
  linear: {
    path: "M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z",
    color: "#5e6ad2",
  },
  stripe: {
    path: "M13.976 9.15c-2.172-.806-3.356-1.426-3.356-2.409 0-.831.683-1.305 1.901-1.305 2.227 0 4.515.858 6.09 1.631l.89-5.494C18.252.975 15.697 0 12.165 0 9.667 0 7.589.654 6.104 1.872 4.56 3.147 3.757 4.992 3.757 7.218c0 4.039 2.467 5.76 6.476 7.219 2.585.92 3.445 1.574 3.445 2.583 0 .98-.84 1.545-2.354 1.545-1.875 0-4.965-.921-6.99-2.109l-.9 5.555C5.175 22.99 8.385 24 11.714 24c2.641 0 4.843-.624 6.328-1.813 1.664-1.305 2.525-3.236 2.525-5.732 0-4.128-2.524-5.851-6.594-7.305h.003z",
    color: "#635bff",
  },
  agentmail: {
    path: "M21.9 4.1H2.1L12 10.9ZM1.2 6.4v11.9a1.6 1.6 0 0 0 1.6 1.6h18.4a1.6 1.6 0 0 0 1.6-1.6V6.4L12 13.5Z",
    color: "#0a0a0a",
  },
};

/**
 * The host a plugin's calls go to, for the line under its name.
 *
 * Read off the endpoint the backend reported rather than written down again
 * here: what an operator is about to authorise has to be the address that is
 * actually dialled, not a second copy of it that can drift.
 */
export function hostOf(endpoint: string): string {
  try {
    return new URL(endpoint).host;
  } catch {
    return endpoint;
  }
}
