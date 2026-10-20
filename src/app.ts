// Decides which screen is shown and keeps it in sync with the Rust core.

import { renderUnlock } from "./screens/unlock";
import { drawVault, isVaultMounted, renderVault, updateSelection } from "./screens/vault";
import { renderWelcome } from "./screens/welcome";
import { BackendError, backend } from "./services/backend";
import { resetVaultState, setVault, store } from "./state/store";
import type { AppStatus, Theme } from "./types";
import { closeMenu } from "./ui/menu";
import { closeAllModals } from "./ui/modal";

function root(): HTMLElement {
  const element = document.getElementById("app");
  if (!element) throw new Error("The page has no #app element");
  return element;
}

// ----- Theme -----

const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");
let themePreference: Theme = "system";

export function applyTheme(theme: Theme): void {
  themePreference = theme;
  const dark = theme === "dark" || (theme === "system" && darkScheme.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

darkScheme.addEventListener("change", () => applyTheme(themePreference));

// ----- Navigation -----

async function loadVault(): Promise<void> {
  setVault(await backend.getVaultView());
  store.searchResults = store.query.trim() ? new Set(await backend.searchEntries(store.query)) : null;
}

/** Shows the screen that matches the session state reported by the core. */
export async function showStatus(status: AppStatus): Promise<void> {
  const previous = store.status;
  store.status = status;
  applyTheme(status.preferences.theme);

  if (status.state === "unlocked") {
    await loadVault();
    if (previous?.state === "unlocked" && isVaultMounted()) {
      drawVault();
    } else {
      renderVault(root());
    }
    return;
  }

  closeMenu();
  closeAllModals();
  resetVaultState();
  const sameLockScreen =
    previous?.state === "locked" && status.state === "locked" && previous.vaultPath === status.vaultPath && root().querySelector(".unlock-screen");
  if (sameLockScreen) return;
  if (status.state === "locked") {
    renderUnlock(root());
  } else {
    renderWelcome(root());
  }
}

export async function refreshStatus(): Promise<void> {
  await showStatus(await backend.getStatus());
}

/** Fetches the vault summary again after a change and redraws the vault screen. */
export async function reloadVault(): Promise<void> {
  try {
    await loadVault();
  } catch (error) {
    if (error instanceof BackendError && (error.kind === "locked" || error.kind === "noVaultSelected")) {
      await refreshStatus();
      return;
    }
    throw error;
  }
  redraw();
}

/** Redraws the vault screen from the data already loaded. */
export function redraw(): void {
  if (isVaultMounted()) drawVault();
}

/** Reflects a new selected entry without redrawing everything. */
export function refreshSelection(): void {
  if (isVaultMounted()) updateSelection();
}
