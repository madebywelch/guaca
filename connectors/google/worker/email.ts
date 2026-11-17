export const CODE_TTL_MINUTES = 10;

function delivers(env: Env): boolean {
  return env.BETTER_AUTH_URL.startsWith("https://");
}

export async function sendSignInCode(env: Env, to: string, code: string): Promise<void> {
  if (!delivers(env)) {
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
    html: [
      '<div style="font-family:system-ui,sans-serif;font-size:16px;line-height:1.5">',
      `<p>Your Guaca sign-in code is <strong style="font-size:22px;letter-spacing:0.2em">${code}</strong></p>`,
      `<p>It expires in ${CODE_TTL_MINUTES} minutes and can be used once.</p>`,
      "<p>If you did not ask to sign in, you can ignore this. Nobody can use the code without it, and no account was changed.</p>",
      "</div>",
    ].join(""),
  });
}
