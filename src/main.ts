import "./styles/base.css";
import "./styles/layout.css";
import "./styles/components.css";

import { refreshStatus, reloadVault, showStatus } from "./app";
import { describeChanges, hasChanges } from "./dialogs/reports";
import { installShortcuts } from "./screens/vault";
import { backend } from "./services/backend";
import { listenToBackend } from "./services/events";
import { store } from "./state/store";
import { h, isTyping } from "./ui/dom";
import { alertDialog } from "./ui/modal";
import { errorMessage, toast, toastError } from "./ui/toast";
import { maybeOfferAppUpdate } from "./services/updater";
import { trackActivity } from "./utils/activity";

/** Keeps the web view from behaving like a browser: no page menu, no file drops that navigate away. */
function behaveLikeAnApplication(): void {
  document.addEventListener("contextmenu", (event) => {
    if (!event.defaultPrevented && !isTyping(event.target)) event.preventDefault();
  });
  window.addEventListener("dragover", (event) => {
    if (event.defaultPrevented) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
  });
  window.addEventListener("drop", (event) => event.preventDefault());
}

async function start(): Promise<void> {
  behaveLikeAnApplication();
  trackActivity();
  installShortcuts();

  await listenToBackend({
    vaultLocked(reason) {
      if (store.status?.state !== "unlocked") return;
      void refreshStatus().catch(toastError);
      if (reason === "inactivity") toast("The vault was locked after a period of inactivity");
      if (reason === "tray") toast("The vault was locked from the system tray");
    },
    vaultChanged(report) {
      if (store.status?.state !== "unlocked") return;
      void reloadVault().catch(toastError);
      if (hasChanges(report)) toast(`Changes saved in another window were merged: ${describeChanges(report)}`, "info", 6000);
    },
    vaultDiverged(copyPath) {
      void refreshStatus().catch(toastError);
      void alertDialog(
        "Your vault was saved as a copy",
        h(
          "div",
          { class: "stack" },
          h("p", {
            text: "The vault file was replaced by a version that cannot be opened with your master key, for example because the key was changed on another computer. To lose nothing, your vault was saved as a separate file, which is now the one open:",
          }),
          h("p", { class: "path-box", text: copyPath }),
          h("p", { class: "muted", text: "Once you can open both files, combine them with More › Merge another vault." }),
        ),
      );
    },
    syncError(message) {
      toast(`The vault could not be synchronized: ${message}`, "error", 6000);
    },
    clipboardCleared() {
      toast("The clipboard was cleared");
    },
  });

  await showStatus(await backend.getStatus());
  // Do not block startup on the network; offer an update only when one exists.
  void maybeOfferAppUpdate();
}

start().catch((error: unknown) => {
  document.getElementById("app")?.replaceChildren(
    h("div", { class: "fatal-error" }, h("h1", { text: "Lados could not start" }), h("p", { text: errorMessage(error) })),
  );
});
