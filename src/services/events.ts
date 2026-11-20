// Notifications pushed by the Rust core (automatic lock, changes made in another window, ...).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AutofillRevealRequest, LockReason, MergeReport } from "../types";

export interface BackendEventHandlers {
  vaultLocked(reason: LockReason): void;
  /** Changes saved by another window or program were merged into the open vault. */
  vaultChanged(report: MergeReport): void;
  /** The file on disk could not be read any more; work continues in this conflict copy. */
  vaultDiverged(copyPath: string): void;
  /** The vault file was deleted or moved while open. Nothing has been saved (report M8). */
  vaultFileMissing(path: string): void;
  /** A save wrote the vault file again at this path, after it had been removed. */
  vaultRecreated(path: string): void;
  syncError(message: string): void;
  clipboardCleared(): void;
  /** The browser extension wants to fill a login and needs the user's answer. */
  autofillReveal(request: AutofillRevealRequest): void;
  /** The window was hidden to the system tray, where the app keeps running. */
  appHiddenToTray(): void;
}

export async function listenToBackend(handlers: BackendEventHandlers): Promise<UnlistenFn> {
  const unlisteners = await Promise.all([
    listen<LockReason>("vault-locked", (event) => handlers.vaultLocked(event.payload)),
    listen<MergeReport>("vault-changed", (event) => handlers.vaultChanged(event.payload)),
    listen<string>("vault-diverged", (event) => handlers.vaultDiverged(event.payload)),
    listen<string>("vault-file-missing", (event) => handlers.vaultFileMissing(event.payload)),
    listen<string>("vault-recreated", (event) => handlers.vaultRecreated(event.payload)),
    listen<string>("vault-sync-error", (event) => handlers.syncError(event.payload)),
    listen("clipboard-cleared", () => handlers.clipboardCleared()),
    listen<AutofillRevealRequest>("autofill-reveal", (event) => handlers.autofillReveal(event.payload)),
    listen("app-hidden-to-tray", () => handlers.appHiddenToTray()),
  ]);
  return () => unlisteners.forEach((unlisten) => unlisten());
}
