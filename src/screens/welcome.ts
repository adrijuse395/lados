// Shown when no vault is selected: create one, open one, download a cloud one or pick a recent one.

import { refreshStatus, showStatus } from "../app";
import { createVaultDialog } from "../dialogs/create-vault";
import { pickVaultToOpen } from "../dialogs/open-vault";
import { downloadCloudVault, openSyncCenter } from "../dialogs/sync-center";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { CloudVault, RecentVault } from "../types";
import { fileNameOf, iconButton } from "../ui/controls";
import { h, mount, type Child } from "../ui/dom";
import { logo, svgIcon, type IconName } from "../ui/icons";
import { confirmDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { relativeTime } from "../utils/format";

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

async function getCloudVault(vault: CloudVault): Promise<void> {
  try {
    const status = await downloadCloudVault(vault);
    if (status) {
      store.status = status;
      await refreshStatus();
    }
  } catch (error) {
    toastError(error);
  }
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

/** A small "In the cloud" / "In this device" marker. */
function attribute(label: string, present: boolean): HTMLElement {
  return h(
    "span",
    { class: ["attribute", present ? "yes" : "no"] },
    svgIcon(present ? "check" : "close", "icon attribute-icon"),
    label,
  );
}

/** The clickable (or static) body of a vault row: icon, name and a second line of detail. */
function vaultRowBody(icon: IconName, title: string, detail: Child, titleAttr: string, onOpen?: () => void): HTMLElement {
  const text = h("span", { class: "recent-text" }, h("span", { class: "recent-name truncate", text: title }), detail);
  if (onOpen) {
    return h("button", { type: "button", class: "recent-open", title: titleAttr, onClick: onOpen }, svgIcon(icon, "icon recent-icon"), text);
  }
  return h("div", { class: "recent-open", title: titleAttr }, svgIcon(icon, "icon recent-icon"), text);
}

function recentItem(vault: RecentVault, mobile: boolean, showAttributes: boolean): HTMLElement {
  const removeLabel = mobile ? "Delete vault from this phone" : "Remove from this list";
  const detail = showAttributes
    ? h(
        "span",
        { class: "recent-attributes" },
        attribute("In the cloud", vault.synced),
        attribute("In this device", vault.exists),
      )
    : h("span", { class: "recent-path", text: vault.exists ? vault.path : `Not found: ${vault.path}` });
  return h(
    "li",
    { class: ["recent-item", !vault.exists && "missing"] },
    vaultRowBody("lock", vault.fileName, detail, vault.path, vault.exists ? () => void selectVault(vault.path) : undefined),
    iconButton("close", removeLabel, () => void removeListedVault(vault.path, mobile), "recent-forget"),
  );
}

function cloudItem(vault: CloudVault, present: boolean, openPath: string | null): HTMLElement {
  const detail = h(
    "span",
    { class: "recent-attributes" },
    attribute("In the cloud", true),
    attribute("In this device", present),
  );
  return h(
    "li",
    { class: ["recent-item", !present && "cloud-missing"] },
    vaultRowBody(
      present ? "cloudCheck" : "cloud",
      vault.name,
      detail,
      vault.name,
      present && openPath ? () => void selectVault(openPath) : undefined,
    ),
    present
      ? null
      : h("button", { type: "button", class: "button small primary", onClick: () => void getCloudVault(vault) }, "Download"),
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
    const signedIn = status?.sync.loggedIn ?? false;
    const openLabel = "Open a vault…";
    const tagline = mobile
      ? "Your passwords, encrypted and kept on this phone."
      : "Your passwords, encrypted and kept on your own computer.";
    const footnote = mobile
      ? `Lados ${status?.appVersion ?? ""} · Works offline on this device.`
      : `Lados ${status?.appVersion ?? ""} · Works offline: your data never leaves this computer.`;

    // Signed in: also show the vaults that live in the cloud, so it is clear what is
    // already on this device and what still has to be downloaded.
    let cloudVaults: CloudVault[] = [];
    if (signedIn && !mobile) {
      cloudVaults = await backend.syncCloudVaults().catch(() => []);
    }
    const deviceGroups = status?.sync.cloud ?? [];
    const syncedPaths = new Set(deviceGroups.flatMap((group) => group.copies));
    const localOnly = signedIn ? listed.filter((vault) => !syncedPaths.has(vault.path)) : listed;

    const rows: Child[] = [];
    for (const vault of cloudVaults) {
      const group = deviceGroups.find((candidate) => candidate.localVaultId === vault.vaultUuid);
      const present = group?.present ?? false;
      rows.push(cloudItem(vault, present, present ? (group?.copies[0] ?? null) : null));
    }
    for (const vault of localOnly) {
      rows.push(recentItem(vault, mobile, signedIn));
    }

    const listTitle = signedIn ? "Your vaults" : mobile ? "Vaults on this phone" : "Recent vaults";

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
          : signedIn
            ? h(
                "p",
                { class: "welcome-sync muted" },
                svgIcon("cloudCheck"),
                h("span", { text: `Signed in as ${status?.sync.email}. ` }),
                status?.sync.passwordChangedAt
                  ? h("span", { text: `Master password changed ${relativeTime(status.sync.passwordChangedAt)}. ` })
                  : null,
                h("button", { type: "button", class: "link-button", onClick: () => void openAccount() }, "Sync settings"),
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
        rows.length > 0 ? h("section", { class: "recent" }, h("h2", { text: listTitle }), h("ul", { class: "recent-list" }, rows)) : null,
        h("p", { class: "footnote", text: footnote }),
      ),
    );
  })().catch(toastError);
}
