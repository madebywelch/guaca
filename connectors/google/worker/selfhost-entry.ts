import app from "./index";

export default {
  fetch(request: Request, env: Env, execution: ExecutionContext) {
    if (!env.SELFHOST_BEARER || env.SELFHOST_BEARER.length < 32 || !env.GOOGLE_CLIENT_ID || !env.GOOGLE_CLIENT_SECRET) {
      return new Response("The connector is not configured.", { status: 503 });
    }
    return app.fetch(request, env, execution);
  },
};
