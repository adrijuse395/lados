import { backend } from "../services/backend";
import { preferences, store } from "../state/store";
import type { GeneratorOptions } from "../types";
import { checkbox, field, iconButton, passwordGlyphs, strengthMeter } from "../ui/controls";
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

function withDefaults(partial: Partial<GeneratorOptions> | undefined): GeneratorOptions {
  return {
    length: 20,
    lowercase: true,
    uppercase: true,
    digits: true,
    symbols: true,
    excludeAmbiguous: false,
    spaces: false,
    excludeCharacters: "",
    beginWithLetter: false,
    noRepeating: false,
    requireEachType: true,
    minLowercase: 0,
    minUppercase: 0,
    minDigits: 0,
    minSymbols: 0,
    ...partial,
  };
}

function minInput(value: number, onChange: (value: number) => void): HTMLInputElement {
  const input = h("input", {
    class: "input length-input",
    type: "number",
    min: "0",
    max: "32",
    value: String(value),
  });
  input.addEventListener("change", () => {
    const next = Number(input.value);
    const clamped = Number.isFinite(next) ? Math.min(32, Math.max(0, Math.round(next))) : 0;
    input.value = String(clamped);
    onChange(clamped);
  });
  return input;
}

/**
 * The password generator. With `pick`, the chosen password is returned to the caller
 * (the entry editor); otherwise it can be copied to the clipboard.
 */
export async function openGenerator(options: { pick?: boolean } = {}): Promise<string | null> {
  const settings: GeneratorOptions = withDefaults(preferences().generator);
  let password = "";
  let saveTimer = 0;
  let generation = 0;

  const output = h("div", { class: "generated-password mono", "aria-live": "polite" });
  const meter = strengthMeter();

  const lengthRange = h("input", {
    class: "range",
    type: "range",
    min: MIN_LENGTH,
    max: MAX_LENGTH,
    value: String(settings.length),
  });
  const lengthNumber = h("input", {
    class: "input length-input",
    type: "number",
    min: MIN_LENGTH,
    max: MAX_LENGTH,
    value: String(settings.length),
  });
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
      // At least one kind of character must stay selected among the main classes.
      if (!checked && !settings.spaces && CLASSES.every((other) => other.key === key || !settings[other.key])) {
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

  const spaces = checkbox("Include spaces", settings.spaces, (checked) => {
    settings.spaces = checked;
    changed();
  });
  const beginWithLetter = checkbox("Begin with a letter", settings.beginWithLetter, (checked) => {
    settings.beginWithLetter = checked;
    changed();
  });
  const noRepeating = checkbox("No repeating characters", settings.noRepeating, (checked) => {
    settings.noRepeating = checked;
    changed();
  });
  const requireEachType = checkbox(
    "Require every selected character type",
    settings.requireEachType,
    (checked) => {
      settings.requireEachType = checked;
      changed();
    },
  );

  const excludeCharacters = h("input", {
    class: "input",
    type: "text",
    value: settings.excludeCharacters,
    placeholder: "e.g. @#$ or vowels to avoid",
    spellcheck: "false",
    autocomplete: "off",
  });
  excludeCharacters.addEventListener("input", () => {
    settings.excludeCharacters = excludeCharacters.value.slice(0, 256);
    changed();
  });

  const minLower = minInput(settings.minLowercase, (value) => {
    settings.minLowercase = value;
    changed();
  });
  const minUpper = minInput(settings.minUppercase, (value) => {
    settings.minUppercase = value;
    changed();
  });
  const minDigits = minInput(settings.minDigits, (value) => {
    settings.minDigits = value;
    changed();
  });
  const minSymbols = minInput(settings.minSymbols, (value) => {
    settings.minSymbols = value;
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
      password = "";
      output.textContent = errorMessage(error);
      meter.update("");
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
      h(
        "details",
        { class: "generator-advanced" },
        h("summary", { text: "Advanced options" }),
        h(
          "div",
          { class: "generator-advanced-body" },
          spaces.element,
          beginWithLetter.element,
          noRepeating.element,
          requireEachType.element,
          field("Also exclude these characters", excludeCharacters, {
            hint: "Applied on top of the look-alike filter when that option is on.",
          }),
          h(
            "div",
            { class: "field" },
            h("span", { class: "field-label", text: "Minimum of each type" }),
            h(
              "div",
              { class: "generator-mins" },
              h("label", { class: "generator-min" }, h("span", { text: "a-z" }), minLower),
              h("label", { class: "generator-min" }, h("span", { text: "A-Z" }), minUpper),
              h("label", { class: "generator-min" }, h("span", { text: "0-9" }), minDigits),
              h("label", { class: "generator-min" }, h("span", { text: "!@#" }), minSymbols),
            ),
            h("div", {
              class: "field-hint",
              text: "0 keeps the default “at least one” rule when “Require every selected character type” is on.",
            }),
          ),
        ),
      ),
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
