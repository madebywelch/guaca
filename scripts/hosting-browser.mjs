// Real Chromium + real guacad, with an offline model. No account or API key.
// GUACAD=/path/to/guacad CHROME_BIN=/path/to/chrome node scripts/hosting-browser.mjs
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, rm, readFile, writeFile } from "node:fs/promises";
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
      () => evaluate(`document.body?.innerText.includes('Browser check')`),
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
  finish("Finished while the client was closed.\n\nThe workspace keeps working while you are away. You can read replies, switch agents, and review their notes from your phone.\n\n- Open Agents to choose a crew.\n- Return to Chat without losing your draft.\n- Open Details for memory and routines.\n\nA long reference should wrap: https://example.com/" + "reference".repeat(18));
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

  // Phone layout uses real touch hit testing. Keep a draft while changing
  // panes, and shrink the viewport as a keyboard would before returning wide.
  const tap = async (selector) => {
    const point = await evaluate(`(() => {
      const node = document.querySelector(${JSON.stringify(selector)});
      if (!node) throw new Error('Missing target: ' + ${JSON.stringify(selector)});
      node.scrollIntoView({block:'nearest'});
      const r = node.getBoundingClientRect();
      if (!r.width || !r.height) throw new Error('Hidden target');
      return {x:r.x+r.width/2,y:r.y+r.height/2};
    })()`);
    await cdp("Input.dispatchTouchEvent", {type:"touchStart", touchPoints:[point]}, session);
    await cdp("Input.dispatchTouchEvent", {type:"touchEnd", touchPoints:[]}, session);
  };
  const fits = async (selector) => {
    const bounds = await evaluate(`(() => {
      const n=document.querySelector(${JSON.stringify(selector)}), r=n.getBoundingClientRect();
      return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:r.width,height:r.height,
        overflow:n.scrollWidth-n.clientWidth, vw:innerWidth, vh:visualViewport.height};
    })()`);
    assert.ok(bounds.width > 0 && bounds.height > 0, `${selector} is visible`);
    assert.ok(bounds.left >= -1 && bounds.right <= bounds.vw+1, `${selector} fits horizontally: ${JSON.stringify(bounds)}`);
    assert.ok(bounds.top >= -1 && bounds.bottom <= bounds.vh+1, `${selector} fits vertically: ${JSON.stringify(bounds)}`);
    assert.ok(bounds.overflow <= 1, `${selector} has no horizontal overflow: ${JSON.stringify(bounds)}`);
  };
  const size = async (width,height,mobile=true) => {
    await cdp("Emulation.setDeviceMetricsOverride", {width,height,deviceScaleFactor:1,mobile},session);
    await cdp("Emulation.setTouchEmulationEnabled", {enabled:mobile},session);
    await delay(150);
  };
  for (const width of [320,390,430,768]) {
    await size(width,844);
    await fits(".app");
    await fits(".pane");
    await fits(".pane__scroll");
    await fits(".composer");
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.rail')).display"),"none");
    await tap(".composer__input");
    await cdp("Input.insertText", {text:"Draft survives navigation"},session);
    await tap(".mobile-nav button:first-child");
    await fits(".rail");
    await fits(".mobile-crews select");
    await tap(".agent-row");
    assert.equal(await evaluate("document.querySelector('.composer__input').value"),"Draft survives navigation");
    await tap(".mobile-nav button:last-child");
    await fits(".inspector");
    await tap(".mobile-nav button:nth-child(2)");
    await size(width,400);
    await fits(".composer");
    await size(width,844);
    await evaluate("document.querySelector('.composer__input').focus()");
    // React must see the input event; assigning .value alone would not clear its draft.
    await evaluate(`(() => { const n=document.querySelector('.composer__input');
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(n,'');
      n.dispatchEvent(new Event('input',{bubbles:true})); n.blur(); })()`);
    await tap(".mobile-nav button:first-child");
    await tap(".rail__foot button:last-child");
    await until(() => evaluate("!!document.querySelector('.dialog--settings')"),"settings opens on phone");
    await delay(250);
    await fits(".dialog--settings");
    await fits(".settings__pane");
    await fits(".settings__foot");
    if (width === 320) {
      const tabs = await evaluate("document.querySelectorAll('.settings__tab').length");
      for (let i=1;i<=tabs;i++) {
        await tap(`.settings__tab:nth-child(${i})`);
        await delay(100);
        await fits(".settings__pane");
        await fits(".settings__foot");
      }
      await tap(".settings__tab:first-child");
    }
    if (process.env.GUACA_BROWSER_SHOTS) {
      await mkdir(process.env.GUACA_BROWSER_SHOTS,{recursive:true});
      const shot=await cdp("Page.captureScreenshot",{format:"png"},session);
      await writeFile(path.join(process.env.GUACA_BROWSER_SHOTS,`settings-${width}.png`),Buffer.from(shot.data,"base64"));
    }
    await evaluate("document.querySelector('.scrim__close').click()");
    if (width === 320) {
      for (const [button,dialog] of [[".rail__foot button:first-child",".dialog--cafeteria"],[".rail__foot button:nth-child(2)",".dialog--calendar"],[".rail__for-you",".for-you"]]) {
        await tap(button);
        await until(() => evaluate(`!!document.querySelector('${dialog}')`),`${dialog} opens`);
        await delay(250);
        await fits(dialog);
        await evaluate("document.querySelector('.scrim__close').click()");
      }
    }
    await tap(".agent-row");
    if (process.env.GUACA_BROWSER_SHOTS) {
      const shot=await cdp("Page.captureScreenshot",{format:"png"},session);
      await writeFile(path.join(process.env.GUACA_BROWSER_SHOTS,`chat-${width}.png`),Buffer.from(shot.data,"base64"));
    }
  }
  await size(844,390);
  await fits(".pane");
  await fits(".composer");
  await fits(".mobile-nav");
  await size(1280,900,false);
  await fits(".pane");
  await fits(".rail");
  await fits(".inspector");
  assert.equal(await evaluate("getComputedStyle(document.querySelector('.mobile-nav')).display"),"none");
  console.log("PASS: phone widths, touch navigation, preserved drafts, keyboard-sized viewport, settings and desktop layout.");

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
