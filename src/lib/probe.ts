import { diagnosticRows } from "./diagnostic";
import { useStore } from "./store";
import { attached, saveConnection } from "./transport";

export type Call =
  | { do: "ready" }
  | { do: "view" }
  | { do: "diagnostic" }
  | { do: "attach"; origin: string; token: string }
  | { do: "find"; role: string; name?: string }
  | { do: "click"; role: string; name: string }
  | { do: "type"; role: string; name: string; text: string }
  | { do: "key"; key: string }
  | { do: "text"; role?: string; name?: string }
  | { do: "box"; role: string; name: string }
  | { do: "focus" };

export type Answer = { ok: true; value: unknown } | { ok: false; error: string };

function implicitRole(element: Element): string | null {
  const tag = element.tagName.toLowerCase();
  switch (tag) {
    case "button":
    case "summary":
      return "button";
    case "a":
      return element.hasAttribute("href") ? "link" : null;
    case "textarea":
      return "textbox";
    case "select":
      return element.hasAttribute("multiple") ? "listbox" : "combobox";
    case "h1":
    case "h2":
    case "h3":
    case "h4":
    case "h5":
    case "h6":
      return "heading";
    case "nav":
      return "navigation";
    case "main":
      return "main";
    case "dialog":
      return "dialog";
    case "ul":
    case "ol":
      return "list";
    case "li":
      return "listitem";
    case "img":
      return element.getAttribute("alt") === "" ? null : "img";
    case "option":
      return "option";
    case "input": {
      const type = (element.getAttribute("type") ?? "text").toLowerCase();
      if (type === "checkbox") return "checkbox";
      if (type === "radio") return "radio";
      if (type === "range") return "slider";
      if (type === "button" || type === "submit" || type === "reset") return "button";
      if (type === "search") return "searchbox";
      if (type === "hidden") return null;
      return "textbox";
    }
    default:
      return null;
  }
}

export function roleOf(element: Element): string | null {
  const explicit = element.getAttribute("role")?.trim().split(/\s+/)[0];
  return explicit || implicitRole(element);
}

const squash = (text: string | null | undefined) => (text ?? "").replace(/\s+/g, " ").trim();

function labelled(element: Element): string {
  const own = element.ownerDocument;
  const id = element.getAttribute("id");
  const byFor = id ? own.querySelector(`label[for="${CSS.escape(id)}"]`) : null;
  const wrapping = element.closest("label");
  return squash((byFor ?? wrapping)?.textContent);
}

export function nameOf(element: Element): string {
  const ids = element.getAttribute("aria-labelledby");
  if (ids) {
    const named = ids
      .split(/\s+/)
      .map((id) => squash(element.ownerDocument.getElementById(id)?.textContent))
      .filter(Boolean)
      .join(" ");
    if (named) return named;
  }
  const label = squash(element.getAttribute("aria-label"));
  if (label) return label;
  const tag = element.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") {
    const field =
      labelled(element) ||
      squash(element.getAttribute("placeholder")) ||
      squash(element.getAttribute("title"));
    if (field) return field;
  }
  if (tag === "img") return squash(element.getAttribute("alt"));
  return squash(element.textContent) || squash(element.getAttribute("title"));
}

export function shown(element: Element): boolean {
  if (element.closest("[hidden], [aria-hidden='true'], [inert]")) return false;
  const check = (element as Element & { checkVisibility?: (o?: object) => boolean })
    .checkVisibility;
  if (typeof check === "function") return check.call(element, { visibilityProperty: true });
  for (let at: Element | null = element; at; at = at.parentElement) {
    const style = at.ownerDocument.defaultView?.getComputedStyle(at);
    if (style?.display === "none" || style?.visibility === "hidden") return false;
  }
  return true;
}

export function findAll(root: ParentNode, role: string, name?: string): Element[] {
  return Array.from(root.querySelectorAll("*")).filter(
    (element) =>
      roleOf(element) === role &&
      shown(element) &&
      (name === undefined || nameOf(element) === squash(name)),
  );
}

function only(root: ParentNode, role: string, name: string): Element {
  const found = findAll(root, role, name);
  if (found.length === 1) return found[0] as Element;
  if (found.length === 0) {
    const near = findAll(root, role).map(nameOf).filter(Boolean).slice(0, 12);
    throw new Error(
      `no ${role} named "${name}" is shown${near.length ? `; shown: ${near.map((n) => `"${n}"`).join(", ")}` : ""}`,
    );
  }
  throw new Error(`${found.length} of the ${role}s shown are named "${name}"`);
}

function disabled(element: Element): boolean {
  return element.matches(":disabled") || element.closest("[aria-disabled='true']") !== null;
}

function press(element: Element): void {
  if (disabled(element)) throw new Error(`"${nameOf(element)}" is disabled`);
  const view = element.ownerDocument.defaultView ?? window;
  const pointer = typeof view.PointerEvent === "function" ? view.PointerEvent : view.MouseEvent;
  const init = { bubbles: true, cancelable: true, composed: true, button: 0 };
  element.dispatchEvent(new pointer("pointerdown", init));
  element.dispatchEvent(new view.MouseEvent("mousedown", init));
  if (element instanceof view.HTMLElement) element.focus();
  element.dispatchEvent(new pointer("pointerup", init));
  element.dispatchEvent(new view.MouseEvent("mouseup", init));
  element.dispatchEvent(new view.MouseEvent("click", init));
}

function typeInto(element: Element, text: string): void {
  if (disabled(element)) throw new Error(`"${nameOf(element)}" is disabled`);
  const view = element.ownerDocument.defaultView ?? window;
  const proto =
    element instanceof view.HTMLTextAreaElement
      ? view.HTMLTextAreaElement.prototype
      : element instanceof view.HTMLInputElement
        ? view.HTMLInputElement.prototype
        : null;
  if (!proto) throw new Error(`"${nameOf(element)}" is not a field that takes text`);
  (element as HTMLElement).focus();
  Object.getOwnPropertyDescriptor(proto, "value")?.set?.call(element, text);
  element.dispatchEvent(new view.Event("input", { bubbles: true }));
}

function key(document: Document, name: string): void {
  const target = document.activeElement ?? document.body;
  const view = document.defaultView ?? window;
  for (const type of ["keydown", "keyup"]) {
    target.dispatchEvent(
      new view.KeyboardEvent(type, { key: name, bubbles: true, cancelable: true }),
    );
  }
}

function box(element: Element) {
  const view = element.ownerDocument.defaultView ?? window;
  const rect = element.getBoundingClientRect();
  const node = element as HTMLElement;
  return {
    left: rect.left,
    right: rect.right,
    top: rect.top,
    bottom: rect.bottom,
    width: rect.width,
    gutter:
      node.offsetWidth -
      node.clientWidth -
      node.clientLeft -
      Number.parseFloat(view.getComputedStyle(element).borderRightWidth || "0"),
    viewport: { width: view.innerWidth, height: view.innerHeight },
  };
}

function ring(element: Element): { ring: boolean; clipped: boolean } {
  const view = element.ownerDocument.defaultView ?? window;
  const style = view.getComputedStyle(element);
  const width = Number.parseFloat(style.outlineWidth || "0");
  if (style.outlineStyle === "none" || !(width > 0)) return { ring: false, clipped: false };
  const reach = Number.parseFloat(style.outlineOffset || "0") + width;
  const rect = element.getBoundingClientRect();
  for (let at = element.parentElement; at; at = at.parentElement) {
    const clips = view.getComputedStyle(at);
    if (clips.overflowX === "visible" && clips.overflowY === "visible") continue;
    const edge = at.getBoundingClientRect();
    const left = edge.left + at.clientLeft;
    const top = edge.top + at.clientTop;
    if (
      rect.left - reach < left - 0.5 ||
      rect.right + reach > left + at.clientWidth + 0.5 ||
      rect.top - reach < top - 0.5 ||
      rect.bottom + reach > top + at.clientHeight + 0.5
    ) {
      return { ring: true, clipped: true };
    }
  }
  return { ring: true, clipped: false };
}

function focused(document: Document) {
  const element = document.activeElement;
  if (!element || element === document.body) return null;
  return { role: roleOf(element), name: nameOf(element), ...ring(element) };
}

function view() {
  const state = useStore.getState();
  return {
    selected: state.selected,
    agents: state.agents.filter((agent) => agent.lifecycle !== "terminated").map((a) => a.name),
  };
}

export function run(call: Call, document: Document = window.document): Answer {
  try {
    switch (call.do) {
      case "ready":
        return {
          ok: true,
          value: {
            page: document.location?.pathname ?? null,
            loaded: document.readyState,
            attached: attached() !== null,
            since: performance.timeOrigin,
          },
        };
      case "view":
        return { ok: true, value: view() };
      case "diagnostic":
        return { ok: true, value: diagnosticRows() };
      case "attach":
        saveConnection({ origin: call.origin, token: call.token }, "remote");
        setTimeout(() => document.location.reload(), 0);
        return { ok: true, value: null };
      case "find": {
        const found = findAll(document, call.role, call.name);
        return { ok: true, value: { count: found.length, names: found.map(nameOf) } };
      }
      case "click":
        press(only(document, call.role, call.name));
        return { ok: true, value: null };
      case "type":
        typeInto(only(document, call.role, call.name), call.text);
        return { ok: true, value: null };
      case "key":
        key(document, call.key);
        return { ok: true, value: null };
      case "text": {
        const scope =
          call.role !== undefined && call.name !== undefined
            ? only(document, call.role, call.name)
            : document.body;
        const element = scope as HTMLElement;
        // biome-ignore lint/performance/useDomNodeTextContent: the question is what a person can read, and only innerText leaves hidden text out.
        return { ok: true, value: squash(element.innerText ?? element.textContent) };
      }
      case "box":
        return { ok: true, value: box(only(document, call.role, call.name)) };
      case "focus":
        return { ok: true, value: focused(document) };
      default:
        return { ok: false, error: `no such call: ${JSON.stringify(call)}` };
    }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
}

export function install(): void {
  (window as Window & { __guacaProbe?: { run: typeof run } }).__guacaProbe = { run };
}
