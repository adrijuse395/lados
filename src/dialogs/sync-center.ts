// The dedicated "Lados Sync" space: the account, the cloud vaults and the local vaults.
// It is deliberately separate from Settings, because it is not a preference.

import { listen } from "@tauri-apps/api/event";
import { performSync } from "../app";
import { backend, isCredentialError } from "../services/backend";
import { store } from "../state/store";
import type { AppStatus, CloudVault, LocalVaultView } from "../types";
import { field, fileNameOf, keyFileField, passwordInput } from "../ui/controls";
import { h, mount, type Child } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog, showDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { formatDay, relativeTime } from "../utils/format";
import { estimateStrength } from "../utils/strength";
import { confirmCredentialsDialog, credentialsDialog } from "./credentials";
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

/** The blocking, guided change of the one master password: the account and every synced vault.
 *  Exported so the vault menu can offer it too; the caller re-renders as it needs. */
export async function openChangeMasterPassword(): Promise<AppStatus | null> {
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

  if (!result) return null;
  store.status = result;
  toast("Master password changed", "success");
  return result;
}

/** Opens the sync space: the sign-in form when signed out, the management view otherwise. */
export async function openSyncCenter(): Promise<void> {
  const container = h("div", { class: "stack sync-center" });

  const refresh = async () => {
    // Ask the server about the account first: this is where a session that ended on another
    // device (for example after a master-password change) shows up, so we can ask to sign in.
    if (store.status?.sync.loggedIn) {
      try {
        store.status = await backend.syncRefresh();
      } catch {
        // Offline: keep what we already know about the account.
      }
    }
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
    size: "large",
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
      if (isCredentialError(failure)) {
        showError(
          "Incorrect email or password. If you created the account with a key file, add it below; if you changed your master password on another device, sign in with the new one.",
        );
      } else {
        toastError(failure);
      }
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
    store.status?.sync.serverUrl
      ? h("p", { class: "muted", text: `Server: ${store.status.sync.serverUrl}` })
      : null,
    field("Email", email),
    field("Master password", password.element, { target: password.input }),
    h("p", {
      class: "field-hint",
      text: "This master password is your account password and the password of every vault you sync. It must be very strong.",
    }),
    field("Key file", keyFile.element, { hint: "Only if your account was created with one — leave it empty otherwise." }),
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
          // The account's key file, not the open vault's: pre-filling the vault's made it easy to
          // send the wrong credentials to the account (see the red-team report, M2).
          null,
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
      try {
        const report = await performSync();
        toast(
          hasChanges(report) ? `Synced: ${describeChanges(report)}` : "Everything is up to date",
          hasChanges(report) ? "success" : "info",
        );
      } catch (error) {
        // The vault was re-keyed on its own and no longer matches the account: offer to fix it.
        if (isDifferentMaster(error) && openPath) {
          await syncCopy(openPath, "Sync now");
          return;
        }
        throw error;
      }
    });

  // The sync map: one row per cloud vault, linking the cloud node to this device.
  const mapRows = cloud.map((vault) => {
    const device = sync.cloud.find((group) => group.localVaultId === vault.vaultUuid);
    const present = device?.present ?? false;
    const isOpen = !!openPath && (device?.copies.includes(openPath) ?? false);
    const when = device?.lastSyncedAt ? relativeTime(device.lastSyncedAt) : null;

    const cloudNode = h(
      "div",
      { class: "sync-node sync-node-cloud" },
      h("span", { class: ["sync-node-icon", present && "on"] }, svgIcon(present ? "cloudCheck" : "cloud")),
      h(
        "div",
        { class: "sync-node-text" },
        h("span", { class: "sync-node-name truncate", title: vault.name, text: vault.name }),
        h("span", {
          class: "sync-node-detail",
          text: present ? (when ? `Synced · ${when}` : "Synced") : "In the cloud only",
        }),
      ),
      h(
        "button",
        {
          type: "button",
          class: "icon-button small",
          title: `Delete “${vault.name}” from the cloud`,
          "aria-label": "Delete from the cloud",
          onClick: () => void deleteCloudVault(vault),
        },
        svgIcon("trash"),
      ),
    );

    const deviceActions: Child[] = [];
    if (present) {
      deviceActions.push(
        h(
          "button",
          {
            type: "button",
            class: "button small",
            onClick: () => void (isOpen ? syncNowOpen() : syncCopy(device!.copies[0], "Sync now")),
          },
          "Sync now",
        ),
        h("button", { type: "button", class: "button small", onClick: () => void stopSyncing(vault.vaultUuid ?? vault.id) }, "Stop syncing"),
      );
    } else {
      deviceActions.push(
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

    const deviceNode = h(
      "div",
      { class: ["sync-node", "sync-node-device", !present && "empty"] },
      present && device
        ? h(
            "div",
            { class: "sync-node-text" },
            ...device.copies.map((copy) =>
              h(
                "span",
                { class: "sync-device-copy", title: copy },
                svgIcon("monitor", "icon sync-device-copy-icon"),
                h("span", { class: "truncate", text: fileNameOf(copy) }),
              ),
            ),
          )
        : h("div", { class: "sync-node-text" }, h("span", { class: "sync-node-detail", text: "Not on this device" })),
      h("div", { class: "sync-node-actions" }, deviceActions),
    );

    const state = present ? (when ? "synced" : "pending") : "missing";
    const link = h(
      "div",
      { class: "sync-link", dataset: { state }, role: "presentation" },
      h("span", { class: "sync-link-track" }),
      h("span", { class: "sync-link-dot" }),
    );

    return h("div", { class: "sync-map-row" }, cloudNode, link, deviceNode);
  });

  // ----- Entitlement (the server is authoritative; this only renders it) -----
  const plan = sync.plan ?? "active";
  const planUntil = sync.planValidUntil;
  const stillPaid = planUntil != null && planUntil > Date.now();
  // A subscription scheduled to cancel keeps working until its date: offer to resume it.
  const scheduledCancel = plan === "canceled" && stillPaid;
  // A failed payment within its grace window is not something to buy your way out of.
  const paymentFailed = plan === "past_due" && stillPaid;
  // Only states with no live subscription may start a new one (the server enforces this too).
  const canSubscribe =
    plan === "trialing" || plan === "expired" || ((plan === "canceled" || plan === "past_due") && !stillPaid);

  // Paddle's webhook flips the plan within seconds of paying. Poll briefly so the badge and the
  // buttons update on their own, instead of waiting for a manual refresh.
  const watchForPayment = () => {
    const before = plan;
    void (async () => {
      for (let attempt = 0; attempt < 20; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 3000));
        try {
          const next = (await backend.syncRefresh()).sync?.plan ?? before;
          if (next !== before) {
            await refresh();
            toast(next === "active" ? "Sync is active — you're all set." : `Plan updated: ${next}.`, "success");
            return;
          }
        } catch {
          // Offline or transient: keep waiting.
        }
      }
    })();
  };

  const subscribe = () =>
    run(async () => {
      const url = await backend.billingCheckout();
      await backend.openExternalUrl(url);
      toast("Finish in your browser — syncing turns on as soon as the payment goes through.", "info", 8000);
      watchForPayment();
    });

  const cancelSubscription = () =>
    run(async () => {
      const confirmed = await confirmDialog({
        title: "Cancel your subscription?",
        message: "You keep syncing until the end of the period you have already paid for, and your vaults stay on the server.",
        confirmLabel: "Cancel subscription",
        danger: true,
      });
      if (!confirmed) return;
      await backend.billingCancel();
      toast("Subscription canceled — syncing continues until the period ends.");
      refresh();
      setTimeout(refresh, 2500); // show the scheduled cancellation once the webhook lands
    });

  const resumeSubscription = () =>
    run(async () => {
      await backend.billingResume();
      toast("Subscription resumed — it will renew as usual.");
      refresh();
      setTimeout(refresh, 2500);
    });

  const notice = (() => {
    if (paymentFailed) {
      return "Your last payment didn't go through. Use the link Paddle emailed you to update your card — syncing keeps working until the period ends.";
    }
    if (plan === "trialing" && !stillPaid) {
      return "Your free trial has ended. Subscribe to keep syncing — you can still download your vaults from the cloud.";
    }
    if (plan === "expired" || (plan === "past_due" && !stillPaid)) {
      return "Syncing is paused. You can still download your vaults from the cloud — subscribe to resume uploads.";
    }
    return null;
  })();

  const planBadge = (() => {
    switch (plan) {
      case "comp":
        return { text: "Complimentary", tone: "success" };
      case "active":
        return { text: "Active", tone: "success" };
      case "trialing": {
        if (!stillPaid) return { text: "Trial ended", tone: "warning" };
        const days = planUntil ? Math.max(0, Math.ceil((planUntil - Date.now()) / 86_400_000)) : null;
        return { text: days != null ? `Trial · ${days} day${days === 1 ? "" : "s"} left` : "Trial", tone: "neutral" };
      }
      case "past_due":
        return { text: "Payment failed", tone: "warning" };
      case "canceled":
        return { text: planUntil ? `Ends ${formatDay(planUntil)}` : "Cancels soon", tone: "warning" };
      case "expired":
        return { text: "Expired", tone: "warning" };
      default:
        return { text: plan, tone: "neutral" };
    }
  })();

  const accountCard = h(
    "section",
    { class: "sync-card sync-account" },
    h(
      "div",
      { class: "sync-account-head" },
      h("span", { class: "sync-account-avatar" }, h("img", { class: "sync-account-logo", src: "/lados-sync-logo.png", alt: "" })),
      h(
        "div",
        { class: "sync-account-id" },
        h("span", { class: "sync-account-email truncate", title: sync.email ?? "", text: sync.email ?? "" }),
        h(
          "span",
          { class: "sync-account-meta" },
          h("span", { class: ["chip", `chip-${planBadge.tone}`] }, planBadge.text),
          plan === "active" && planUntil
            ? h("span", { class: "muted", text: `Renews ${formatDay(planUntil)}` })
            : null,
          sync.passwordChangedAt
            ? h("span", { class: "muted", text: `Master password changed ${relativeTime(sync.passwordChangedAt)}` })
            : null,
        ),
      ),
    ),
    h(
      "div",
      { class: "sync-actions" },
      canSubscribe
        ? h(
            "button",
            { type: "button", class: "button small primary", onClick: () => void subscribe() },
            plan === "trialing" ? "Subscribe" : "Renew",
          )
        : scheduledCancel
          ? h(
              "button",
              { type: "button", class: "button small primary", onClick: () => void resumeSubscription() },
              "Resume subscription",
            )
          : plan === "active"
            ? h("button", { type: "button", class: "button small", onClick: () => void cancelSubscription() }, "Cancel subscription")
            : null,
      h(
        "button",
        {
          type: "button",
          class: "button small",
          onClick: () => void openChangeMasterPassword().then((status) => { if (status) refresh(); }),
        },
        "Change master password",
      ),
      h("button", { type: "button", class: "button small", onClick: () => void signOutOtherDevices() }, "Sign out other devices"),
      h("button", { type: "button", class: "button small", onClick: () => void signOut(refresh) }, "Sign out"),
      h("button", { type: "button", class: "button small danger", onClick: () => void removeAccount(refresh) }, "Delete account"),
    ),
    notice
      ? h("div", { class: "notice warning" }, svgIcon("warning"), h("span", { text: notice }))
      : null,
  );

  const mapCard = h(
    "section",
    { class: "sync-card sync-map-card" },
    h("h3", { class: "sync-card-title" }, "Where your vaults are"),
    h(
      "div",
      { class: "sync-map-head" },
      h("span", { class: "sync-map-lane" }, svgIcon("cloud"), "Cloud"),
      h("span", { class: "sync-map-lane sync-map-lane-center" }),
      h("span", { class: "sync-map-lane" }, svgIcon("monitor"), "This device"),
    ),
    mapRows.length > 0
      ? h("div", { class: "sync-map-body" }, mapRows)
      : h("p", { class: "sync-empty", text: "No vaults are synced yet. Use “Sync this vault” below." }),
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
  );

  const deviceCard = h(
    "section",
    { class: "sync-card" },
    h("h3", { class: "sync-card-title" }, "On this device, not synced"),
    sync.local.length > 0
      ? h(
          "ul",
          { class: "sync-device-list" },
          sync.local.map((vault: LocalVaultView) =>
            h(
              "li",
              { class: "sync-device-row" },
              h("span", { class: "sync-node-icon" }, svgIcon("lock")),
              h(
                "div",
                { class: "sync-node-text" },
                h("span", { class: "sync-node-name truncate", title: vault.path, text: vault.fileName }),
                h("span", { class: "sync-node-detail", text: "Kept on this computer only" }),
              ),
              h(
                "button",
                { type: "button", class: "button small primary", onClick: () => void syncCopy(vault.path, "Sync this vault") },
                "Sync this vault",
              ),
            ),
          ),
        )
      : h("p", { class: "sync-empty", text: "Every vault on this device is synced." }),
    h("p", {
      class: "sync-note",
      text: "Syncing keeps the vault here and uploads an encrypted copy after every save; it is merged on your device, never on the server.",
    }),
  );

  return [accountCard, mapCard, deviceCard];
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
      field("Key file", keyFile.element, { hint: "Only if your account was created with one — leave it empty otherwise." }),
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
    // A stale "Sync failed" chip has nothing left to describe.
    store.syncError = null;
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
      "This permanently deletes your account and the encrypted copies stored on the server, and cancels any subscription so you are no longer charged. The vaults on this device are kept. This cannot be undone.",
    confirmLabel: "Delete account",
    danger: true,
  });
  if (!confirmed) return;
  const current = await confirmCredentialsDialog(
    "Confirm it's you",
    "This deletes your account and every encrypted copy on the server, and cancels the subscription. Enter your sync master password. If you no longer know it: cancel the subscription in Paddle, or write to ladospwm@proton.me and we will remove the account.",
  );
  if (!current) return;
  try {
    store.status = await backend.syncDeleteAccount(current.password, current.keyFile);
    // Nothing is synced any more, so the old failure note must not outlive the account.
    store.syncError = null;
    toast("Account deleted");
    refresh();
  } catch (error) {
    toastError(error);
  }
}
