/**
 * The services a credential can be added for, by name.
 *
 * Adding a credential used to be five empty boxes: a service, an account, a
 * variable name, the token, and a note. Four of those are things the operator
 * has to guess and only one is something they actually have. Picking GitHub
 * from a list answers all four, because the variable a GitHub token belongs in
 * is not a preference: it is `GITHUB_TOKEN`, the same on every machine.
 *
 * The brand marks are Simple Icons (https://simpleicons.org), CC0 1.0, copied
 * in as path data rather than pulled as a dependency: eleven glyphs do not
 * justify a package carrying three thousand. Trademarks belong to their owners.
 *
 * Lives on this side of the IPC boundary on purpose. The backend stores
 * whatever service and variable it is handed and has no opinion about which
 * ones exist, so a catalog in Rust would be a second copy of a list that is
 * only ever used to fill in a form.
 */
export interface CatalogEntry {
  /** Shown on the tile. */
  service: string;
  /** The variable the agent will find it in. Conventional, not invented. */
  envVar: string;
  /** Where to get one. Shown under the single field the operator has to fill. */
  where: string;
  /**
   * The brand's real mark, as the `d` of a single path on a 24x24 viewBox.
   *
   * Taken verbatim from Simple Icons rather than drawn here, because a logo
   * redrawn by eye is a wrong logo. Absent for the few brands Simple Icons does
   * not carry, and those fall back to `mark`.
   */
  path?: string;
  /** The initial, used when there is no icon for the brand. */
  mark: string;
  /** The brand's own colour, which is what makes the grid scannable. */
  color: string;
  /** What to tell the agent about it, when the service needs saying. */
  note?: string;
}

export const CATALOG: CatalogEntry[] = [
  {
    service: "GitHub",
    envVar: "GITHUB_TOKEN",
    where: "github.com → Settings → Developer settings → Personal access tokens",
    path: "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
    mark: "G",
    color: "#181717",
  },
  {
    service: "Cloudflare",
    envVar: "CLOUDFLARE_API_TOKEN",
    where: "dash.cloudflare.com → My Profile → API Tokens",
    path: "M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268zm2.7568-5.5634c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727",
    mark: "C",
    color: "#f38020",
  },
  {
    service: "Linear",
    envVar: "LINEAR_API_KEY",
    where: "linear.app → Settings → Security & access → Personal API keys",
    path: "M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z",
    mark: "L",
    color: "#5e6ad2",
  },
  {
    service: "Sentry",
    envVar: "SENTRY_AUTH_TOKEN",
    where: "sentry.io → Settings → Auth Tokens",
    path: "M13.91 2.505c-.873-1.448-2.972-1.448-3.844 0L6.904 7.92a15.478 15.478 0 0 1 8.53 12.811h-2.221A13.301 13.301 0 0 0 5.784 9.814l-2.926 5.06a7.65 7.65 0 0 1 4.435 5.848H2.194a.365.365 0 0 1-.298-.534l1.413-2.402a5.16 5.16 0 0 0-1.614-.913L.296 19.275a2.182 2.182 0 0 0 .812 2.999 2.24 2.24 0 0 0 1.086.288h6.983a9.322 9.322 0 0 0-3.845-8.318l1.11-1.922a11.47 11.47 0 0 1 4.95 10.24h5.915a17.242 17.242 0 0 0-7.885-15.28l2.244-3.845a.37.37 0 0 1 .504-.13c.255.14 9.75 16.708 9.928 16.9a.365.365 0 0 1-.327.543h-2.287c.029.612.029 1.223 0 1.831h2.297a2.206 2.206 0 0 0 1.922-3.31z",
    mark: "S",
    color: "#362d59",
  },
  {
    service: "Vercel",
    envVar: "VERCEL_TOKEN",
    where: "vercel.com → Settings → Tokens",
    // The one mark that is exactly itself at this size.
    path: "m12 1.608 12 20.784H0Z",
    mark: "▲",
    color: "#000000",
  },
  {
    service: "OpenAI",
    envVar: "OPENAI_API_KEY",
    where: "platform.openai.com → API keys",
    mark: "O",
    color: "#10a37f",
  },
  {
    service: "Slack",
    envVar: "SLACK_BOT_TOKEN",
    where: "api.slack.com → Your apps → OAuth & Permissions",
    mark: "S",
    color: "#4a154b",
  },
  {
    service: "Stripe",
    envVar: "STRIPE_API_KEY",
    where: "dashboard.stripe.com → Developers → API keys",
    path: "M13.976 9.15c-2.172-.806-3.356-1.426-3.356-2.409 0-.831.683-1.305 1.901-1.305 2.227 0 4.515.858 6.09 1.631l.89-5.494C18.252.975 15.697 0 12.165 0 9.667 0 7.589.654 6.104 1.872 4.56 3.147 3.757 4.992 3.757 7.218c0 4.039 2.467 5.76 6.476 7.219 2.585.92 3.445 1.574 3.445 2.583 0 .98-.84 1.545-2.354 1.545-1.875 0-4.965-.921-6.99-2.109l-.9 5.555C5.175 22.99 8.385 24 11.714 24c2.641 0 4.843-.624 6.328-1.813 1.664-1.305 2.525-3.236 2.525-5.732 0-4.128-2.524-5.851-6.594-7.305h.003z",
    mark: "S",
    color: "#635bff",
    note: "money moves through this one",
  },
  {
    service: "Notion",
    envVar: "NOTION_API_KEY",
    where: "notion.so/my-integrations",
    path: "M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.374.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.887l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952L12.21 19s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM1.936 1.035l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.934.653.934 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.447-1.632z",
    mark: "N",
    color: "#000000",
  },
  {
    service: "Resend",
    envVar: "RESEND_API_KEY",
    where: "resend.com → API Keys",
    path: "M14.679 0c4.648 0 7.413 2.765 7.413 6.434s-2.765 6.434-7.413 6.434H12.33L24 24h-8.245l-8.88-8.44c-.636-.588-.93-1.273-.93-1.86 0-.831.587-1.565 1.713-1.883l4.574-1.224c1.737-.465 2.936-1.81 2.936-3.572 0-2.153-1.761-3.4-3.939-3.4H0V0z",
    mark: "R",
    color: "#000000",
    note: "sends real email",
  },
  {
    service: "Tavily",
    envVar: "TAVILY_API_KEY",
    where: "tavily.com → API Keys",
    mark: "T",
    color: "#1d6ae5",
  },
];

/** The catalog entry for a stored credential, when it came from the list. */
export function entryFor(service: string): CatalogEntry | undefined {
  return CATALOG.find((entry) => entry.service === service);
}
