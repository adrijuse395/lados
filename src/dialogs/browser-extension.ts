// Guided install for the Lados browser extension (desktop only).
// Shown once after the vault is unlocked; also reachable from Settings.
import { backend } from "../services/backend";
import { showDialog } from "../ui/modal";
import { h } from "../ui/dom";
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

/**
 * Opens the add-on page in the user's default browser. A webview cannot open
 * external URLs itself, so this goes through the Rust opener command.
 */
async function openStore(id: BrowserId): Promise<void> {
  const store = STORES[id];
  if (!store.url) {
    toast(`The ${store.label} version isn’t published yet.`, "info");
    return;
  }
  try {
    await backend.openExternalUrl(store.url);
    toast(`Opening the ${store.label} add-on page in your browser.`, "info");
  } catch {
    toast("Could not open your browser. Visit addons.mozilla.org and search for “Lados”.", "error");
  }
}

function browserButton(id: BrowserId): HTMLElement {
  const store = STORES[id];
  const available = store.url !== null;
  return h(
    "button",
    {
      type: "button",
      class: ["button", available ? "primary" : ""],
      disabled: available ? undefined : true,
      title: available ? "" : "Not published yet",
      onClick: () => void openStore(id),
    },
    store.label,
  );
}

/** The install dialog, shared by the first-run prompt and the Settings button. */
export async function extensionInstallDialog(): Promise<void> {
  await showDialog<"done">({
    title: "Fill passwords in your browser",
    size: "medium",
    body: h(
      "div",
      { class: "stack", style: "gap: 14px" },
      h("p", {}, "Install the Lados extension to autofill logins from your vault. Choose your browser:"),
      h("div", { class: "browser-choices" }, [browserButton("firefox"), browserButton("chrome"), browserButton("edge")]),
      h(
        "div",
        { class: "install-steps" },
        h("strong", {}, "Then:"),
        h("ol", {}, [
          h("li", {}, "Your browser opens the add-on page — click “Add to Firefox / Chrome / Edge”."),
          h("li", {}, "Return to Lados. Passwords will fill on sites where you have an entry."),
        ]),
      ),
      h(
        "p",
        { class: "muted" },
        "No cloud, no account — the extension talks only to Lados on this computer.",
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
