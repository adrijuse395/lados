import "./styles/base.css";
import "./styles/layout.css";
import "./styles/components.css";

import { refreshStatus, reloadVault, showStatus } from "./app";
import { describeChanges, hasChanges } from "./dialogs/reports";
import { askAutofillReveal } from "./dialogs/autofill";
import { captureUnsavedEditor } from "./dialogs/entry-editor";
import { installShortcuts } from "./screens/vault";
import { backend } from "./services/backend";
import { listenToBackend } from "./services/events";
import { store } from "./state/store";
import { h, isTyping } from "./ui/dom";
import { alertDialog, confirmDialog } from "./ui/modal";
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
  let hiddenNoticeShown = false;

  await listenToBackend({
    vaultLocked(reason) {
      // Remember whatever the entry editor holds before its dialog is closed (H4).
      captureUnsavedEditor();
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
    vaultFileMissing(path) {
      // The file is gone and **nothing has been saved** (report M8). Offer to write it back, and do
      // not pretend a save already happened.
      void confirmDialog({
        title: "The vault file is gone",
        message: h(
          "div",
          { class: "stack" },
          h("p", {
            text: "Lados cannot find the vault file. Your data is still open in memory and has not been written anywhere since.",
          }),
          h("p", { class: "path-box", text: path }),
        ),
        confirmLabel: "Save it again here",
        danger: true,
      }).then((again) => {
        if (again) void backend.saveVaultAgain().catch(toastError);
      });
    },
    vaultRecreated(path) {
      void alertDialog(
        "Your vault was saved again",
        h(
          "div",
          { class: "stack" },
          h("p", { text: "The vault file had been deleted or moved, and Lados wrote your vault again at:" }),
          h("p", { class: "path-box", text: path }),
          h("p", { class: "muted", text: "If you meant to remove this vault, close it and delete that file." }),
        ),
      );
    },
    clipboardCleared() {
      toast("The clipboard was cleared");
    },
    autofillReveal(request) {
      void askAutofillReveal(request);
    },
    appHiddenToTray() {
      // Say where it went, once: the window disappears and the app looks closed (M17).
      if (hiddenNoticeShown) return;
      hiddenNoticeShown = true;
      toast("Lados is still running in the system tray. Use its icon to reopen or quit.", "info", 9000);
    },
  });

  await showStatus(await backend.getStatus());
  // Refresh what we know about the account, so the start screen can tell that a session ended
  // (for example, because the master password changed on another device).
  if (store.status?.sync.loggedIn) {
    void backend
      .syncRefresh()
      .then((status) => showStatus(status))
      .catch(() => undefined);
  }
  // Do not block startup on the network; offer an update only when one exists. A marked test build
  // skips this entirely: it must never replace itself with a released build.
  if (!store.status?.channel) void maybeOfferAppUpdate();
}

start().catch((error: unknown) => {
  document.getElementById("app")?.replaceChildren(
    h("div", { class: "fatal-error" }, h("h1", { text: "Lados could not start" }), h("p", { text: errorMessage(error) })),
  );
});
