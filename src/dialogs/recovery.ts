// The recovery key: creating one for the account, and using it to open every vault it covers.
//
// A recovery code is not tied to a single vault: the same code wraps the secret of every vault
// that shares the master password. One file, covering all of them. The code lives only in that
// file — Lados keeps no copy — so the copy here is blunt about what it is worth to anyone who
// finds it.

import { refreshStatus, showStatus } from "../app";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { RecoveryCheckView } from "../types";
import { fileNameOf } from "../ui/controls";
import { h, type Child } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { alertDialog, confirmDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { credentialsDialog } from "./credentials";

const KEEP_IT_SAFE =
  "Anyone who has this file can open every vault it covers without the master password. Keep it " +
  "away from the vaults and their backups: Lados keeps no copy, and cannot make another one.";

/** Which vaults the key the user is about to make will cover. */
function scope(synced: boolean): string {
  return synced
    ? "It is one key for the whole account: it will be added to every vault you sync, so keep it as safe as your master password."
    : "It will be added to every vault on this device that your master password opens.";
}

/** Creates one recovery key covering every vault the master password opens, saving the file. */
export async function createRecoveryKeyDialog(): Promise<void> {
  const synced = store.status?.sync.loggedIn ?? false;
  const replacing = store.status?.hasRecoveryKey ?? false;
  const agreed = await confirmDialog({
    title: replacing ? "Replace the recovery key?" : "Create a recovery key",
    message: replacing
      ? `A new recovery key replaces the one you have: the file you keep now stops working. ${scope(synced)} ${KEEP_IT_SAFE}`
      : `A recovery key is a long code, kept in a file, that opens your vaults if you ever forget the master password. ${scope(synced)} ${KEEP_IT_SAFE}`,
    confirmLabel: replacing ? "Replace it" : "Continue",
    danger: replacing,
  });
  if (!agreed) return;

  // Name it after the account when there is one — the key is account-wide then — and after the
  // vault otherwise. The likeliest mistake is using one vault's key on another.
  const vaultName = (store.status?.vaultFileName ?? "vault").replace(/\.lados$/i, "");
  const account = store.status?.sync.email ?? null;
  const path = await backend.pickSavePath(
    "recovery",
    "Save your recovery key",
    account ? `Lados recovery key (${account}).txt` : `Lados recovery key (${vaultName}).txt`,
  );
  if (!path) return;

  try {
    const created = await credentialsDialog({
      title: "Confirm your master password",
      message: h(
        "div",
        { class: "stack" },
        h("p", {
          text:
            "Lados needs your current master password to add the recovery key to every vault it opens. " +
            "It is used now and never stored.",
        }),
        h("p", { class: "muted", text: `Saved as: ${fileNameOf(path)}` }),
      ),
      submitLabel: "Create the recovery key",
      busyLabel: "Creating…",
      initialKeyFile: store.status?.keyFile ?? null,
      action: ({ password, keyFile }) => backend.createRecoveryKey(path, password, keyFile),
    });
    if (!created) return;
    await refreshStatus();
    const many = created.vaults.length;
    toast(
      `Recovery key saved. It opens ${many === 1 ? "1 vault" : `${many} vaults`}. Keep that file somewhere safe.`,
      "success",
      9000,
    );
    if (created.uncovered.length) {
      toast(
        `Not covered: ${created.uncovered.join(", ")}. Those synced vaults use a different master password.`,
        "error",
        12000,
      );
    }
  } catch (error) {
    toastError(error);
  }
}

/** Opens every vault a recovery key covers, then saves them under a new master password. */
export async function recoverWithKey(): Promise<void> {
  try {
    const file = await backend.pickOpenPath("recovery", "Select your recovery key file");
    if (!file) return;

    const synced = store.status?.sync.loggedIn ?? false;
    const result = await credentialsDialog({
      title: "Set a new master password",
      message: h(
        "div",
        { class: "stack" },
        h("p", {
          text:
            "Every vault this recovery key opens will be saved with the new password below. The key keeps " +
            "working, so keep the file you have.",
        }),
        h("p", { class: "muted", text: `Recovery key: ${fileNameOf(file)}` }),
        h("p", {
          class: "muted",
          text: "The key file below belongs to the new password — leave it empty unless you want to use one.",
        }),
        synced
          ? h(
              "div",
              { class: "notice" },
              svgIcon("cloud"),
              h("span", {
                text:
                  "You are signed in, so your sync account will adopt the new password and your other devices " +
                  "will be signed out.",
              }),
            )
          : h(
              "div",
              { class: "notice warning" },
              svgIcon("warning"),
              h("span", { text: "You are not signed in, so only the vaults on this device can be recovered." }),
            ),
      ),
      submitLabel: "Open the vaults",
      busyLabel: "Recovering…",
      newSecret: true,
      minStrength: 4,
      allowCreateKeyFile: true,
      initialKeyFile: store.status?.keyFile ?? null,
      action: ({ password, keyFile }) => backend.recoverVault(file, password, keyFile),
    });
    if (!result) return;
    await showStatus(result.status);
    const many = result.restored.length;
    toast(
      many ? `Opened ${many === 1 ? "1 vault" : `${many} vaults`} with the recovery key` : "Recovery key accepted",
      "success",
    );
    if (result.uncovered.length) {
      toast(`Not covered: ${result.uncovered.join(", ")}. Those vaults still need the old password.`, "error", 12000);
    }
    if (result.notice) {
      toast(result.notice, "info", 12000);
    }
  } catch (error) {
    toastError(error);
  }
}

/** Checks a recovery key file against your vaults, without opening or changing anything. */
export async function checkRecoveryKeyDialog(): Promise<void> {
  try {
    const file = await backend.pickOpenPath("recovery", "Select a recovery key file to check");
    if (!file) return;
    const check = await backend.checkRecoveryKey(file);
    await alertDialog("Recovery key check", checkBody(file, check));
  } catch (error) {
    toastError(error);
  }
}

function checkList(title: string, names: string[]): Child | null {
  if (names.length === 0) return null;
  return h(
    "div",
    { class: "stack" },
    h("p", { text: title }),
    ...names.map((name) => h("p", { class: "muted", text: `• ${name}` })),
  );
}

function checkBody(file: string, check: RecoveryCheckView): Child {
  const total = check.opens.length + check.otherKey.length + check.noKey.length;
  const verdict = check.complete
    ? h(
        "div",
        { class: "notice" },
        svgIcon("check"),
        h("span", {
          text: total === 1 ? "This key opens your vault." : `This key opens all ${total} of your vaults.`,
        }),
      )
    : h(
        "div",
        { class: "notice warning" },
        svgIcon("warning"),
        h("span", {
          text:
            total === 0
              ? "No vaults were found to check this key against."
              : "This key does not cover every vault we found.",
        }),
      );
  return h(
    "div",
    { class: "stack" },
    h("p", { class: "muted", text: `Recovery key: ${fileNameOf(file)}` }),
    verdict,
    checkList("Opens these vaults:", check.opens),
    checkList("Uses a different key:", check.otherKey),
    checkList("Has no recovery key at all:", check.noKey),
    check.complete
      ? null
      : h("p", {
          class: "muted",
          text: "Open each vault and create a recovery key, so one file covers them all.",
        }),
  );
}
