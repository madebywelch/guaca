// Real Chromium + real guacad, with an offline model. No account or API key.
// GUACAD=/path/to/guacad CHROME_BIN=/path/to/chrome node scripts/hosting-browser.mjs
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, rm, readFile } from "node:fs/promises";
import http from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";

const scratch = await mkdtemp(path.join(tmpdir(), "guaca-browser-"));
const token = "offline-browser-test-token";
const children = [];
let chromeSocket;
let daemon;
let modelResponse;
let requests = 0;
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(test, description, timeout = 20000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await test()) return;
    await delay(100);
  }
  throw new Error(`Timed out: ${description}`);
}
const model = http.createServer(async (req, res) => {
  for await (const _chunk of req) {
    /* consume request */
  }
  if (req.method !== "POST") {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ data: [{ id: "offline-test" }] }));
    return;
  }
  requests++;
  res.writeHead(200, { "content-type": "text/event-stream" });
  modelResponse = res;
  await delay(150);
  if (!res.destroyed) chunk("Working offline. ");
});
function chunk(text) {
  modelResponse.write(
    `data: ${JSON.stringify({ choices: [{ index: 0, delta: { content: text }, finish_reason: null }] })}\n\n`,
  );
}
function finish(text) {
  chunk(text);
  modelResponse.end(
    `data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] })}\n\ndata: [DONE]\n\n`,
  );
  modelResponse = undefined;
}
async function listen(server, port = 0) {
  server.listen(port, "127.0.0.1");
  await once(server, "listening");
  return server.address().port;
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const ended = once(child, "exit");
  child.kill("SIGTERM");
  const timer = setTimeout(() => child.kill("SIGKILL"), 3000);
  await ended;
  clearTimeout(timer);
}
try {
  const modelPort = await listen(model);
  const reservation = http.createServer();
  const port = await listen(reservation);
  await new Promise((resolve) => reservation.close(resolve));
  const base = `http://127.0.0.1:${port}`;
  const env = {
    PATH: process.env.PATH,
    HOME: path.join(scratch, "home"),
    GUACA_ROOT: path.join(scratch, "workspace"),
    GUACA_BIND: `127.0.0.1:${port}`,
    GUACA_TOKEN: token,
    GUACA_WEB: path.resolve("dist"),
    GUAC_LOG: "warn",
  };
  await mkdir(env.HOME);
  const launch = () => {
    daemon = spawn(process.env.GUACAD ?? path.resolve("src-tauri/target/debug/guacad"), [], {
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    children.push(daemon);
    daemon.stderr.on("data", (data) => process.stderr.write(data));
  };
  const healthy = async () => {
    try {
      return (await fetch(`${base}/health`)).ok;
    } catch {
      return false;
    }
  };
  const call = async (name, args = {}) => {
    const response = await fetch(`${base}/v1/call`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify({ name, args }),
    });
    const body = await response.json();
    assert.ok(!body.err, JSON.stringify(body));
    return body.ok;
  };
  launch();
  await until(healthy, "daemon starts");
  const group = await call("create_group", {
    draft: {
      name: "Browser test",
      apiKey: "offline-test-key",
      inference: {
        provider: "compatible",
        baseUrl: `http://127.0.0.1:${modelPort}/v1`,
        defaultModel: "offline-test",
      },
    },
  });
  const agent = await call("create_agent", {
    draft: {
      groupId: group.id,
      name: "Browser check",
      avatar: "avocado",
      color: "#7ab55c",
      model: "offline-test",
      systemPrompt: "Answer briefly.",
    },
  });
  const profile = path.join(scratch, "chrome");
  const chrome = spawn(
    process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-background-networking",
      "about:blank",
    ],
    { stdio: ["ignore", "ignore", "ignore"] },
  );
  children.push(chrome);
  let debuggerInfo;
  await until(async () => {
    try {
      debuggerInfo = (await readFile(path.join(profile, "DevToolsActivePort"), "utf8"))
        .trim()
        .split("\n");
      return true;
    } catch {
      return false;
    }
  }, "Chromium starts");
  chromeSocket = new WebSocket(`ws://127.0.0.1:${debuggerInfo[0]}${debuggerInfo[1]}`);
  await once(chromeSocket, "open");
  let sequence = 0;
  const pending = new Map();
  chromeSocket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (!message.id) return;
    const waiter = pending.get(message.id);
    if (!waiter) return;
    pending.delete(message.id);
    clearTimeout(waiter.timer);
    if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
    else waiter.resolve(message.result);
  });
  const cdp = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, 20000);
      pending.set(id, { resolve, reject, timer });
      chromeSocket.send(JSON.stringify({ id, method, params, sessionId }));
    });
  let tab;
  let session;
  const evaluate = async (expression) => {
    const answer = await cdp(
      "Runtime.evaluate",
      { expression, awaitPromise: true, returnByValue: true },
      session,
    );
    assert.ok(!answer.exceptionDetails, JSON.stringify(answer.exceptionDetails));
    return answer.result.value;
  };
  const openClient = async () => {
    tab = (await cdp("Target.createTarget", { url: `${base}/#token=${token}` })).targetId;
    session = (await cdp("Target.attachToTarget", { targetId: tab, flatten: true })).sessionId;
    await until(
      () => evaluate(`document.body.innerText.includes('Browser check')`),
      "real app renders its roster",
    );
    await evaluate(
      `[...document.querySelectorAll('.agent-row')].find(e=>e.textContent.includes('Browser check')).click()`,
    );
  };
  await openClient();
  assert.equal(await evaluate("location.hash"), "", "invitation is removed from the URL");
  await call("send_message", { agentId: agent.id, text: "Answer with the test result." });
  try {
    await until(() => Boolean(modelResponse), "model starts");
  } catch (error) {
    console.error(JSON.stringify(await call("channel_messages", { channelId: agent.id })));
    throw error;
  }
  await until(
    () => evaluate(`document.body.innerText.includes('Working offline.')`),
    "first client receives partial reply",
  );
  await cdp("Target.closeTarget", { targetId: tab });
  await openClient();
  await until(
    () => evaluate(`document.body.innerText.includes('Working offline.')`),
    "reconnected client restores partial reply",
  );
  await cdp("Target.closeTarget", { targetId: tab });
  finish("Finished while the client was closed.");
  await until(
    async () =>
      (await call("channel_messages", { channelId: agent.id })).some((m) =>
        m.parts.some((p) => p.text?.includes("Finished while")),
      ),
    "backend completes without a client",
  );
  await openClient();
  await until(
    () => evaluate(`document.body.innerText.includes('Finished while the client was closed.')`),
    "reconnected client renders persisted result",
  );
  console.log(
    "PASS: partial reply restored; backend finished with no client; transcript restored.",
  );

  // Exercise the real artifact route in Chromium, including its opaque origin.
  const artifact = await call("frame_artifact", {
    html: `<script>let isolated=false;try{parent.localStorage.getItem('guaca.workspace.token')}catch{isolated=true}guaca.answer({isolated});</script>`,
  });
  const artifactUrl = `${base}/v1/artifact/${artifact.id}?token=${artifact.ticket}`;
  await evaluate(
    `window.artifactResult=null;window.addEventListener('message',e=>{if(e.data?.guaca==='artifact-answer') window.artifactResult=JSON.parse(e.data.value)});const frame=document.createElement('iframe');frame.src=${JSON.stringify(artifactUrl)};document.body.append(frame);`,
  );
  await until(
    () => evaluate("window.artifactResult?.isolated === true"),
    "artifact executes its bridge but cannot read credentials",
  );
  console.log("PASS: artifact bridge works and its script cannot read workspace storage.");

  // Kill an active backend; reboot reports once and does not repeat the call.
  await call("send_message", { agentId: agent.id, text: "This request will be interrupted." });
  await until(() => Boolean(modelResponse), "second model call starts");
  const before = requests;
  const exited = once(daemon, "exit");
  daemon.kill("SIGKILL");
  await exited;
  modelResponse.destroy();
  modelResponse = undefined;
  launch();
  await until(healthy, "daemon restarts on the same volume");
  await until(
    () =>
      evaluate(
        `document.body.innerText.includes('The backend restarted before this conversation finished.')`,
      ),
    "existing browser reconnects and shows interruption",
  );
  assert.equal(requests, before, "interrupted actions are not automatically replayed");
  const messages = await call("channel_messages", { channelId: agent.id });
  assert.equal(messages.filter((m) => m.parts.some((p) => p.kind === "interrupted")).length, 1);
  console.log(
    "PASS: crash recovery preserves work, updates the connected UI, and does not replay actions.",
  );
} finally {
  chromeSocket?.close();
  for (const child of children.reverse()) await stop(child);
  model.closeAllConnections();
  await new Promise((resolve) => model.close(resolve));
  await rm(scratch, { recursive: true, force: true });
}
