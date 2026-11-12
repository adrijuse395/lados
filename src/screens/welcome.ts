// Shown when no vault is selected: create one, open one or pick a recent one.

import { refreshStatus, showStatus } from "../app";
import { createVaultDialog } from "../dialogs/create-vault";
import { pickVaultToOpen } from "../dialogs/open-vault";
import { openSyncCenter } from "../dialogs/sync-center";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { RecentVault } from "../types";
import { fileNameOf, iconButton } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { logo, svgIcon } from "../ui/icons";
import { confirmDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";

async function selectVault(path: string): Promise<void> {
  try {
    await showStatus(await backend.selectVault(path));
  } catch (error) {
    toastError(error);
  }
}

async function openVault(mobile: boolean): Promise<void> {
  try {
    const path = await pickVaultToOpen();
    if (!path) {
      if (mobile) toast("No file selected");
      return;
    }
    const before = mobile ? await backend.listDeviceVaults().catch(() => []) : [];
    await selectVault(path);
    if (mobile && !before.some((vault) => vault.path === path)) {
      toast("Vault imported into Documents › Lados");
    }
  } catch (error) {
    toastError(error);
  }
}

async function createVault(): Promise<void> {
  try {
    const status = await createVaultDialog();
    if (status) await showStatus(status);
  } catch (error) {
    toastError(error);
  }
}

async function openAccount(): Promise<void> {
  await openSyncCenter();
  await refreshStatus();
}

async function removeListedVault(path: string, mobile: boolean): Promise<void> {
  try {
    if (mobile) {
      const name = fileNameOf(path);
      const confirmed = await confirmDialog({
        title: "Delete vault?",
        message: `This permanently deletes “${name}” from Documents › Lados on this phone. It cannot be undone.`,
        confirmLabel: "Delete vault",
        danger: true,
      });
      if (!confirmed) return;
    }
    await showStatus(await backend.removeListedVault(path));
    if (mobile) toast("Vault deleted");
  } catch (error) {
    toastError(error);
  }
}

function recentItem(vault: RecentVault, mobile: boolean): HTMLElement {
  const removeLabel = mobile ? "Delete vault from this phone" : "Remove from this list";
  return h(
    "li",
    { class: ["recent-item", !vault.exists && "missing"] },
    h(
      "button",
      {
        type: "button",
        class: "recent-open",
        disabled: !vault.exists,
        title: vault.path,
        onClick: () => void selectVault(vault.path),
      },
      svgIcon("lock", "icon recent-icon"),
      h(
        "span",
        { class: "recent-text" },
        h("span", { class: "recent-name", text: vault.fileName }),
        h("span", {
          class: "recent-path",
          text: vault.exists ? vault.path : `Not found: ${vault.path}`,
        }),
      ),
      store.status?.sync.loggedIn && vault.synced ? svgIcon("cloudCheck", "icon recent-sync-badge") : null,
    ),
    iconButton("close", removeLabel, () => void removeListedVault(vault.path, mobile), "recent-forget"),
  );
}

function mergeVaultLists(device: RecentVault[], recent: RecentVault[]): RecentVault[] {
  const seen = new Set<string>();
  const merged: RecentVault[] = [];
  for (const vault of [...device, ...recent]) {
    const key = vault.path;
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(vault);
  }
  return merged;
}

export function renderWelcome(container: HTMLElement): void {
  const status = store.status;
  const recent = status?.recentVaults ?? [];

  const root = h("main", { class: "center-screen welcome-screen" });
  mount(container, root);

  void (async () => {
    const mobile = await backend.isMobileShell().catch(() => false);
    let deviceVaults: RecentVault[] = [];
    if (mobile) {
      deviceVaults = await backend.listDeviceVaults().catch(() => []);
    }
    const listed = mobile ? mergeVaultLists(deviceVaults, recent) : recent;
    const openLabel = "Open a vault…";
    const tagline = mobile
      ? "Your passwords, encrypted and kept on this phone."
      : "Your passwords, encrypted and kept on your own computer.";
    const footnote = mobile
      ? `Lados ${status?.appVersion ?? ""} · Works offline on this device.`
      : `Lados ${status?.appVersion ?? ""} · Works offline: your data never leaves this computer.`;

    mount(
      root,
      h(
        "div",
        { class: "welcome" },
        h(
          "div",
          { class: "brand" },
          logo("logo large"),
          h("h1", { text: "Lados" }),
          h("p", { class: "tagline", text: tagline }),
        ),
        h(
          "div",
          { class: "welcome-actions" },
          h(
            "button",
            { type: "button", class: "button primary large", onClick: () => void createVault() },
            svgIcon("plus"),
            "Create a new vault",
          ),
          h(
            "button",
            { type: "button", class: "button large", onClick: () => void openVault(mobile) },
            svgIcon("folder"),
            openLabel,
          ),
        ),
        mobile
          ? null
          : status?.sync.loggedIn
            ? h(
                "p",
                { class: "welcome-sync muted" },
                svgIcon("cloudCheck"),
                h("span", { text: `Synced as ${status.sync.email}. ` }),
                h("button", { type: "button", class: "link-button", onClick: () => void openAccount() }, "Manage"),
              )
            : h(
                "p",
                { class: "welcome-sync muted" },
                svgIcon("cloud"),
                h("span", { text: "The same vault, on every device: " }),
                h("button", { type: "button", class: "link-button", onClick: () => void openAccount() }, "Lados Sync"),
              ),
        mobile
          ? h("p", {
              class: "welcome-hint muted",
              text: "Create and open use the same folder: Documents › Lados. It survives reinstalling the app. “Browse elsewhere” only imports a copy into that folder.",
            })
          : null,
        listed.length > 0
          ? h(
              "section",
              { class: "recent" },
              h("h2", { text: mobile ? "Vaults on this phone" : "Recent vaults" }),
              h(
                "ul",
                { class: "recent-list" },
                listed.map((vault) => recentItem(vault, mobile)),
              ),
            )
          : null,
        h("p", { class: "footnote", text: footnote }),
      ),
    );
  })().catch(toastError);
}
