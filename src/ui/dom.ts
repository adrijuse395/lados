// A tiny element builder. User data always ends up in text nodes, never parsed as HTML.

export type Child = Node | string | number | null | undefined | false | Child[];

export type Attributes = Record<string, unknown> & {
  class?: string | (string | false | null | undefined)[];
  text?: string;
  dataset?: Record<string, string>;
  style?: string;
};

/** Names assigned as DOM properties instead of HTML attributes. */
const PROPERTIES = new Set([
  "value", "checked", "disabled", "selected", "hidden", "readOnly", "required", "multiple",
  "tabIndex", "spellcheck", "draggable", "indeterminate", "maxLength",
]);

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attributes?: Attributes | null,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (attributes) setAttributes(element, attributes);
  append(element, ...children);
  return element;
}

export function setAttributes(element: HTMLElement, attributes: Attributes): void {
  for (const [name, value] of Object.entries(attributes)) {
    if (value === undefined || value === null || value === false) continue;
    if (name === "class") {
      element.className = Array.isArray(value) ? value.filter(Boolean).join(" ") : String(value);
    } else if (name === "text") {
      element.textContent = String(value);
    } else if (name === "dataset") {
      Object.assign(element.dataset, value);
    } else if (name.startsWith("on") && typeof value === "function") {
      element.addEventListener(name.slice(2).toLowerCase(), value as EventListener);
    } else if (PROPERTIES.has(name)) {
      (element as unknown as Record<string, unknown>)[name] = value;
    } else {
      element.setAttribute(name, value === true ? "" : String(value));
    }
  }
}

export function append(parent: Node, ...children: Child[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (Array.isArray(child)) {
      append(parent, ...child);
    } else if (child instanceof Node) {
      parent.appendChild(child);
    } else {
      parent.appendChild(document.createTextNode(String(child)));
    }
  }
}

/** Replaces everything inside `container`. */
export function mount(container: Element, ...children: Child[]): void {
  container.replaceChildren();
  append(container, ...children);
}

/** Whether the keyboard focus is in a place where typing has its own meaning. */
export function isTyping(target: EventTarget | null = document.activeElement): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["checkbox", "radio", "button", "submit", "range", "color"].includes(target.type);
  }
  return false;
}
