// Guided install for the Lados Firefox extension (desktop only).
// Shown once after the vault is unlocked; also reachable from Settings.
import { backend } from "../services/backend";
import { showDialog } from "../ui/modal";
import { h } from "../ui/dom";
import { toast } from "../ui/toast";

// TODO: Replace with your real addons.mozilla.org URL after publishing the extension there (free).
const AMO_URL = "https://addons.mozilla.org/en-US/firefox/addon/lados/";

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
 * Opens the add-on page in the user's default browser.
 * A webview cannot open external URLs itself, so this goes through the Rust
 * opener command; `window.open` would silently do nothing.
 */
export async function openExtensionPage(): Promise<void> {
  try {
    await backend.openExternalUrl(AMO_URL);
    toast("Opening the Lados extension page in your browser.", "info");
  } catch (error) {
    toast("Could not open the browser. Open addons.mozilla.org and search for “Lados”.", "error");
    console.error(error);
  }
}

export async function maybeShowExtensionInstall(): Promise<void> {
  // The extension talks to the desktop app; there is nothing to install on mobile.
  if (await backend.isMobileShell().catch(() => false)) return;
  if (extensionAlreadyDismissed()) return;

  await showDialog<"install">({
    title: "Fill passwords in Firefox with Lados",
    size: "medium",
    body: h(
      "div",
      { class: "stack", style: "gap: 16px" },
      h(
        "p",
        {},
        "Install the Lados extension for Firefox to autofill login forms directly from your vault.",
      ),
      h(
        "ul",
        { class: "muted", style: "padding-left: 20px; margin: 0; line-height: 1.6" },
        h("li", {}, "No cloud, no account — the extension talks only to Lados on this computer."),
        h("li", {}, "Works on any login page where you have a saved entry."),
        h("li", {}, "You can also install it later from Settings."),
      ),
      h(
        "div",
        { class: "install-steps" },
        h("strong", {}, "Install in 2 steps:"),
        h("ol", {}, [
          h("li", {}, "Click “Install in Firefox” below — your browser opens the add-on page."),
          h("li", {}, "Click “Add to Firefox” there. Lados will then fill your logins."),
        ]),
      ),
    ),
    submitLabel: "Install in Firefox",
    cancelLabel: "Later",
    focusCancel: true,
    onSubmit: async (): Promise<"install" | undefined> => {
      await openExtensionPage();
      return "install";
    },
  });

  // Shown once: from now on the Settings entry is the place to install it.
  markExtensionDismissed();
}
