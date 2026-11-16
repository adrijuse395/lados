// The dedicated "Lados Sync" space: the account, the cloud vaults and the local vaults.
// It is deliberately separate from Settings, because it is not a preference.

import { listen } from "@tauri-apps/api/event";
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

/** A vault whose master password isn't the account's cannot be synced until it is re-keyed. */
function isDifferentMaster(error: unknown): boolean {
  return error instanceof Error && error.message.includes("different master password");
}

/** Asks for a vault's or the master credentials; returns them, or null if cancelled. */
function askPassword(title: string, message: string, initialKeyFile: string | null) {
  return credentialsDialog<{ password: string; keyFile: string | null }>({
    title,
    message,
    submitLabel: "Continue",
    busyLabel: "Checking…",
    initialKeyFile,
    action: async (credentials) => credentials,
  });
}

/** Progress pushed while the master password is being changed across every synced vault. */
interface PasswordChangeProgress {
  step: "vaults" | "account" | "sessions" | "done";
  index: number;
  total: number;
  label: string;
}

/** The blocking, guided change of the one master password: the account and every synced vault. */
async function changeMasterPassword(refresh: () => void): Promise<void> {
  const current = passwordInput({ autofocus: true, placeholder: "Current master password" });
  const currentKeyFile = keyFileField({});
  const next = passwordInput({ strength: true, placeholder: "Choose a strong password" });
  const confirmation = passwordInput({ placeholder: "Type it again" });
  const nextKeyFile = keyFileField({ allowCreate: true });
  const progress = h("div", { class: "stack", hidden: true }, h("p", { class: "muted", text: "" }));
  const progressText = progress.firstElementChild as HTMLParagraphElement;
  let unlisten: (() => void) | null = null;

  const result = await showDialog<AppStatus>({
    title: "Change master password",
    size: "medium",
    body: h(
      "div",
      { class: "stack" },
      h(
        "div",
        { class: "notice warning" },
        svgIcon("warning"),
        h("span", {
          text:
            "Every synced vault is encrypted again with the new password, and your account uses it too. " +
            "Your other devices must sign in with the new password afterwards. Do not close Lados while this runs.",
        }),
      ),
      field("Current master password", current.element, { target: current.input }),
      field("Current key file", currentKeyFile.element, { hint: "Only if you use one." }),
      field("New master password", next.element, { target: next.input }),
      field("Confirm the new password", confirmation.element, { target: confirmation.input }),
      field("New key file", nextKeyFile.element, {
        hint: "Optional. If you use one, the vaults need it together with the new password.",
      }),
      progress,
    ),
    submitLabel: "Change master password",
    busyLabel: "Changing…",
    onSubmit: async () => {
      if (!current.value()) {
        current.input.focus();
        throw new Error("Enter your current master password");
      }
      if (!next.value()) {
        next.input.focus();
        throw new Error("Enter the new master password");
      }
      if (next.value() !== confirmation.value()) {
        confirmation.input.focus();
        throw new Error("The new passwords do not match");
      }
      if (estimateStrength(next.value()).score < 4) {
        next.input.focus();
        throw new Error("Choose a very strong master password — it protects every synced vault.");
      }
      progress.hidden = false;
      progressText.textContent = "Starting…";
      unlisten = await listen<PasswordChangeProgress>("master-password-progress", (event) => {
        const { step, index, total, label } = event.payload;
        progressText.textContent =
          step === "vaults"
            ? `Re-encrypting ${label || "a vault"} (${index + 1}/${total})…`
            : step === "account"
              ? "Updating your account…"
              : step === "sessions"
                ? "Signing out your other devices…"
                : "Finishing…";
      });
      try {
        return await backend.changeMasterPassword(
          current.value(),
          currentKeyFile.value(),
          next.value(),
          nextKeyFile.value(),
        );
      } finally {
        unlisten?.();
        unlisten = null;
      }
    },
  });

  if (!result) return;
  store.status = result;
  toast("Master password changed", "success");
  refresh();
}

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
      text: "Sign in to sync your vaults across devices, or to download them on this one. Your vault is encrypted on this device before it is uploaded; only your master password opens it.",
    }),
    field("Email", email),
    field("Master password", password.element, { target: password.input }),
    h("p", {
      class: "field-hint",
      text: "This master password is your account password and the password of every vault you sync. It must be very strong.",
    }),
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

  // Older versions uploaded labels in clear; offer to encrypt them without changing anything else.
  const needsNames = cloud.some((vault) => !vault.nameCipher && vault.name);

  /** Syncs a local copy: in place when it is the open vault, otherwise after asking its password.
   *  If the vault's master password isn't the account's, it is re-encrypted first. */
  const syncCopy = (path: string, title: string) =>
    run(async () => {
      const rekeyed = "This vault was re-encrypted with your master password and is now synced";
      if (openPath && path === openPath) {
        try {
          store.status = await backend.syncEnable();
          toast(title === "Sync now" ? "Synced" : "This vault is now synced", "success");
          return;
        } catch (error) {
          if (!isDifferentMaster(error)) throw error;
        }
        const master = await askPassword(
          "Use your sync master password",
          `Synced vaults share your sync master password. Enter it to re-encrypt “${fileNameOf(path)}”.`,
          store.status?.keyFile ?? null,
        );
        if (!master) return;
        await backend.rekeyVaultFile(path, null, null, master.password, master.keyFile);
        store.status = await backend.syncEnable();
        toast(rekeyed, "success");
        return;
      }
      const vault = await askPassword("Sync this vault", `Enter the master password of “${fileNameOf(path)}”.`, null);
      if (!vault) return;
      try {
        store.status = await backend.syncEnableFile(path, vault.password, vault.keyFile);
        toast(title === "Sync now" ? "Synced" : "This vault is now synced", "success");
        return;
      } catch (error) {
        if (!isDifferentMaster(error)) throw error;
      }
      const master = await askPassword(
        "Use your sync master password",
        `Synced vaults share your sync master password. Enter it to re-encrypt “${fileNameOf(path)}”.`,
        null,
      );
      if (!master) return;
      await backend.rekeyVaultFile(path, vault.password, vault.keyFile, master.password, master.keyFile);
      store.status = await backend.syncEnableFile(path, master.password, master.keyFile);
      toast(rekeyed, "success");
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
        h("button", { type: "button", class: "button", onClick: () => void changeMasterPassword(refresh) }, "Change master password"),
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
      needsNames
        ? h(
            "div",
            { class: "notice warning" },
            svgIcon("warning"),
            h("span", { text: "Some vault names are still stored without encryption." }),
            h(
              "button",
              {
                type: "button",
                class: "button small",
                onClick: () =>
                  void run(async () => {
                    store.status = await backend.resyncLabels();
                    toast("Vault names are now encrypted", "success");
                  }),
              },
              "Encrypt names",
            ),
          )
        : null,
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
