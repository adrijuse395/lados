import { backend } from "../services/backend";
import { preferences, store } from "../state/store";
import type { GeneratorOptions } from "../types";
import { checkbox, iconButton, passwordGlyphs, strengthMeter } from "../ui/controls";
import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { errorMessage, toast, toastError } from "../ui/toast";

const MIN_LENGTH = 4;
const MAX_LENGTH = 128;

type CharacterClass = "lowercase" | "uppercase" | "digits" | "symbols";

const CLASSES: { key: CharacterClass; label: string }[] = [
  { key: "lowercase", label: "Lowercase (a-z)" },
  { key: "uppercase", label: "Uppercase (A-Z)" },
  { key: "digits", label: "Digits (0-9)" },
  { key: "symbols", label: "Symbols (!@#…)" },
];

/**
 * The password generator. With `pick`, the chosen password is returned to the caller
 * (the entry editor); otherwise it can be copied to the clipboard.
 */
export async function openGenerator(options: { pick?: boolean } = {}): Promise<string | null> {
  const settings: GeneratorOptions = { ...preferences().generator };
  let password = "";
  let saveTimer = 0;
  let generation = 0;

  const output = h("div", { class: "generated-password mono", "aria-live": "polite" });
  const meter = strengthMeter();

  const lengthRange = h("input", { class: "range", type: "range", min: MIN_LENGTH, max: MAX_LENGTH, value: String(settings.length) });
  const lengthNumber = h("input", { class: "input length-input", type: "number", min: MIN_LENGTH, max: MAX_LENGTH, value: String(settings.length) });
  const setLength = (value: number, source: HTMLInputElement) => {
    if (!Number.isFinite(value)) return;
    settings.length = Math.min(MAX_LENGTH, Math.max(MIN_LENGTH, Math.round(value)));
    if (source !== lengthRange) lengthRange.value = String(settings.length);
    if (source !== lengthNumber) lengthNumber.value = String(settings.length);
    changed();
  };
  lengthRange.addEventListener("input", () => setLength(Number(lengthRange.value), lengthRange));
  lengthNumber.addEventListener("change", () => setLength(Number(lengthNumber.value), lengthNumber));

  const classBoxes = CLASSES.map(({ key, label }) => {
    const box = checkbox(label, settings[key], (checked) => {
      // At least one kind of character must stay selected.
      if (!checked && CLASSES.every((other) => other.key === key || !settings[other.key])) {
        box.input.checked = true;
        return;
      }
      settings[key] = checked;
      changed();
    });
    return box.element;
  });
  const ambiguous = checkbox("Avoid look-alike characters (I l 1 O 0 o)", settings.excludeAmbiguous, (checked) => {
    settings.excludeAmbiguous = checked;
    changed();
  });

  async function regenerate() {
    const current = ++generation;
    try {
      const generated = await backend.generatePassword(settings);
      if (current !== generation) return;
      password = generated;
      output.replaceChildren(...passwordGlyphs(generated));
      meter.update(generated);
    } catch (error) {
      output.textContent = errorMessage(error);
    }
  }

  function changed() {
    void regenerate();
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(async () => {
      try {
        store.status = await backend.updatePreferences({ ...preferences(), generator: { ...settings } });
      } catch (error) {
        toastError(error);
      }
    }, 500);
  }

  const copy = async () => {
    await backend.copySecretText(password);
    const seconds = preferences().clipboardClearSeconds;
    toast(seconds > 0 ? `Password copied. The clipboard will be cleared in ${seconds} s` : "Password copied", "success");
  };

  await regenerate();
  const result = await showDialog<string>({
    title: "Password generator",
    size: "medium",
    body: h(
      "div",
      { class: "generator" },
      h("div", { class: "generated-box" }, output, iconButton("refresh", "Generate another", () => void regenerate())),
      meter.element,
      h("div", { class: "field" }, h("span", { class: "field-label", text: "Length" }), h("div", { class: "length-row" }, lengthRange, lengthNumber)),
      h("div", { class: "field" }, h("span", { class: "field-label", text: "Characters" }), h("div", { class: "checkbox-grid" }, classBoxes)),
      ambiguous.element,
    ),
    extraActions: options.pick ? h("button", { type: "button", class: "button", onClick: () => void copy().catch(toastError) }, "Copy") : null,
    cancelLabel: options.pick ? "Cancel" : "Close",
    submitLabel: options.pick ? "Use this password" : "Copy",
    onSubmit: async () => {
      if (!password) return undefined;
      if (!options.pick) await copy();
      return password;
    },
  });
  window.clearTimeout(saveTimer);
  return result;
}
