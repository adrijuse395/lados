// Notifications pushed by the Rust core (automatic lock, changes made in another window, ...).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { LockReason, MergeReport } from "../types";

export interface BackendEventHandlers {
  vaultLocked(reason: LockReason): void;
  /** Changes saved by another window or program were merged into the open vault. */
  vaultChanged(report: MergeReport): void;
  /** The file on disk could not be read any more; work continues in this conflict copy. */
  vaultDiverged(copyPath: string): void;
  syncError(message: string): void;
  clipboardCleared(): void;
}

export async function listenToBackend(handlers: BackendEventHandlers): Promise<UnlistenFn> {
  const unlisteners = await Promise.all([
    listen<LockReason>("vault-locked", (event) => handlers.vaultLocked(event.payload)),
    listen<MergeReport>("vault-changed", (event) => handlers.vaultChanged(event.payload)),
    listen<string>("vault-diverged", (event) => handlers.vaultDiverged(event.payload)),
    listen<string>("vault-sync-error", (event) => handlers.syncError(event.payload)),
    listen("clipboard-cleared", () => handlers.clipboardCleared()),
  ]);
  return () => unlisteners.forEach((unlisten) => unlisten());
}
