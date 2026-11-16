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
  // The agents are in the Groups sheet, opened from the people button in the
  // top row; a row there opens the channel and closes the sheet.
  const openAgent = (name) =>
    evaluate(`(async () => {
      const row = () => [...document.querySelectorAll('.sheet .agent-row')].find((e) => e.textContent.includes(${JSON.stringify(name)}));
      if (!row()) [...document.querySelectorAll('.topbar__places button')].find((e) => /^(Agents|Groups and agents)$/.test(e.getAttribute('aria-label') || ''))?.click();
      await new Promise((r) => setTimeout(r, 100));
      if (!row()) return false;
      row().click();
      return true;
    })()`);
  const openClient = async () => {
    tab = (await cdp("Target.createTarget", { url: `${base}/#token=${token}` })).targetId;
    session = (await cdp("Target.attachToTarget", { targetId: tab, flatten: true })).sessionId;
    await until(() => openAgent("Browser check"), "real app renders its roster");
    // A working agent opens on Now; what this check reads is the transcript.
    await until(
      () =>
        evaluate(
          "(() => { const tab = [...document.querySelectorAll('.agent-tabs__tab')].find((e) => e.textContent === 'Conversation'); tab?.click(); return !!tab; })()",
        ),
      "the conversation tab",
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
  finish("Finished while the client was closed.\n\nThe workspace keeps working while you are away. You can read replies, switch agents, and review their notes from your phone.\n\n- Open Chats to choose a crew.\n- Return to Chat without losing your draft.\n- Open Details for memory and routines.\n\nA long reference should wrap: https://example.com/" + "reference".repeat(18));
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
  const shoot = async (name) => {
    if (!process.env.GUACA_BROWSER_SHOTS) return;
    await mkdir(process.env.GUACA_BROWSER_SHOTS,{recursive:true});
    const shot=await cdp("Page.captureScreenshot",{format:"png"},session);
    await writeFile(path.join(process.env.GUACA_BROWSER_SHOTS,`${name}.png`),Buffer.from(shot.data,"base64"));
  };
  const pageTab = (label) => `.agent-tabs__tab:nth-child(${["Now","Conversation"].indexOf(label)+1})`;
  const box = (selector) => evaluate(`(() => { const r=document.querySelector(${JSON.stringify(selector)})?.getBoundingClientRect(); return r ? {left:r.left,right:r.right,top:r.top,bottom:r.bottom,height:r.height} : null; })()`);
  // Every label on the timeline is inside the top row's margins, and none
  // is crossed by the now line.
  const ribbonClear = async (where) => {
    const facts = await evaluate(`(() => {
      const edge = parseFloat(getComputedStyle(document.querySelector('.topbar')).paddingRight);
      const now = document.querySelector('.ribbon__now').getBoundingClientRect();
      const nowX = now.left + now.width / 2;
      const labels = [...document.querySelectorAll('.ribbon__name, .ribbon__item-what, .ribbon__item-when, .ribbon__pile-label, .ribbon__pile-faces')]
        .filter((n) => n.getClientRects().length)
        .map((n) => { const r = n.getBoundingClientRect(); return {text:n.textContent, left:r.left, right:r.right}; });
      return {edge, nowX, vw: innerWidth, labels};
    })()`);
    for (const label of facts.labels) {
      assert.ok(label.left >= facts.edge - 1 && label.right <= facts.vw - facts.edge + 1, `${where}: "${label.text}" is inside the margins: ${JSON.stringify({...label, edge:facts.edge, vw:facts.vw})}`);
      if (label.text) assert.ok(label.right <= facts.nowX - 1 || label.left >= facts.nowX + 1, `${where}: "${label.text}" is clear of the now line at ${facts.nowX}: ${JSON.stringify(label)}`);
    }
  };
  for (const width of [320,390,430,768]) {
    await size(width,844);
    await fits(".app");
    await fits(".topbar");
    await fits(".pane");
    assert.ok((await evaluate("document.querySelector('.pane__title').textContent")).length > 0, "the page names its agent");
    await fits(".pane__scroll");
    await fits(".composer");
    assert.equal(await evaluate("document.querySelector('.app > .rail')"), null, "no rail beside the column");
    // An agent's page on a phone is its conversation: no timeline, a one-line
    // head, a one-row status bar, and most of the height to read and write in.
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.ribbon')).display"), "none", "the timeline gives way on an agent's page");
    const head = await box(".pane__header");
    assert.ok(head.height <= 60, `the page's head is one line: ${JSON.stringify(head)}`);
    const bar = await box(".statusbar");
    assert.ok(bar.height <= 60, `the status bar is one row: ${JSON.stringify(bar)}`);
    const reading = await evaluate("(() => { const log=document.querySelector('.pane__scroll[role=log]').getBoundingClientRect(); const c=document.querySelector('.composer').getBoundingClientRect(); return (c.bottom - log.top) / visualViewport.height; })()");
    assert.ok(reading >= 0.6, `conversation and composer take at least 60% of the height: ${reading}`);
    await tap(".composer__input");
    // The draft from the last width is still there; typing replaces it.
    await evaluate("document.querySelector('.composer__input').select()");
    await cdp("Input.insertText", {text:"Draft survives navigation"},session);
    // The agent menu has a way in by touch on the page itself.
    await tap(".pane__more");
    await until(() => evaluate("!!document.querySelector('.menu')"), "agent actions open by touch");
    await fits(".menu");
    await evaluate("document.querySelector('.menu__scrim').click()");
    await tap(".topbar__back");
    await until(() => evaluate("!!document.querySelector('.home')"), "Today opens from the top row");
    await fits(".home");
    await fits(".ribbon");
    await ribbonClear(`Today at ${width}`);
    await tap('.topbar__places button[aria-label="Groups and agents"], .topbar__places button[aria-label="Agents"]');
    await until(() => evaluate("!!document.querySelector('.sheet')"), "the Groups sheet opens");
    await delay(400);
    await fits(".sheet");
    await fits(".sheet__head");
    await fits(".mobile-crews select");
    assert.equal(await evaluate("[...document.querySelectorAll('.sheet button')].filter((b) => b.getClientRects().length && /^(For you|Calendar|Artifacts)/.test(b.textContent.trim())).length"), 0, "a place is drawn once: not again in the sheet");
    await shoot(`chats-${width}`);
    await tap(".sheet__head button");
    await until(() => evaluate("!document.querySelector('.sheet')"), "the Groups sheet closes by its own button");
    await tap('.topbar__places button[aria-label="Search"]');
    await until(() => evaluate("!!document.querySelector('.palette__input')"), "search opens from the top row");
    await tap(".palette__input");
    await cdp("Input.insertText", {text:"Browser check"},session);
    await until(() => evaluate("!!document.querySelector('.palette__row')"), "agent is found");
    await tap(".palette__row");
    await fits(".pane");
    assert.equal(await evaluate("document.querySelector('.composer__input').value"),"Draft survives navigation");
    await tap(pageTab("Now"));
    await until(() => evaluate("!!document.querySelector('.agent-panel')"), "the agent's details open as a tab");
    await fits(".pane__scroll:has(> .agent-panel)");
    // Writing while watching: the composer stays under Now.
    await fits(".composer");
    await shoot(`details-${width}`);
    await tap(pageTab("Conversation"));
    await tap('.topbar__places button[aria-label="Search"]');
    await until(() => evaluate("!!document.querySelector('.palette')"), "search opens from chat");
    await fits(".palette");
    await tap(".palette__query button");
    await until(() => evaluate("!document.querySelector('.palette')"), "search cancels by touch");
    assert.equal(await evaluate("document.querySelector('.composer__input').value"), "Draft survives navigation");
    await size(width,400);
    await fits(".composer");
    await size(width,844);
    await evaluate("document.querySelector('.composer__input').focus()");
    // React must see the input event; assigning .value alone would not clear its draft.
    await evaluate(`(() => { const n=document.querySelector('.composer__input');
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(n,'');
      n.dispatchEvent(new Event('input',{bubbles:true})); n.blur(); })()`);
    await tap('.topbar__places button[aria-label="Settings"]');
    await until(() => evaluate("!!document.querySelector('.dialog--settings')"),"settings opens on phone");
    await delay(250);
    await fits(".dialog--settings");
    await fits(".settings__pane");
    await fits(".settings__foot");
    if (width === 320) {
      const sections = await evaluate("Array.from(document.querySelector('.settings__section select').options).map(o=>o.value)");
      for (const section of sections) {
        await evaluate(`(() => {const n=document.querySelector('.settings__section select'); n.value=${JSON.stringify(section)}; n.dispatchEvent(new Event('change',{bubbles:true}));})()`);
        await delay(100);
        await fits(".settings__pane");
        await fits(".settings__foot");
      }
    }
    await shoot(`settings-${width}`);
    await evaluate("document.querySelector('.scrim__close').click()");
    if (width === 320) {
      for (const [button,dialog] of [[".statusbar__places button:nth-child(1)",".dialog--artifacts"],[".statusbar__places button:nth-child(2)",".dialog--calendar"],[".statusbar__places button:nth-child(3)",".for-you"]]) {
        await tap(button);
        await until(() => evaluate(`!!document.querySelector('${dialog}')`),`${dialog} opens`);
        await delay(250);
        await fits(dialog);
        await evaluate("document.querySelector('.scrim__close').click()");
      }
      await tap('.topbar__places .rail__new');
      await until(() => evaluate("!!document.querySelector('.menu')"), "the New menu opens by touch");
      await evaluate("[...document.querySelectorAll('.menu .menu__item')].find((n) => n.textContent.includes('cafeteria')).click()");
      await until(() => evaluate("!!document.querySelector('.dialog--cafeteria')"), ".dialog--cafeteria opens");
      await delay(250);
      await fits(".dialog--cafeteria");
      await evaluate("document.querySelector('.scrim__close').click()");
    }
    await shoot(`chat-${width}`);
  }
  await size(844,390);
  await fits(".pane");
  await fits(".composer");
  await size(1280,900,false);
  await fits(".topbar");
  await fits(".ribbon");
  await fits(".pane");
  assert.equal(await evaluate("document.querySelector('.app > .rail')"), null, "no rail beside the column");
  await tap(".topbar__back");
  await until(() => evaluate("!!document.querySelector('.home')"), "Today opens on the desktop");
  await ribbonClear("Today at 1280");
  console.log("PASS: phone widths, touch navigation and the agent menu, preserved drafts, keyboard-sized viewport, places, timeline margins and desktop layout.");

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
  // Chrome's helpers can still be writing its profile a moment after it exits.
  await rm(scratch, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}
