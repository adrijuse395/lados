// The recovery key: creating one, and using one to open a vault whose master password is lost.
//
// The code lives only in the file the user saves — Lados keeps no copy — so the copy here is
// blunt about what that file is worth to anyone who finds it.

import { refreshStatus, showStatus } from "../app";
import { backend } from "../services/backend";
import { store } from "../state/store";
import { fileNameOf } from "../ui/controls";
import { h } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { credentialsDialog } from "./credentials";

const KEEP_IT_SAFE =
  "Anyone who has this file can open the vault without the master password. Keep it away from the " +
  "vault and its backups: Lados keeps no copy, and cannot make another one.";

/** Creates a recovery key for the open vault, saving the file the user keeps. */
export async function createRecoveryKeyDialog(): Promise<void> {
  const replacing = store.status?.hasRecoveryKey ?? false;
  const agreed = await confirmDialog({
    title: replacing ? "Replace the recovery key?" : "Create a recovery key",
    message: replacing
      ? `This vault already has a recovery key. A new one replaces it, and the file you have now stops working. ${KEEP_IT_SAFE}`
      : `A recovery key is a long code, kept in a file, that opens this vault if you ever forget the master password. ${KEEP_IT_SAFE}`,
    confirmLabel: replacing ? "Replace it" : "Choose where to save it",
    danger: replacing,
  });
  if (!agreed) return;

  try {
    const path = await backend.pickSavePath("recovery", "Save your recovery key", "Lados recovery key.txt");
    if (!path) return;
    await backend.createRecoveryKey(path);
    await refreshStatus();
    toast("Recovery key saved. Keep that file somewhere safe.", "success", 9000);
  } catch (error) {
    toastError(error);
  }
}

/** Opens the selected vault with a recovery key, then sets a new master password on it. */
export async function recoverWithKey(): Promise<void> {
  try {
    const file = await backend.pickOpenPath("recovery", "Select your recovery key file");
    if (!file) return;

    const synced = store.status?.sync.loggedIn ?? false;
    const status = await credentialsDialog({
      title: "Set a new master password",
      message: h(
        "div",
        { class: "stack" },
        h("p", {
          text:
            "The vault will be opened with the recovery key you chose, then encrypted again with the new " +
            "password below. The recovery key keeps working, so keep the file you have.",
        }),
        h("p", { class: "muted", text: `Recovery key: ${fileNameOf(file)}` }),
        h("p", {
          class: "muted",
          text: "The key file below belongs to the new password — leave it empty unless you want to use one.",
        }),
        synced
          ? h(
              "div",
              { class: "notice warning" },
              svgIcon("warning"),
              h("span", {
                text:
                  "This vault is synced, and your sync account still uses the old password. Syncing resumes " +
                  "once you sign in and change it from Sync settings.",
              }),
            )
          : null,
      ),
      submitLabel: "Open the vault",
      busyLabel: "Opening…",
      newSecret: true,
      minStrength: 4,
      allowCreateKeyFile: true,
      initialKeyFile: store.status?.keyFile ?? null,
      action: ({ password, keyFile }) => backend.recoverVault(file, password, keyFile),
    });
    if (!status) return;
    await showStatus(status);
    toast("Vault opened with the recovery key", "success");
  } catch (error) {
    toastError(error);
  }
}
