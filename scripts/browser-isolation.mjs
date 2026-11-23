const [command, base, ...rest] = process.argv.slice(2);

const CALL_TIMEOUT_MS = 30000;

function failAfter(ms, what) {
  let timer;
  const promise = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} took longer than ${ms} ms`)), ms);
  });
  return { promise, clear: () => clearTimeout(timer) };
}

async function within(ms, what, work) {
  const limit = failAfter(ms, what);
  try {
    return await Promise.race([work, limit.promise]);
  } finally {
    limit.clear();
  }
}

async function version() {
  const response = await fetch(`${base}/json/version`);
  return { status: response.status, body: await response.text() };
}

async function connect() {
  const { status, body } = await version();
  if (status !== 200) throw new Error(`json/version answered ${status}: ${body}`);
  const socket = new WebSocket(JSON.parse(body).webSocketDebuggerUrl);
  await within(CALL_TIMEOUT_MS, "connecting", new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = () => reject(new Error("the socket would not open"));
  }));
  let next = 0;
  const waiting = new Map();
  const listeners = [];
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && waiting.has(message.id)) {
      const { resolve, reject } = waiting.get(message.id);
      waiting.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
      return;
    }
    for (const listen of listeners) listen(message);
  };
  const call = (method, params = {}, sessionId) => {
    const id = ++next;
    socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    return within(CALL_TIMEOUT_MS, method, new Promise((resolve, reject) => waiting.set(id, { resolve, reject })));
  };
  const once = (method, ms) =>
    within(ms, method, new Promise((resolve) => {
      listeners.push((message) => {
        if (message.method === method) resolve(message.params);
      });
    }));
  return { socket, call, once, on: (listen) => listeners.push(listen) };
}

async function page(cdp) {
  const { targetId } = await cdp.call("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.call("Target.attachToTarget", { targetId, flatten: true });
  await cdp.call("Page.enable", {}, sessionId);
  const evaluate = async (expression) =>
    (await cdp.call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId))
      .result.value;
  const open = async (url) => {
    const loaded = cdp.once("Page.loadEventFired", CALL_TIMEOUT_MS).catch(() => null);
    const navigated = await cdp.call("Page.navigate", { url }, sessionId);
    if (navigated.errorText) return { error: navigated.errorText };
    await loaded;
    return { title: await evaluate("document.title"), text: await evaluate("document.body ? document.body.innerText : ''") };
  };
  return { targetId, sessionId, evaluate, open };
}

const WEBRTC = `new Promise((done) => {
  const found = [];
  const pc = new RTCPeerConnection({ iceServers: [{ urls: "stun:stun.l.google.com:19302" }] });
  pc.onicecandidate = (event) => event.candidate ? found.push(event.candidate.candidate) : done(found);
  pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && done(found);
  pc.createDataChannel("probe");
  pc.createOffer().then((offer) => pc.setLocalDescription(offer));
  setTimeout(() => done(found), 8000);
})`;

async function main() {
  switch (command) {
    case "version": {
      const { status, body } = await version();
      console.log(JSON.stringify({ status, url: status === 200 ? JSON.parse(body).webSocketDebuggerUrl : body }));
      return;
    }
    case "visit": {
      const cdp = await connect();
      const tab = await page(cdp);
      console.log(JSON.stringify(await tab.open(rest[0])));
      cdp.socket.close();
      return;
    }
    case "webrtc": {
      const cdp = await connect();
      const tab = await page(cdp);
      await tab.open(rest[0]);
      const found = await tab.evaluate(WEBRTC);
      console.log(JSON.stringify({ candidates: found, udp: found.filter((c) => / udp /i.test(c)) }));
      cdp.socket.close();
      return;
    }
    case "download": {
      const cdp = await connect();
      await cdp.call("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: rest[0], eventsEnabled: true });
      const began = [];
      cdp.on((message) => {
        if (message.method === "Browser.downloadWillBegin") began.push(message.params.suggestedFilename);
      });
      const tab = await page(cdp);
      await tab.open(
        "data:text/html,<a id=get href='data:application/octet-stream;base64,aGVsbG8=' download='probe.bin'>get</a>",
      );
      await tab.evaluate("document.getElementById('get').click(); true");
      console.log(JSON.stringify({ began }));
      cdp.socket.close();
      return;
    }
    default:
      throw new Error(`unknown command ${command}`);
  }
}

main().catch((error) => {
  console.log(JSON.stringify({ failed: String(error.message ?? error) }));
  process.exit(1);
});
