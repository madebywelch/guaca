#!/usr/bin/env python3
"""Drives the sandbox's Chrome over the DevTools Protocol.

Guac calls this instead of aiming a pointer at pixels. Chrome already knows
where every element is, what it says and what it does, so asking it is exact
where a screenshot is a guess.

Elements are addressed by the number `read` gives them. The numbering is stored
on the page itself, so a click refers to the same element the model was shown
rather than to whatever now happens to sit at some coordinate.
"""

import json
import sys
import urllib.request

import websocket

DEBUG_URL = "http://127.0.0.1:9222"

# Anything a person could click, type into, or choose from.
COLLECT_JS = r"""
(() => {
  const sel = 'a,button,input,select,textarea,summary,[role=button],[role=link],'
            + '[role=tab],[role=checkbox],[role=menuitem],[contenteditable=true],[onclick]';
  const out = [];
  window.__guacEls = [];
  for (const el of document.querySelectorAll(sel)) {
    const r = el.getBoundingClientRect();
    // Off-screen and zero-sized elements are not things a person can use, and
    // listing them buries the ones that are.
    if (r.width < 2 || r.height < 2) continue;
    if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
    const style = getComputedStyle(el);
    if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') continue;

    const label = (el.innerText || el.value || el.placeholder || el.getAttribute('aria-label')
                   || el.getAttribute('title') || el.alt || '').trim().replace(/\s+/g, ' ');
    const id = window.__guacEls.length;
    window.__guacEls.push(el);
    out.push({
      id,
      tag: el.tagName.toLowerCase(),
      type: el.getAttribute('type') || '',
      text: label.slice(0, 80),
      x: Math.round(r.x + r.width / 2),
      y: Math.round(r.y + r.height / 2),
    });
    if (out.length >= 120) break;
  }
  const body = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n');
  return JSON.stringify({
    url: location.href,
    title: document.title,
    scroll: Math.round(scrollY),
    height: Math.round(document.body ? document.body.scrollHeight : 0),
    text: body.slice(0, 6000),
    elements: out,
  });
})()
"""


def page_socket():
    targets = json.load(urllib.request.urlopen(DEBUG_URL + "/json", timeout=10))
    pages = [t for t in targets if t.get("type") == "page"]
    if not pages:
        raise SystemExit("no page is open in the browser")
    # The most recently opened tab is the one being worked on.
    return websocket.create_connection(
        pages[-1]["webSocketDebuggerUrl"], timeout=30, suppress_origin=True
    )


class Session:
    def __init__(self):
        self.ws = page_socket()
        self.next_id = 0

    def send(self, method, params=None):
        self.next_id += 1
        want = self.next_id
        self.ws.send(json.dumps({"id": want, "method": method, "params": params or {}}))
        while True:
            message = json.loads(self.ws.recv())
            if message.get("id") == want:
                if "error" in message:
                    raise SystemExit(message["error"].get("message", "the browser refused that"))
                return message.get("result", {})

    def evaluate(self, expression):
        result = self.send(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": True},
        )
        if result.get("exceptionDetails"):
            detail = result["exceptionDetails"]
            # The useful part is nested; the top-level text is always "Uncaught".
            described = (detail.get("exception") or {}).get("description") or detail.get("text")
            raise SystemExit(str(described)[:400])
        return result.get("result", {}).get("value")

    def require(self, index):
        """A page that has re-rendered has forgotten its numbering.

        Saying so plainly beats acting on whatever element now happens to hold
        that number, which is how an agent ends up clicking the wrong thing and
        reporting success.
        """
        ok = self.evaluate(
            f"Boolean(window.__guacEls && window.__guacEls[{index}]"
            f" && document.contains(window.__guacEls[{index}]))"
        )
        if not ok:
            raise SystemExit(
                f"element {index} is no longer on the page; read it again to renumber"
            )

    def settle(self, ms=1200):
        """Give the page a moment to react before it is read again."""
        self.evaluate(f"new Promise(r => setTimeout(() => r(1), {ms}))")


def main():
    action = sys.argv[1] if len(sys.argv) > 1 else "read"
    args = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
    s = Session()

    if action == "open":
        url = args.get("url", "")
        if not url.startswith(("http://", "https://")):
            url = "https://" + url
        s.send("Page.navigate", {"url": url})
        s.settle(2500)

    elif action == "click":
        s.require(int(args["id"]))
        s.evaluate(f"window.__guacEls[{int(args['id'])}].scrollIntoView({{block:'center'}})")
        s.evaluate(f"window.__guacEls[{int(args['id'])}].click()")
        s.settle()

    elif action == "type":
        target = int(args["id"])
        s.require(target)
        text = json.dumps(args.get("text", ""))
        s.evaluate(
            f"(() => {{ const e = window.__guacEls[{target}]; e.focus();"
            f" const v = {text};"
            f" if (e.isContentEditable) {{ e.textContent = v; }} else {{ e.value = v; }}"
            f" e.dispatchEvent(new Event('input', {{bubbles:true}}));"
            f" e.dispatchEvent(new Event('change', {{bubbles:true}})); return 1; }})()"
        )
        if args.get("submit"):
            # A real key event, because many pages listen for the keystroke
            # rather than for a form submission.
            for kind in ("keyDown", "char", "keyUp"):
                s.send(
                    "Input.dispatchKeyEvent",
                    {
                        "type": kind,
                        "key": "Enter",
                        "code": "Enter",
                        "text": "\r",
                        "windowsVirtualKeyCode": 13,
                        "nativeVirtualKeyCode": 13,
                    },
                )
            s.settle(2500)

    elif action == "scroll":
        amount = int(args.get("amount", 3)) * 400
        if args.get("direction") == "up":
            amount = -amount
        s.evaluate(f"scrollBy(0, {amount})")
        s.settle(600)

    elif action == "back":
        s.evaluate("history.back()")
        s.settle(2000)

    elif action == "read":
        pass

    else:
        raise SystemExit(f"unknown action {action!r}")

    print(s.evaluate(COLLECT_JS))


if __name__ == "__main__":
    main()
