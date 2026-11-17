// Decides which screen is shown and keeps it in sync with the Rust core.

import { renderUnlock } from "./screens/unlock";
import { drawVault, isVaultMounted, renderVault, updateSelection } from "./screens/vault";
import { renderWelcome } from "./screens/welcome";
import { clearAutofillCache, syncAutofillCache } from "./services/autofill-bridge";
import { BackendError, backend } from "./services/backend";
import { resetVaultState, setVault, store } from "./state/store";
import type { AppStatus, MergeReport, Theme } from "./types";
import { API_REVISION } from "./types";
import { closeMenu } from "./ui/menu";
import { closeAllModals } from "./ui/modal";
import { errorMessage, toast } from "./ui/toast";
import { maybeShowExtensionInstall } from "./dialogs/browser-extension";

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

// ----- Cloud sync (reconcile on open, push after a change) -----

let syncTimer: number | undefined;
let syncRunning = false;

/** Reconciles the open vault with the server when it is signed in and marked as synced. */
export function scheduleSync(): void {
  const sync = store.status?.sync;
  if (!sync?.loggedIn || !sync.thisVaultSynced || syncRunning) return;
  window.clearTimeout(syncTimer);
  syncTimer = window.setTimeout(() => {
    void performSync().catch(() => undefined);
  }, 900);
}

/** Runs one sync pass and refreshes the interface. Throws on failure. */
export async function performSync(): Promise<MergeReport> {
  if (syncRunning) throw new Error("A sync is already in progress");
  syncRunning = true;
  store.syncing = true;
  store.syncError = null;
  redraw();
  try {
    const report = await backend.syncNow();
    store.status = await backend.getStatus();
    await loadVault();
    return report;
  } catch (error) {
    store.syncError = errorMessage(error);
    if (error instanceof BackendError && error.kind === "sessionExpired") {
      // The session ended (for example the master password changed elsewhere): sign out locally
      // and tell the user, instead of failing silently on every later sync.
      try {
        store.status = await backend.syncRefresh();
      } catch {
        // Ignore: the message below is what matters.
      }
      toast(errorMessage(error), "error", 8000);
    }
    throw error;
  } finally {
    syncRunning = false;
    store.syncing = false;
    redraw();
  }
}

/** Warns once when the running process is an older build than this interface, which otherwise
 *  shows up as a baffling error on the first call to a command or argument that did not exist yet. */
let warnedAboutStaleBuild = false;

function warnIfStaleBuild(status: AppStatus): void {
  if (warnedAboutStaleBuild || status.apiRevision >= API_REVISION) return;
  warnedAboutStaleBuild = true;
  toast(
    "This Lados process is older than its interface, so new features will fail. Close the app and start it again.",
    "error",
    15000,
  );
}

/** Shows the screen that matches the session state reported by the core. */
export async function showStatus(status: AppStatus): Promise<void> {
  const previous = store.status;
  store.status = status;
  warnIfStaleBuild(status);
  applyTheme(status.preferences.theme);

  if (status.state === "unlocked") {
    await loadVault();
    if (previous?.state === "unlocked" && isVaultMounted()) {
      drawVault();
    } else {
      renderVault(root());
    }
    void syncAutofillCache();
    void maybeShowExtensionInstall();
    scheduleSync();
    return;
  }

  clearAutofillCache();
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
  void syncAutofillCache();
  scheduleSync();
}

/** Redraws the vault screen from the data already loaded. */
export function redraw(): void {
  if (isVaultMounted()) drawVault();
}

/** Reflects a new selected entry without redrawing everything. */
export function refreshSelection(): void {
  if (isVaultMounted()) updateSelection();
}
