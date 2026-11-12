// The dedicated "Lados Sync" space: the account, the synced vaults and the subscription.
// It is deliberately separate from Settings, because it is not a preference.

import { performSync } from "../app";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { SyncVaultView } from "../types";
import { field, keyFileField, passwordInput } from "../ui/controls";
import { h, mount, type Child } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog, showDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { relativeTime } from "../utils/format";
import { credentialsDialog } from "./credentials";
import { describeChanges, hasChanges } from "./reports";

/** Opens the sync space: the sign-in form when signed out, the management view otherwise. */
export async function openSyncCenter(): Promise<void> {
  const container = h("div", { class: "stack sync-center" });
  const refresh = () => {
    const sync = store.status?.sync;
    mount(container, sync?.loggedIn ? signedInSections(refresh) : signInSection(refresh));
  };
  refresh();
  await showDialog<true>({
    title: "Lados Sync",
    size: "medium",
    body: container,
    hideCancel: true,
    submitLabel: "Done",
    onSubmit: () => true,
  });
}

/** The form shown when no account is signed in on this device. */
function signInSection(refresh: () => void): HTMLElement {
  const email = h("input", {
    class: "input",
    type: "email",
    placeholder: "you@example.com",
    autocomplete: "off",
    spellcheck: false,
  });
  const password = passwordInput({ placeholder: "Master password" });
  const keyFile = keyFileField({});
  const submit = h("button", { type: "button", class: "button primary" }, "Sign in or create account");
  submit.addEventListener("click", async () => {
    if (!email.value.trim()) {
      email.focus();
      return;
    }
    if (!password.value()) {
      password.input.focus();
      return;
    }
    submit.disabled = true;
    try {
      store.status = await backend.syncSignIn(email.value.trim(), password.value(), keyFile.value());
      toast("Signed in to Lados Sync", "success");
      refresh();
    } catch (error) {
      toastError(error);
      submit.disabled = false;
    }
  });

  return h(
    "div",
    { class: "stack" },
    h("p", {
      class: "muted",
      text: "Sign in to keep a vault in sync across your devices. Your vault is encrypted on this device before it is uploaded; only your master password opens it. A new email creates a new account.",
    }),
    field("Email", email),
    field("Master password", password.element, { target: password.input }),
    field("Key file", keyFile.element, { hint: "Only if the vault was created with one." }),
    h("div", { class: "sync-center-actions" }, submit),
  );
}

/** The account, vault and subscription sections shown to a signed-in user. */
function signedInSections(refresh: () => void): Child[] {
  const sync = store.status?.sync;
  if (!sync?.loggedIn) return [];
  const openPath = store.status?.state === "unlocked" ? store.status.vaultPath : null;

  const run = async (action: () => Promise<void>) => {
    try {
      await action();
      refresh();
    } catch (error) {
      toastError(error);
    }
  };

  const syncVault = (vault: SyncVaultView) =>
    run(async () => {
      if (openPath && vault.path === openPath) {
        store.status = await backend.syncEnable();
      } else {
        const status = await credentialsDialog({
          title: vault.synced ? "Sync now" : "Sync this vault",
          message: `Enter the master password of “${vault.fileName}”.`,
          submitLabel: "Sync",
          busyLabel: "Syncing…",
          initialKeyFile: vault.keyFile,
          action: ({ password, keyFile }) => backend.syncEnableFile(vault.path, password, keyFile),
        });
        if (status === null) return;
        store.status = status;
      }
      toast("This vault is now synced", "success");
    });

  const stopSyncing = (vault: SyncVaultView) =>
    run(async () => {
      store.status =
        openPath && vault.path === openPath
          ? await backend.syncDisable()
          : await backend.syncDisableFile(vault.path);
      toast("Stopped syncing this vault");
    });

  const syncNowOpen = () =>
    run(async () => {
      const report = await performSync();
      toast(
        hasChanges(report) ? `Synced: ${describeChanges(report)}` : "Everything is up to date",
        hasChanges(report) ? "success" : "info",
      );
    });

  const rows = sync.vaults.map((vault) => {
    const isOpen = !!openPath && vault.path === openPath;
    const when = vault.synced && vault.lastSyncedAt ? ` · ${relativeTime(vault.lastSyncedAt)}` : "";
    const actions = vault.synced
      ? [
          h(
            "button",
            { type: "button", class: "button small", onClick: () => void (isOpen ? syncNowOpen() : syncVault(vault)) },
            "Sync now",
          ),
          h("button", { type: "button", class: "button small", onClick: () => void stopSyncing(vault) }, "Stop syncing"),
        ]
      : [h("button", { type: "button", class: "button small primary", onClick: () => void syncVault(vault) }, "Sync this vault")];
    return h(
      "li",
      { class: "sync-vault-row" },
      svgIcon(vault.synced ? "cloudCheck" : "cloud", "icon sync-vault-icon"),
      h(
        "div",
        { class: "sync-vault-text" },
        h("span", { class: "sync-vault-name truncate", text: vault.fileName }),
        h("span", { class: ["sync-vault-status", vault.synced && "synced"], text: vault.synced ? `Synced${when}` : "Not synced" }),
      ),
      h("div", { class: "sync-vault-actions" }, actions),
    );
  });

  return [
    h(
      "section",
      { class: "sync-center-section" },
      h("h3", { text: "Account" }),
      h("p", { class: "muted", text: `Signed in as ${sync.email}` }),
      h(
        "div",
        { class: "sync-center-actions" },
        h("button", { type: "button", class: "button", disabled: true, title: "Subscriptions are coming soon" }, "Manage subscription"),
        h("button", { type: "button", class: "button", onClick: () => void signOut(refresh) }, "Sign out"),
        h("button", { type: "button", class: "button danger", onClick: () => void removeAccount(refresh) }, "Delete account"),
      ),
    ),
    h(
      "section",
      { class: "sync-center-section" },
      h("h3", { text: "Vaults" }),
      rows.length > 0
        ? h("ul", { class: "sync-vault-list" }, rows)
        : h("p", { class: "muted", text: "No vaults yet." }),
      h("p", {
        class: "muted",
        text: "Syncing keeps the vault on this device and uploads an encrypted copy after every save; it is merged on your device, never on the server.",
      }),
    ),
  ];
}

async function signOut(refresh: () => void): Promise<void> {
  try {
    store.status = await backend.syncSignOut();
    toast("Signed out");
    refresh();
  } catch (error) {
    toastError(error);
  }
}

async function removeAccount(refresh: () => void): Promise<void> {
  const confirmed = await confirmDialog({
    title: "Delete your Lados Sync account?",
    message:
      "This permanently deletes your account and the encrypted copies stored on the server. The vaults on this device are kept. This cannot be undone.",
    confirmLabel: "Delete account",
    danger: true,
  });
  if (!confirmed) return;
  try {
    store.status = await backend.syncDeleteAccount();
    toast("Account deleted");
    refresh();
  } catch (error) {
    toastError(error);
  }
}
