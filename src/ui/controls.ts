// Reusable form controls and small view helpers.

import { backend } from "../services/backend";
import { store } from "../state/store";
import type { Icon } from "../types";
import { estimateStrength } from "../utils/strength";
import { h, type Child } from "./dom";
import { isIconName, svgIcon, type IconName } from "./icons";
import { toast, toastError } from "./toast";

let idCounter = 0;

export function uniqueId(prefix: string): string {
  return `${prefix}-${++idCounter}`;
}

export function fileNameOf(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** A labelled form row. `target` is the element that receives focus when the label is clicked. */
export function field(label: string, control: Child, options: { hint?: Child; target?: HTMLElement } = {}): HTMLElement {
  const target = options.target ?? (control instanceof HTMLElement ? control : undefined);
  if (target && !target.id) target.id = uniqueId("field");
  return h(
    "div",
    { class: "field" },
    h("label", { class: "field-label", for: target?.id, text: label }),
    control,
    options.hint ? h("div", { class: "field-hint" }, options.hint) : null,
  );
}

export function spinner(): HTMLElement {
  return h("span", { class: "spinner", "aria-hidden": "true" });
}

export function iconButton(icon: IconName, label: string, onClick: (event: MouseEvent) => void, className = ""): HTMLButtonElement {
  return h("button", { type: "button", class: ["icon-button", className], title: label, "aria-label": label, onClick }, svgIcon(icon));
}

// ----- Entry and folder icons -----

let iconCache: { source: unknown; images: Map<string, string> } = { source: null, images: new Map() };

function customIconSource(id: string): string | undefined {
  const icons = store.vault?.customIcons ?? [];
  if (iconCache.source !== icons) {
    iconCache = { source: icons, images: new Map(icons.map((icon) => [icon.id, `data:${icon.mime};base64,${icon.data}`])) };
  }
  return iconCache.images.get(id);
}

export function itemIcon(icon: Icon | null, fallback: IconName, className = "item-icon"): Element {
  if (icon?.kind === "custom") {
    const source = customIconSource(icon.value);
    if (source) return h("img", { class: [className, "custom"], src: source, alt: "", draggable: false });
  } else if (icon?.kind === "builtin" && isIconName(icon.value)) {
    return svgIcon(icon.value, `icon ${className}`);
  }
  return svgIcon(fallback, `icon ${className}`);
}

// ----- Passwords -----

/** A password with digits and symbols colored, so look-alike characters are easier to tell apart. */
export function passwordGlyphs(password: string): HTMLElement[] {
  return [...password].map((character) =>
    h("span", { class: /\d/.test(character) ? "digit" : /[a-z]/i.test(character) ? "letter" : "symbol", text: character }),
  );
}

export interface PasswordInput {
  element: HTMLElement;
  input: HTMLInputElement;
  value(): string;
  setValue(value: string): void;
}

export function passwordInput(
  options: {
    value?: string;
    placeholder?: string;
    strength?: boolean;
    autofocus?: boolean;
    onGenerate?: () => void;
    onInput?: () => void;
  } = {},
): PasswordInput {
  const input = h("input", {
    class: "input mono",
    type: "password",
    value: options.value ?? "",
    placeholder: options.placeholder,
    autocomplete: "off",
    autocapitalize: "off",
    spellcheck: false,
    "data-autofocus": options.autofocus ? "" : undefined,
  });

  const reveal = h("button", {
    type: "button",
    class: "icon-button",
    title: "Show password",
    "aria-label": "Show password",
    "aria-pressed": "false",
    onClick: () => {
      const show = input.type === "password";
      input.type = show ? "text" : "password";
      reveal.replaceChildren(svgIcon(show ? "eyeOff" : "eye"));
      reveal.title = show ? "Hide password" : "Show password";
      reveal.setAttribute("aria-label", reveal.title);
      reveal.setAttribute("aria-pressed", String(show));
    },
  }, svgIcon("eye"));

  const generate = options.onGenerate ? iconButton("dice", "Generate a password", options.onGenerate) : null;
  const meter = options.strength ? strengthMeter() : null;
  const capsLock = h("div", { class: "caps-lock", hidden: true }, svgIcon("capsLock"), "Caps Lock is on");

  input.addEventListener("input", () => {
    meter?.update(input.value);
    options.onInput?.();
  });
  const checkCapsLock = (event: KeyboardEvent) => {
    capsLock.hidden = !event.getModifierState?.("CapsLock");
  };
  input.addEventListener("keydown", checkCapsLock);
  input.addEventListener("keyup", checkCapsLock);
  input.addEventListener("blur", () => (capsLock.hidden = true));

  meter?.update(input.value);
  return {
    element: h("div", { class: "password-input" }, h("div", { class: "input-group" }, input, reveal, generate), meter?.element, capsLock),
    input,
    value: () => input.value,
    setValue: (value) => {
      input.value = value;
      meter?.update(value);
    },
  };
}

export function strengthMeter(): { element: HTMLElement; update(password: string): void } {
  const label = h("span", { class: "strength-label" });
  const element = h(
    "div",
    { class: "strength", "data-score": "none" },
    h("div", { class: "strength-bar" }, [0, 1, 2, 3, 4].map(() => h("span"))),
    label,
  );
  return {
    element,
    update(password) {
      if (!password) {
        element.dataset.score = "none";
        label.textContent = "";
        return;
      }
      const strength = estimateStrength(password);
      element.dataset.score = String(strength.score);
      label.textContent = strength.label;
    },
  };
}

// ----- Key file picker -----

export interface KeyFileField {
  element: HTMLElement;
  value(): string | null;
  setValue(path: string | null): void;
}

export function keyFileField(options: { value?: string | null; allowCreate?: boolean } = {}): KeyFileField {
  let path: string | null = null;
  const name = h("span", { class: "key-file-name" });
  const location = h("span", { class: "key-file-location" });

  const clear = iconButton("close", "Don't use a key file", () => set(null));
  const browse = h("button", {
    type: "button",
    class: "button small",
    onClick: async () => {
      try {
        const picked = await backend.pickOpenPath("keyFile", "Choose the key file");
        if (picked) set(picked);
      } catch (error) {
        toastError(error);
      }
    },
  }, "Browse…");
  const create = options.allowCreate
    ? h("button", {
        type: "button",
        class: "button small",
        onClick: async () => {
          try {
            const created = await backend.generateKeyFile();
            if (created) {
              set(created);
              toast("Key file created. Keep a copy somewhere safe: the vault cannot be opened without it.", "success", 7000);
            }
          } catch (error) {
            toastError(error);
          }
        },
      }, "Create new…")
    : null;

  function set(value: string | null) {
    path = value;
    name.textContent = value ? fileNameOf(value) : "No key file";
    location.textContent = value ?? "";
    location.hidden = !value;
    element.title = value ?? "";
    element.classList.toggle("empty", !value);
    clear.hidden = !value;
  }

  const element = h(
    "div",
    { class: "key-file-field" },
    svgIcon("keyFile"),
    h("div", { class: "key-file-text" }, name, location),
    clear,
    browse,
    create,
  );
  set(options.value ?? null);
  return { element, value: () => path, setValue: set };
}

// ----- Choice controls -----

export function checkbox(label: string, checked: boolean, onChange?: (checked: boolean) => void): { element: HTMLElement; input: HTMLInputElement } {
  const input = h("input", { type: "checkbox", checked, onChange: () => onChange?.(input.checked) });
  return { element: h("label", { class: "checkbox" }, input, h("span", { text: label })), input };
}

export function toggleSwitch(label: string, checked: boolean, onChange: (checked: boolean) => void, hint?: string): HTMLElement {
  const input = h("input", { type: "checkbox", role: "switch", checked, onChange: () => onChange(input.checked) });
  return h(
    "label",
    { class: "switch-row" },
    h("span", { class: "switch-text" }, h("span", { text: label }), hint ? h("span", { class: "field-hint", text: hint }) : null),
    h("span", { class: "switch" }, input, h("span", { class: "switch-track" })),
  );
}

export function segmented<T extends string>(
  choices: { value: T; label: string }[],
  current: T,
  onChange: (value: T) => void,
): HTMLElement {
  const buttons = choices.map((choice) =>
    h("button", {
      type: "button",
      class: ["segment", choice.value === current && "active"],
      role: "radio",
      "aria-checked": String(choice.value === current),
      onClick: () => {
        for (const button of buttons) {
          const active = button === buttons[choices.indexOf(choice)];
          button.classList.toggle("active", active);
          button.setAttribute("aria-checked", String(active));
        }
        onChange(choice.value);
      },
    }, choice.label),
  );
  return h("div", { class: "segmented", role: "radiogroup" }, buttons);
}

export function selectInput<T extends string | number>(choices: { value: T; label: string }[], current: T): HTMLSelectElement {
  return h(
    "select",
    { class: "input" },
    choices.map((choice) => h("option", { value: String(choice.value), selected: choice.value === current, text: choice.label })),
  );
}
