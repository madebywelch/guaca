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
  await call("create_agent", {
    draft: {
      groupId: group.id,
      name: "Browser check reviewer",
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
      "--window-size=1280,820",
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
  const showingAgent = (name) =>
    evaluate(`document.querySelector(${JSON.stringify(`[role="log"][aria-label="Conversation with ${name}"]`)})?.checkVisibility()`);
  const openClient = async () => {
    tab = (await cdp("Target.createTarget", { url: `${base}/#token=${token}` })).targetId;
    session = (await cdp("Target.attachToTarget", { targetId: tab, flatten: true })).sessionId;
    await until(() => showingAgent("Browser check"), "the workspace opens on the first agent's conversation");
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

  const tap = async (selector, text = null) => {
    const point = await evaluate(`(() => {
      const text = ${JSON.stringify(text)};
      const node = [...document.querySelectorAll(${JSON.stringify(selector)})]
        .find(candidate => text === null || candidate.textContent.trim() === text);
      if (!node) throw new Error('Missing target: ' + ${JSON.stringify(selector)});
      node.scrollIntoView({block:'nearest'});
      const r = node.getBoundingClientRect();
      if (!r.width || !r.height) throw new Error('Hidden target');
      return {x:r.x+r.width/2,y:r.y+r.height/2};
    })()`);
    await touch(point);
  };
  const touch = async (point) => {
    await cdp("Input.dispatchTouchEvent", {type:"touchStart", touchPoints:[point]}, session);
    await cdp("Input.dispatchTouchEvent", {type:"touchEnd", touchPoints:[]}, session);
  };
  const byRole = async (role, name, within) => {
    const scope = within ? {backendNodeId:within.backendDOMNodeId} : {nodeId:(await cdp("DOM.getDocument", {depth:0}, session)).root.nodeId};
    const { nodes } = await cdp("Accessibility.queryAXTree", {...scope, role}, session);
    return nodes.find((node) => !node.ignored && (name instanceof RegExp ? name.test(node.name?.value) : node.name?.value === name));
  };
  const tapRole = async (role, name, within) => {
    const node = await byRole(role, name, within);
    if (!node) throw new Error(`Missing ${role} named ${name}`);
    await cdp("DOM.scrollIntoViewIfNeeded", {backendNodeId:node.backendDOMNodeId}, session);
    const [left, top, , , right, bottom] = (await cdp("DOM.getBoxModel", {backendNodeId:node.backendDOMNodeId}, session)).model.border;
    await touch({x:(left+right)/2, y:(top+bottom)/2});
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
  const rosterClear = async (where) => {
    await fits(".roster");
    const facts = await evaluate(`(() => {
      const roster = document.querySelector('.roster');
      const range = document.createRange();
      return [...roster.querySelectorAll('.roster__agent')].map(row => {
        const name = row.querySelector('.roster__name');
        range.selectNodeContents(name);
        const box = row.getBoundingClientRect(), bounds = name.getBoundingClientRect();
        const clipped = name.scrollWidth > name.clientWidth || name.scrollHeight > name.clientHeight;
        const visible = clipped ? bounds : range.getBoundingClientRect();
        const style = getComputedStyle(name);
        return { name: name.textContent.trim(), clipped, shown: visible.width > 0 && visible.height > 0 &&
          name.checkVisibility({opacityProperty: true, visibilityProperty: true}) &&
          visible.left >= box.left - 1 && visible.left >= bounds.left - 1 &&
          visible.right <= box.right + 1 && visible.right <= bounds.right + 1 &&
          visible.top >= box.top - 1 && visible.bottom <= box.bottom + 1 &&
          (!clipped || (style.webkitLineClamp === '2' && style.overflow === 'hidden')),
          fullName: name.title === name.textContent && row.getAttribute('aria-label')?.startsWith(name.textContent + ', ') };
      });
    })()`);
    assert.deepEqual(facts.map(row => row.name), ["Browser check", "Browser check reviewer"], `${where}: both names are present`);
    assert.ok(!facts[0].clipped && facts.every(row => row.shown && row.fullName),
      `${where}: the shorter name fits and both give their full names on hover and to assistive technology: ${JSON.stringify(facts)}`);
  };
  for (const width of [320,390,430,768]) {
    await size(width,844);
    await fits(".app");
    await fits(".topbar");
    await fits(".pane");
    assert.equal(await evaluate("document.querySelector('.pane__title').textContent"), "Browser check", "the page names its agent");
    await fits(".pane__scroll");
    await fits(".composer");
    assert.equal(await evaluate("document.querySelector('.app > .rail')"), null, "no rail beside the column");
    const head = await box(".pane__header");
    assert.ok(head.height <= 60, `the page's head is one line: ${JSON.stringify(head)}`);
    const foot = await box(".roster__foot");
    const pane = await box(".pane");
    assert.ok(foot.height <= 60 || foot.right <= pane.left + 1, `the sidebar's foot is one row, or sits beside the conversation: ${JSON.stringify({ foot, pane })}`);
    const reading = await evaluate("(() => { const log=document.querySelector('.pane__scroll[role=log]').getBoundingClientRect(); const c=document.querySelector('.composer').getBoundingClientRect(); return (c.bottom - log.top) / visualViewport.height; })()");
    assert.ok(reading >= 0.6, `conversation and composer take at least 60% of the height: ${reading}`);
    await tap(".composer__input");
    await evaluate("document.querySelector('.composer__input').select()");
    await cdp("Input.insertText", {text:"Draft survives navigation"},session);
    await tap(".pane__more");
    await until(() => evaluate("!!document.querySelector('.menu')"), "agent actions open by touch");
    await fits(".menu");
    await evaluate("document.querySelector('.menu__scrim').click()");
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
    await tap('.topbar__places button[aria-label="Groups and agents"], .topbar__places button[aria-label="Agents"]');
    await until(() => evaluate("document.querySelectorAll('.sheet .agent-row').length === 2"), "both agents are on the Groups sheet");
    await tap(".sheet .agent-row__name", "Browser check reviewer");
    await until(() => showingAgent("Browser check reviewer"), "touch opens the other conversation");
    assert.equal(await evaluate("document.querySelector('.composer__input').value"), "", "the other agent does not get the draft");
    await tapRole("button", "Groups and agents");
    await until(() => byRole("dialog", "Groups and agents"), "the Groups sheet opens again");
    await tapRole("button", "Command menu");
    await until(async () => (await byRole("textbox", "Command menu"))?.properties?.some((p) => p.name === "focused" && p.value.value),
      "the command menu opens from the sheet, ready to type");
    await cdp("Input.insertText", {text:"Browser check"},session);
    const menu = await byRole("dialog", "Command menu");
    await until(() => byRole("button", /^Browser check(?! reviewer)/, menu), "the exact agent is found");
    await tapRole("button", /^Browser check(?! reviewer)/, menu);
    await until(() => showingAgent("Browser check"), "the command menu returns to the drafted conversation");
    await fits(".pane");
    assert.equal(await evaluate("document.querySelector('.composer__input').value"),"Draft survives navigation");
    await tap(pageTab("Now"));
    await until(() => evaluate("!!document.querySelector('.agent-panel')"), "the agent's details open as a tab");
    await fits(".pane__scroll:has(> .agent-panel)");
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
    if (width === 320) {
      for (const height of [355, 320]) {
        await size(width,height);
        await fits(".composer__input");
        await fits(".composer__send");
        assert.equal(await evaluate("document.querySelector('.roster__compact').getClientRects().length"), 1, "the agent picker is visible with the keyboard up");
      }
    }
    await size(width,844);
    await evaluate("document.querySelector('.composer__input').focus()");
    await evaluate(`(() => { const n=document.querySelector('.composer__input');
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(n,'');
      n.dispatchEvent(new Event('input',{bubbles:true})); n.blur(); })()`);
    await tap('.topbar__places button[aria-label="App settings"]');
    await until(() => evaluate("!!document.querySelector('.settings')"),"settings opens on phone");
    await fits(".settings");
    await fits(".settings__pane");
    await tap('.settings__pane input[placeholder="Unnamed"]');
    await cdp("Input.insertText", {text:"Browser operator"},session);
    await until(() => evaluate("!!document.querySelector('.settings__foot')"), "an edit shows the Settings actions");
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
    await evaluate("document.querySelector('.settings__foot button').click()");
    await until(() => evaluate("!document.querySelector('.settings__foot')"), "discard clears the unsaved Settings actions");
    await tap(".topbar__back");
    await until(() => showingAgent("Browser check"), "Settings back returns to the conversation");
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
  await size(900,600,false);
  await fits(".pane");
  await fits(".composer");
  const short = await evaluate("(() => { const log=document.querySelector('.pane__scroll[role=log]').getBoundingClientRect(); const c=document.querySelector('.composer').getBoundingClientRect(); return (c.bottom - log.top) / innerHeight; })()");
  assert.ok(short >= 0.6, `at 900 by 600, conversation and composer take at least 60% of the height: ${short}`);
  await size(1280,900,false);
  await fits(".topbar");
  await rosterClear("the 1280px conversation");
  await fits(".pane");
  assert.equal(await evaluate("document.querySelector('.app > .rail')"), null, "no rail beside the column");
  const draft = { groupId: group.id, name: "Browser check", avatar: "avocado", color: "#7ab55c", systemPrompt: "Answer briefly." };
  const routine = await call("create_routine", { agentId: agent.id, draft: { name: "Morning check", what: "Say good morning.", trigger: "every:86400", inSecs: null, skipIfWorking: false } });
  await call("update_agent", { id: agent.id, draft: { ...draft, model: "offline-test-with-a-long-dated-model-identifier-2026-11-16" } });
  await until(() => evaluate("!!document.querySelector('.agent-tabs__count')"), "the routine count shows on its tab");
  await until(() => evaluate("document.querySelector('.pane__header .pane__meta')?.textContent.includes('Long Dated')"), "the long model is in the caption");
  for (const width of [1280, 900]) {
    await size(width, 900, false);
    await rosterClear(`the ${width}px conversation`);
    const row = await evaluate(`(() => {
      const list = document.querySelector('.agent-tabs__list'), edge = list.getBoundingClientRect().right;
      const tabs = [...list.querySelectorAll('.agent-tabs__tab')].map((n) => ({ text: n.textContent, right: n.getBoundingClientRect().right, cut: n.scrollWidth - n.clientWidth }));
      return { edge, overflow: list.scrollWidth - list.clientWidth, tabs, caption: getComputedStyle(document.querySelector('.pane__header .pane__meta')).display };
    })()`);
    assert.notEqual(row.caption, "none", `the caption is drawn at ${width}: ${JSON.stringify(row)}`);
    assert.ok(row.overflow <= 1, `the tabs fit their row at ${width}: ${JSON.stringify(row)}`);
    for (const tab of row.tabs) assert.ok(tab.cut <= 1 && tab.right <= row.edge + 1, `"${tab.text}" is not cut at ${width}: ${JSON.stringify(row)}`);
    await shoot(`tabs-${width}`);
  }
  await size(1280, 900, false);
  await call("delete_routine", { id: routine.id });
  await call("update_agent", { id: agent.id, draft: { ...draft, model: "offline-test" } });
  console.log("PASS: phone widths, touch navigation and the agent menu, preserved drafts, keyboard-sized viewport, places, and readable desktop roster names.");

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
  await rm(scratch, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}
