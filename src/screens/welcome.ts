// Shown when no vault is selected: create one, open one or pick a recent one.

import { showStatus } from "../app";
import { createVaultDialog } from "../dialogs/create-vault";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { RecentVault } from "../types";
import { iconButton } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { logo, svgIcon } from "../ui/icons";
import { toastError } from "../ui/toast";

async function selectVault(path: string): Promise<void> {
  try {
    await showStatus(await backend.selectVault(path));
  } catch (error) {
    toastError(error);
  }
}

async function openVault(): Promise<void> {
  try {
    const path = await backend.pickOpenPath("vault", "Open a vault");
    if (path) await selectVault(path);
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

function recentItem(vault: RecentVault): HTMLElement {
  return h(
    "li",
    { class: ["recent-item", !vault.exists && "missing"] },
    h(
      "button",
      { type: "button", class: "recent-open", disabled: !vault.exists, title: vault.path, onClick: () => void selectVault(vault.path) },
      svgIcon("lock", "icon recent-icon"),
      h(
        "span",
        { class: "recent-text" },
        h("span", { class: "recent-name", text: vault.fileName }),
        h("span", { class: "recent-path", text: vault.exists ? vault.path : `Not found: ${vault.path}` }),
      ),
    ),
    iconButton("close", "Remove from this list", () => void forget(vault.path), "recent-forget"),
  );
}

export function renderWelcome(container: HTMLElement): void {
  const status = store.status;
  const recent = status?.recentVaults ?? [];
  mount(
    container,
    h(
      "main",
      { class: "center-screen welcome-screen" },
      h(
        "div",
        { class: "welcome" },
        h(
          "div",
          { class: "brand" },
          logo("logo large"),
          h("h1", { text: "Lados" }),
          h("p", { class: "tagline", text: "Your passwords, encrypted and kept on your own computer." }),
        ),
        h(
          "div",
          { class: "welcome-actions" },
          h("button", { type: "button", class: "button primary large", onClick: () => void createVault() }, svgIcon("plus"), "Create a new vault"),
          h("button", { type: "button", class: "button large", onClick: () => void openVault() }, svgIcon("folder"), "Open a vault…"),
        ),
        recent.length > 0
          ? h("section", { class: "recent" }, h("h2", { text: "Recent vaults" }), h("ul", { class: "recent-list" }, recent.map(recentItem)))
          : null,
        h("p", { class: "footnote", text: `Lados ${status?.appVersion ?? ""} · Works offline: your data never leaves this computer.` }),
      ),
    ),
  );
}
