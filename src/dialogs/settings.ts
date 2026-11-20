import { applyTheme } from "../app";
import { extensionInstallDialog, openBrowserDownload, openExtensionStore } from "./browser-extension";
import { backend } from "../services/backend";
import { checkForAppUpdateInteractive } from "../services/updater";
import { preferences, store } from "../state/store";
import type { AutofillReveal, Preferences, Theme } from "../types";
import { field, segmented, selectInput, toggleSwitch } from "../ui/controls";
import { h } from "../ui/dom";
import { svgIcon } from "../ui/icons";
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

  // The pairing code the extension must be given. It already exists (the app creates it at start),
  // so failing here only hides the section rather than breaking Settings.
  const autofillCode = mobile ? null : await backend.autofillPairingToken().catch(() => null);
  const autofillCodeText = h("p", { class: "path-box", text: autofillCode ?? "" });
  const regenerateAutofillCode = async () => {
    try {
      autofillCodeText.textContent = await backend.autofillPairingToken(true);
      toast("New pairing code created — pair the extension again", "success", 9000);
    } catch (error) {
      toastError(error);
    }
  };
  const revealMode = segmented<AutofillReveal>(
    [
      { value: "always", label: "Ask every time" },
      { value: "oncePerSite", label: "Ask once per entry" },
      { value: "never", label: "Never ask" },
    ],
    current.autofillReveal,
    (value) => void save((preferences) => (preferences.autofillReveal = value)),
  );

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
          hint:
            store.status?.os === "windows"
              ? "Copied passwords are also kept out of the Windows clipboard history."
              : "Some clipboard managers keep their own history; Lados cannot clear those copies.",
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
            h("p", {
              class: "muted",
              text: "The Lados extension fills login forms from this vault while Lados is running and the vault is unlocked. The same extension runs in Firefox, Chrome and Edge.",
            }),
            h(
              "p",
              { class: "muted" },
              browsers?.firefox
                ? [
                    h(
                      "button",
                      { type: "button", class: "link-button", onClick: () => void openExtensionStore("firefox") },
                      "Add it in Firefox (addons.mozilla.org)",
                    ),
                    ".",
                  ]
                : [
                    h(
                      "button",
                      { type: "button", class: "link-button", onClick: () => void openBrowserDownload("firefox") },
                      "Get Firefox",
                    ),
                    " to add it there.",
                  ],
            ),
            h("p", {
              class: "muted",
              text: "The Chrome and Edge builds are in their stores' review; Lados will link them here once they are published.",
            }),
            h(
              "button",
              { type: "button", class: "button", onClick: () => void extensionInstallDialog() },
              "Install the browser extension",
            ),
          ),
      mobile || !autofillCode
        ? null
        : h(
            "section",
            { class: "settings-section" },
            h("h3", { text: "Browser pairing" }),
            h("p", {
              class: "muted",
              text: "Paste this code once into the Lados extension. Without it the extension cannot reach this app — and neither can a web page, another extension, or a script on this computer. Every password the extension asks for must still be allowed here first.",
            }),
            autofillCodeText,
            h(
              "div",
              { class: "browser-choices" },
              h(
                "button",
                {
                  type: "button",
                  class: "button",
                  onClick: () =>
                    void backend
                      .copySecretText(autofillCode)
                      .then(() => toast("Pairing code copied"))
                      .catch(toastError),
                },
                "Copy the code",
              ),
              h(
                "button",
                { type: "button", class: "button", onClick: () => void regenerateAutofillCode() },
                "New code",
              ),
            ),
            field("When the extension asks for a password", revealMode, {
              hint: "“Ask every time” is the safest: a program on this computer cannot answer the prompt.",
            }),
          ),
      h(
        "section",
        { class: "settings-section" },
        h("h3", { text: "Updates" }),
        store.status?.channel
          ? h(
              "div",
              { class: "notice warning" },
              svgIcon("warning"),
              h("span", {
                text: `This is a ${store.status.channel} test build, so updates are disabled: it must stay pointed at its test server.`,
              }),
            )
          : h("p", {
              class: "muted",
              text: "Vault files never leave this computer. Optional update checks only contact GitHub Releases.",
            }),
        store.status?.channel
          ? null
          : h(
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
        `Lados ${store.status?.appVersion ?? ""} · build ${
          store.status?.builtAt ? new Date(store.status.builtAt).toLocaleString() : "unknown"
        }${
          store.status?.channel ? ` · ${store.status.channel} · ${store.status.syncServer ?? ""}` : ""
        } · Vaults are encrypted with XChaCha20-Poly1305 and a key derived with Argon2id.`,
      ),
    ),
    hideCancel: true,
    submitLabel: "Done",
    onSubmit: () => true,
  });
}
