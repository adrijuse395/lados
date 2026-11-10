// Guided install for the Lados Firefox extension (desktop only).
// Shown once after the vault is unlocked, until the user dismisses it.
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

/** Opens the AMO page in the default browser (ideally Firefox). */
export async function openExtensionPage(): Promise<void> {
  void window.open(AMO_URL, "_blank");
  toast("Opening the Lados extension page in your browser.", "info");
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
        h("li", {}, "Your vault stays unlocked only while you use it; locking clears everything."),
      ),
      h(
        "div",
        { class: "install-steps" },
        h("strong", {}, "Install in 3 steps:"),
        h("ol", {}, [
          h("li", {}, "Click “Install in Firefox” below."),
          h("li", {}, "On addons.mozilla.org, click “Add to Firefox”."),
          h("li", {}, "Return to Lados — passwords will fill automatically."),
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
}
