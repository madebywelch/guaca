/**
 * The one email this origin sends.
 *
 * Cloudflare Email Sending, through the `EMAIL` binding rather than the REST
 * API: a Worker already has the binding, and an API token would be one more
 * secret to hold on the origin whose whole job is holding secrets. The sending
 * domain has to be onboarded first (`wrangler email sending enable guaca.bot`)
 * or the send is rejected. See README, "Signing in".
 *
 * Sent, not queued. The person is waiting on a form post, and a failure they
 * find out about now beats one they discover by watching an inbox that will
 * never fill.
 */

/** Kept in step with `emailOTP`'s `expiresIn` in `auth.ts`. */
export const CODE_TTL_MINUTES = 10;

/**
 * True when this deployment can actually deliver mail.
 *
 * Local development has no onboarded domain, so the code is printed to the
 * console instead and the flow can be exercised end to end without a mail
 * provider. Gated on the origin not being HTTPS, which production cannot be:
 * every OAuth callback on this origin is registered against `https://`, and
 * `build()` refuses to start without `BETTER_AUTH_URL`.
 */
function delivers(env: Env): boolean {
  return env.BETTER_AUTH_URL.startsWith("https://");
}

export async function sendSignInCode(env: Env, to: string, code: string): Promise<void> {
  if (!delivers(env)) {
    // The one place a live credential is written to a log, and it is the
    // reason the condition above is about the origin rather than a flag
    // someone could set in production by accident.
    console.log(`[dev] sign-in code for ${to}: ${code}`);
    return;
  }

  const from = env.EMAIL_FROM;
  if (!from) throw new Error("EMAIL_FROM is not set");

  const text = [
    `Your Guaca sign-in code is ${code}.`,
    "",
    `It expires in ${CODE_TTL_MINUTES} minutes and can be used once.`,
    "If you did not ask to sign in, you can ignore this. Nobody can use the",
    "code without it, and no account was changed.",
  ].join("\n");

  await env.EMAIL.send({
    to,
    from: { email: from, name: "Guaca" },
    subject: `${code} is your Guaca sign-in code`,
    text,
    // Deliberately close to the plain text. A sign-in code is read in two
    // seconds on a phone, and every image, button and tracking pixel a
    // template would add is another reason to land in spam.
    html: [
      '<div style="font-family:system-ui,sans-serif;font-size:16px;line-height:1.5">',
      `<p>Your Guaca sign-in code is <strong style="font-size:22px;letter-spacing:0.2em">${code}</strong></p>`,
      `<p>It expires in ${CODE_TTL_MINUTES} minutes and can be used once.</p>`,
      "<p>If you did not ask to sign in, you can ignore this. Nobody can use the code without it, and no account was changed.</p>",
      "</div>",
    ].join(""),
  });
}
