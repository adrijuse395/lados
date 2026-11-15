// The dedicated "Lados Sync" space: the account, the cloud vaults and the local vaults.
// It is deliberately separate from Settings, because it is not a preference.

import { performSync } from "../app";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { AppStatus, CloudVault, LocalVaultView } from "../types";
import { field, fileNameOf, keyFileField, passwordInput } from "../ui/controls";
import { h, mount, type Child } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog, showDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { relativeTime } from "../utils/format";
import { estimateStrength } from "../utils/strength";
import { credentialsDialog } from "./credentials";
import { describeChanges, hasChanges } from "./reports";

/** Opens the sync space: the sign-in form when signed out, the management view otherwise. */
export async function openSyncCenter(): Promise<void> {
  const container = h("div", { class: "stack sync-center" });

  const refresh = async () => {
    const sync = store.status?.sync;
    if (!sync?.loggedIn) {
      mount(container, signInSection(() => void refresh()));
      return;
    }
    const cloud = await backend.syncCloudVaults().catch(() => [] as CloudVault[]);
    mount(container, signedInSections(cloud, () => void refresh()));
  };

  await refresh();
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
  const password = passwordInput({ placeholder: "Master password", strength: true });
  const keyFile = keyFileField({});
  const error = h("p", { class: "form-error", hidden: true });
  const signIn = h("button", { type: "button", class: "button primary" }, "Sign in");
  const create = h("button", { type: "button", class: "button" }, "Create account");
  const setBusy = (busy: boolean) => {
    signIn.disabled = busy;
    create.disabled = busy;
  };
  const showError = (message: string) => {
    error.textContent = message;
    error.hidden = false;
  };

  const submit = async (mode: "in" | "up") => {
    error.hidden = true;
    if (!email.value.trim()) {
      email.focus();
      return;
    }
    if (!password.value()) {
      password.input.focus();
      return;
    }
    if (mode === "up") {
      // The account password is the one protecting cloud vaults, so it is not allowed to be weak.
      const strength = estimateStrength(password.value());
      if (strength.score < 4) {
        showError(`The account password must be very strong (this one is “${strength.label}”). Add length and variety.`);
        password.input.focus();
        return;
      }
    }
    setBusy(true);
    try {
      store.status =
        mode === "up"
          ? await backend.syncRegister(email.value.trim(), password.value(), keyFile.value())
          : await backend.syncSignIn(email.value.trim(), password.value(), keyFile.value());
      toast(mode === "up" ? "Account created" : "Signed in to Lados Sync", "success");
      refresh();
    } catch (failure) {
      toastError(failure);
      setBusy(false);
    }
  };
  signIn.addEventListener("click", () => void submit("in"));
  create.addEventListener("click", () => void submit("up"));

  return h(
    "div",
    { class: "stack" },
    h("p", {
      class: "muted",
      text: "Sign in to keep a vault in sync across your devices, and to download your cloud vaults on this one. Your vault is encrypted on this device before it is uploaded; only your master password opens it.",
    }),
    field("Email", email),
    field("Master password", password.element, { target: password.input }),
    h("p", { class: "field-hint", text: "New accounts need a very strong password: it is what protects your vaults in the cloud." }),
    field("Key file", keyFile.element, { hint: "Only if the vault was created with one." }),
    error,
    h("div", { class: "sync-center-actions" }, signIn, create),
  );
}

/** The account, cloud-vault and local-vault sections shown to a signed-in user. */
function signedInSections(cloud: CloudVault[], refresh: () => void): Child[] {
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

  /** Syncs a local copy: in place when it is the open vault, otherwise after asking its password. */
  const syncCopy = (path: string, title: string) =>
    run(async () => {
      if (openPath && path === openPath) {
        store.status = await backend.syncEnable();
      } else {
        const status = await credentialsDialog({
          title,
          message: `Enter the master password of “${fileNameOf(path)}”.`,
          submitLabel: "Sync",
          busyLabel: "Syncing…",
          action: ({ password, keyFile }) => backend.syncEnableFile(path, password, keyFile),
        });
        if (status === null) return;
        store.status = status;
      }
      toast(title === "Sync now" ? "Synced" : "This vault is now synced", "success");
    });

  const stopSyncing = (localVaultId: string) =>
    run(async () => {
      store.status = await backend.syncDisableVault(localVaultId);
      toast("Stopped syncing this vault");
    });

  const deleteCloudVault = (vault: CloudVault) =>
    run(async () => {
      const confirmed = await confirmDialog({
        title: `Delete “${vault.name}” from the cloud?`,
        message:
          "This permanently deletes the cloud copy and the data stored on the server. The vault files on this computer are kept, and they stop syncing. This cannot be undone.",
        confirmLabel: "Delete from cloud",
        danger: true,
      });
      if (!confirmed) return;
      store.status = await backend.syncDeleteVault(vault.id);
      toast(`“${vault.name}” was deleted from the cloud`);
    });

  const signOutOtherDevices = () =>
    run(async () => {
      await backend.syncSignOutEverywhere();
      toast("Signed out on your other devices");
    });

  const syncNowOpen = () =>
    run(async () => {
      const report = await performSync();
      toast(
        hasChanges(report) ? `Synced: ${describeChanges(report)}` : "Everything is up to date",
        hasChanges(report) ? "success" : "info",
      );
    });

  const cloudRows = cloud.map((vault) => {
    const device = sync.cloud.find((group) => group.localVaultId === vault.vaultUuid);
    const present = device?.present ?? false;
    const isOpen = !!openPath && (device?.copies.includes(openPath) ?? false);
    const when = device?.lastSyncedAt ? relativeTime(device.lastSyncedAt) : null;
    const status = present ? (when ? `Synced · ${when}` : "Not synced yet") : "Not on this device";

    const actions: Child[] = [];
    if (present) {
      actions.push(
        h(
          "button",
          {
            type: "button",
            class: "button small",
            onClick: () => void (isOpen ? syncNowOpen() : syncCopy(device!.copies[0], "Sync now")),
          },
          "Sync now",
        ),
      );
      actions.push(
        h("button", { type: "button", class: "button small", onClick: () => void stopSyncing(vault.vaultUuid ?? vault.id) }, "Stop syncing"),
      );
    } else {
      actions.push(
        h(
          "button",
          {
            type: "button",
            class: "button small primary",
            onClick: () =>
              void run(async () => {
                const status = await downloadCloudVault(vault);
                if (status) {
                  store.status = status;
                  toast(`“${vault.name}” was saved on this computer`, "success");
                }
              }),
          },
          "Download",
        ),
      );
    }
    actions.push(
      h("button", { type: "button", class: "button small danger", onClick: () => void deleteCloudVault(vault) }, "Delete"),
    );

    return h(
      "li",
      { class: "sync-vault-row" },
      svgIcon(present ? "cloudCheck" : "cloud", "icon sync-vault-icon"),
      h(
        "div",
        { class: "sync-vault-text" },
        h("span", { class: "sync-vault-name truncate", text: vault.name }),
        h("span", { class: ["sync-vault-status", present && "synced"], text: status }),
        present && device ? h("span", { class: "sync-vault-copies", text: `This device: ${device.copies.map(fileNameOf).join(", ")}` }) : null,
      ),
      h("div", { class: "sync-vault-actions" }, actions),
    );
  });

  const localRows = sync.local.map((vault: LocalVaultView) =>
    h(
      "li",
      { class: "sync-vault-row" },
      svgIcon("cloud", "icon sync-vault-icon"),
      h(
        "div",
        { class: "sync-vault-text" },
        h("span", { class: "sync-vault-name truncate", text: vault.fileName }),
        h("span", { class: "sync-vault-status", text: "Not synced · on this device only" }),
      ),
      h(
        "div",
        { class: "sync-vault-actions" },
        h(
          "button",
          { type: "button", class: "button small primary", onClick: () => void syncCopy(vault.path, "Sync this vault") },
          "Sync this vault",
        ),
      ),
    ),
  );

  return [
    h(
      "section",
      { class: "sync-center-section" },
      h("h3", { text: "Account" }),
      h("p", { class: "muted", text: `Signed in as ${sync.email}` }),
      h("p", { class: "muted", text: "To use a different account, sign out and sign in with the other email." }),
      h(
        "div",
        { class: "sync-center-actions" },
        h("button", { type: "button", class: "button", disabled: true, title: "Subscriptions are coming soon" }, "Manage subscription"),
        h("button", { type: "button", class: "button", onClick: () => void signOutOtherDevices() }, "Sign out other devices"),
        h("button", { type: "button", class: "button", onClick: () => void signOut(refresh) }, "Sign out"),
        h("button", { type: "button", class: "button danger", onClick: () => void removeAccount(refresh) }, "Delete account"),
      ),
    ),
    h(
      "section",
      { class: "sync-center-section" },
      h("h3", { text: "Cloud vaults" }),
      h("p", {
        class: "muted",
        text: "Every vault on your account, in the cloud. A vault can be on this computer or on another one.",
      }),
      cloudRows.length > 0 ? h("ul", { class: "sync-vault-list" }, cloudRows) : h("p", { class: "muted", text: "No vaults are synced yet." }),
      store.syncError
        ? h("div", { class: "notice warning" }, svgIcon("warning"), h("span", { text: store.syncError }))
        : null,
    ),
    h(
      "section",
      { class: "sync-center-section" },
      h("h3", { text: "On this device, not synced" }),
      localRows.length > 0
        ? h("ul", { class: "sync-vault-list" }, localRows)
        : h("p", { class: "muted", text: "Every vault on this device is synced." }),
      h("p", {
        class: "muted",
        text: "Syncing keeps the vault on this device and uploads an encrypted copy after every save; it is merged on your device, never on the server.",
      }),
    ),
  ];
}

/** Asks for the master password and a folder, then downloads the cloud vault into it. */
export async function downloadCloudVault(vault: CloudVault): Promise<AppStatus | null> {
  let folder = await backend.vaultsFolder().catch(() => "");
  const preview = h("code", { class: "muted", text: joinPath(folder, vault.name) });
  const password = passwordInput({ placeholder: "Master password" });
  const keyFile = keyFileField({});
  const change = h(
    "button",
    {
      type: "button",
      class: "button small",
      onClick: async () => {
        const picked = await backend.pickFolder(`Choose a folder for “${vault.name}”`);
        if (picked) {
          folder = picked;
          preview.textContent = joinPath(folder, vault.name);
        }
      },
    },
    "Change folder",
  );

  return showDialog<AppStatus>({
    title: `Download “${vault.name}”`,
    size: "medium",
    body: h(
      "div",
      { class: "stack" },
      h("p", { class: "muted", text: "The file keeps the vault's own name — you only choose where to store it." }),
      field("Save to", h("div", { class: "stack", style: "gap: 8px" }, preview, h("div", null, change))),
      field("Master password", password.element, { target: password.input }),
      field("Key file", keyFile.element, { hint: "Only if the vault was created with one." }),
    ),
    submitLabel: "Download",
    busyLabel: "Downloading…",
    onSubmit: async () => {
      if (!password.value()) {
        password.input.focus();
        throw new Error("Enter the master password");
      }
      return backend.syncDownload(vault.id, folder, password.value(), keyFile.value());
    },
  });
}

/** The path shown in the download preview; the backend decides the final name. */
function joinPath(folder: string, name: string): string {
  const separator = folder.includes("\\") ? "\\" : "/";
  return `${folder.replace(/[\\/]$/, "")}${separator}${name}.lados`;
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
