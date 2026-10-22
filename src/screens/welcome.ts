// Shown when no vault is selected: create one, open one or pick a recent one.

import { showStatus } from "../app";
import { createVaultDialog } from "../dialogs/create-vault";
import { pickVaultToOpen } from "../dialogs/open-vault";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { RecentVault } from "../types";
import { iconButton } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { logo, svgIcon } from "../ui/icons";
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

async function forget(path: string): Promise<void> {
  try {
    await showStatus(await backend.forgetRecentVault(path));
  } catch (error) {
    toastError(error);
  }
}

function recentItem(vault: RecentVault, showForget: boolean): HTMLElement {
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
    ),
    showForget ? iconButton("close", "Remove from this list", () => void forget(vault.path), "recent-forget") : null,
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
                listed.map((vault) => recentItem(vault, !mobile || recent.some((item) => item.path === vault.path))),
              ),
            )
          : null,
        h("p", { class: "footnote", text: footnote }),
      ),
    );
  })().catch(toastError);
}
