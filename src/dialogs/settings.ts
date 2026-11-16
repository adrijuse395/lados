import { applyTheme } from "../app";
import { extensionInstallDialog, openBrowserDownload, openExtensionStore } from "./browser-extension";
import { backend } from "../services/backend";
import { checkForAppUpdateInteractive } from "../services/updater";
import { preferences, store } from "../state/store";
import type { Preferences, Theme } from "../types";
import { field, segmented, selectInput, toggleSwitch } from "../ui/controls";
import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";

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
  const mobile = await backend.isMobileShell().catch(() => false);
  const autostart = mobile ? false : await backend.getAutostart().catch(() => false);
  const browsers = mobile ? null : await backend.detectedBrowsers().catch(() => null);

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

  const startup = toggleSwitch(
    "Start Lados when you sign in",
    autostart,
    (checked) => {
      void backend.setAutostart(checked).catch(toastError);
    },
    "Opens in the background so browser autofill is ready without launching it first.",
  );

  const vaultsFolderText = h("code", { class: "muted", text: await backend.vaultsFolder().catch(() => "") });
  const changeVaultsFolder = h(
    "button",
    {
      type: "button",
      class: "button small",
      onClick: async () => {
        try {
          const picked = await backend.pickFolder("Choose the folder for new vaults");
          if (!picked) return;
          await save((preferences) => (preferences.vaultsFolder = picked));
          vaultsFolderText.textContent = await backend.vaultsFolder().catch(() => picked);
          toast("Vaults folder changed");
        } catch (error) {
          toastError(error);
        }
      },
    },
    "Change",
  );
  const resetVaultsFolder = h(
    "button",
    {
      type: "button",
      class: "button small",
      onClick: async () => {
        try {
          await save((preferences) => (preferences.vaultsFolder = null));
          vaultsFolderText.textContent = await backend.vaultsFolder().catch(() => "");
          toast("Using the default folder");
        } catch (error) {
          toastError(error);
        }
      },
    },
    "Reset",
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
      h("section", { class: "settings-section" }, h("h3", { text: "Window" }), tray, mobile ? null : startup),
      h(
        "section",
        { class: "settings-section" },
        h("h3", { text: "Vaults" }),
        h("p", {
          class: "muted",
          text: "New vaults are created here, and cloud vaults are downloaded here. You can change it.",
        }),
        field(
          "Folder",
          h(
            "div",
            { class: "stack", style: "gap: 8px" },
            vaultsFolderText,
            h("div", { class: "browser-choices" }, changeVaultsFolder, resetVaultsFolder),
          ),
        ),
      ),
      mobile
        ? null
        : h(
            "section",
            { class: "settings-section" },
            h("h3", { text: "Browser fill (desktop)" }),
            browsers?.firefox
              ? h(
                  "p",
                  { class: "muted" },
                  "Firefox is installed. The Lados extension fills login forms from this vault while Lados is running and the vault is unlocked. ",
                  h(
                    "button",
                    { type: "button", class: "link-button", onClick: () => void openExtensionStore("firefox") },
                    "Add the extension from addons.mozilla.org",
                  ),
                  ".",
                )
              : h(
                  "p",
                  { class: "muted" },
                  "The Lados extension fills login forms from this vault while Lados is running. You’ll need Firefox first — ",
                  h(
                    "button",
                    { type: "button", class: "link-button", onClick: () => void openBrowserDownload("firefox") },
                    "download Firefox",
                  ),
                  ", then open this again to add the extension in one click.",
                ),
            h(
              "button",
              { type: "button", class: "button", onClick: () => void extensionInstallDialog() },
              "Install the browser extension",
            ),
          ),
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
