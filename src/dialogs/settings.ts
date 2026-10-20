import { applyTheme } from "../app";
import { backend } from "../services/backend";
import { checkForAppUpdateInteractive } from "../services/updater";
import { preferences, store } from "../state/store";
import type { Preferences, Theme } from "../types";
import { field, segmented, selectInput, toggleSwitch } from "../ui/controls";
import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { toastError } from "../ui/toast";

const AUTO_LOCK_CHOICES = [0, 1, 2, 5, 10, 15, 30, 60, 120, 240];
const CLIPBOARD_CHOICES = [0, 10, 20, 30, 45, 60, 90, 120];

function withCurrent(values: number[], current: number): number[] {
  return values.includes(current) ? values : [...values, current].sort((a, b) => a - b);
}

function minutesLabel(minutes: number): string {
  if (minutes === 0) return "Never";
  if (minutes % 60 === 0) return minutes === 60 ? "1 hour" : `${minutes / 60} hours`;
  return minutes === 1 ? "1 minute" : `${minutes} minutes`;
}

function secondsLabel(seconds: number): string {
  return seconds === 0 ? "Never" : `${seconds} seconds`;
}

export async function openSettings(): Promise<void> {
  const save = async (change: (preferences: Preferences) => void) => {
    const updated = structuredClone(preferences());
    change(updated);
    try {
      store.status = await backend.updatePreferences(updated);
      applyTheme(store.status.preferences.theme);
    } catch (error) {
      toastError(error);
    }
  };
  const current = preferences();

  const theme = segmented<Theme>(
    [
      { value: "system", label: "System" },
      { value: "light", label: "Light" },
      { value: "dark", label: "Dark" },
    ],
    current.theme,
    (value) => {
      applyTheme(value);
      void save((preferences) => (preferences.theme = value));
    },
  );

  const autoLock = selectInput(
    withCurrent(AUTO_LOCK_CHOICES, current.autoLockMinutes).map((value) => ({ value, label: minutesLabel(value) })),
    current.autoLockMinutes,
  );
  autoLock.addEventListener("change", () => void save((preferences) => (preferences.autoLockMinutes = Number(autoLock.value))));

  const clipboard = selectInput(
    withCurrent(CLIPBOARD_CHOICES, current.clipboardClearSeconds).map((value) => ({ value, label: secondsLabel(value) })),
    current.clipboardClearSeconds,
  );
  clipboard.addEventListener("change", () => void save((preferences) => (preferences.clipboardClearSeconds = Number(clipboard.value))));

  const tray = toggleSwitch(
    "Keep running in the system tray",
    current.closeToTray,
    (checked) => void save((preferences) => (preferences.closeToTray = checked)),
    "Closing the window hides it; the vault keeps locking itself as usual.",
  );

  await showDialog<true>({
    title: "Settings",
    size: "medium",
    body: h(
      "div",
      { class: "settings" },
      h("section", { class: "settings-section" }, h("h3", { text: "Appearance" }), field("Theme", theme)),
      h(
        "section",
        { class: "settings-section" },
        h("h3", { text: "Security" }),
        field("Lock the vault after this long without activity", autoLock),
        field("Clear copied passwords from the clipboard after", clipboard, {
          hint: "Copied passwords are also kept out of the Windows clipboard history.",
        }),
      ),
      h("section", { class: "settings-section" }, h("h3", { text: "Window" }), tray),
      h(
        "section",
        { class: "settings-section" },
        h("h3", { text: "Updates" }),
        h(
          "p",
          { class: "muted", text: "Vault files never leave this computer. Optional update checks only contact GitHub Releases." },
        ),
        h(
          "button",
          {
            type: "button",
            class: "button",
            onClick: () => void checkForAppUpdateInteractive().catch(toastError),
          },
          "Check for updates",
        ),
      ),
      h(
        "p",
        { class: "settings-about" },
        `Lados ${store.status?.appVersion ?? ""} · Vaults are encrypted with XChaCha20-Poly1305 and a key derived with Argon2id.`,
      ),
    ),
    hideCancel: true,
    submitLabel: "Done",
    onSubmit: () => true,
  });
}
