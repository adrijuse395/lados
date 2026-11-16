// Guided install for the Lados browser extension (desktop only).
// Shown once after the vault is unlocked; also reachable from Settings.
import { backend } from "../services/backend";
import type { BrowserPresence } from "../types";
import { h, type Child } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { toast } from "../ui/toast";

type BrowserId = "firefox" | "chrome" | "edge";

/**
 * Store pages per browser. Set the URL once the add-on is published there;
 * leave `null` while it is still pending review (the button shows as unavailable).
 */
const STORES: Record<BrowserId, { label: string; url: string | null }> = {
  firefox: { label: "Firefox", url: "https://addons.mozilla.org/en-US/firefox/addon/lados/" },
  chrome: { label: "Chrome", url: null },
  edge: { label: "Edge", url: null },
};

/** Where to get the browser itself, for when it is not installed yet. */
const BROWSER_DOWNLOADS: Record<BrowserId, string> = {
  firefox: "https://www.mozilla.org/firefox/new/",
  chrome: "https://www.google.com/chrome/",
  edge: "https://www.microsoft.com/edge",
};

const DISMISSED_KEY = "lados-browser-extension-dismissed";

export function extensionAlreadyDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function markExtensionDismissed(): void {
  try {
    localStorage.setItem(DISMISSED_KEY, "1");
  } catch {
    // Ignore storage errors.
  }
}

/** Opens an external page through the Rust opener (a webview cannot open URLs itself). */
async function openUrl(url: string, what: string): Promise<void> {
  try {
    await backend.openExternalUrl(url);
    toast(`Opening ${what} in your browser.`, "info");
  } catch {
    toast(`Could not open your browser. Visit ${url}`, "error");
  }
}

/** Opens a browser's published store page, or says that it is not published yet. The page opens
 *  in that browser itself (so Firefox's page does not land in Chrome, which would offer Firefox). */
export async function openExtensionStore(id: BrowserId = "firefox"): Promise<void> {
  const store = STORES[id];
  if (!store.url) {
    toast(`The ${store.label} version isn’t published yet.`, "info");
    return;
  }
  try {
    await backend.openInBrowser(id, store.url);
    toast(`Opening the ${store.label} add-on page in ${store.label}.`, "info");
  } catch {
    toast(`Could not open ${store.label}. Visit ${store.url}`, "error");
  }
}

/** Opens a browser's download page, for when it is not installed yet. */
export async function openBrowserDownload(id: BrowserId = "firefox"): Promise<void> {
  await openUrl(BROWSER_DOWNLOADS[id], `the ${STORES[id].label} download page`);
}

function actionButton(label: string, action: () => void, primary = false): HTMLElement {
  return h("button", { type: "button", class: ["button", primary ? "primary" : ""], onClick: action }, label);
}

/** The steps for Firefox, depending on whether it is already installed. */
function firefoxSteps(installed: boolean): Child[] {
  if (installed) {
    return [
      h("p", { class: "muted", text: "Firefox is ready. Add the Lados extension:" }),
      h(
        "div",
        { class: "browser-choices" },
        actionButton("Add to Firefox", () => void openExtensionStore("firefox"), true),
      ),
      h("p", {
        class: "field-hint",
        text: "The add-on page opens in Firefox — click “Add to Firefox” there and confirm. There is no file to download.",
      }),
    ];
  }
  return [
    h("p", { class: "muted", text: "You need Firefox first — it’s free:" }),
    h(
      "div",
      { class: "browser-choices" },
      actionButton("Download Firefox", () => void openBrowserDownload("firefox"), true),
    ),
    h("p", {
      class: "field-hint",
      text: "Install Firefox, then open this dialog again (Settings ▸ Install the browser extension) to add the Lados extension in one click.",
    }),
  ];
}

/** The install dialog, shared by the first-run prompt and the Settings button. */
export async function extensionInstallDialog(): Promise<void> {
  const browsers: BrowserPresence = await backend
    .detectedBrowsers()
    .catch(() => ({ firefox: false, chrome: false, edge: false }));

  await showDialog<"done">({
    title: "Autofill in your browser",
    size: "medium",
    body: h(
      "div",
      { class: "stack", style: "gap: 14px" },
      h("p", {}, "The Lados extension fills logins from your vault. It works offline, with no account."),
      ...firefoxSteps(browsers.firefox),
      h(
        "p",
        { class: "muted" },
        "The extension talks only to Lados on this computer. Chrome and Edge versions are coming soon.",
      ),
    ),
    submitLabel: "Close",
    hideCancel: true,
    onSubmit: () => "done",
  });
}

export async function maybeShowExtensionInstall(): Promise<void> {
  // The extension talks to the desktop app; there is nothing to install on mobile.
  if (await backend.isMobileShell().catch(() => false)) return;
  if (extensionAlreadyDismissed()) return;

  await extensionInstallDialog();
  // Shown once: from now on the Settings entry is the place to install it.
  markExtensionDismissed();
}
