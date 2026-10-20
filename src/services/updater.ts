import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { toast } from "../ui/toast";

export type UpdateProgress = {
  downloaded: number;
  total: number | null;
};

/** Returns an available update, or null when up to date / offline / check failed quietly. */
export async function checkForAppUpdate(): Promise<Update | null> {
  try {
    return await check();
  } catch {
    // Offline-first: a failed check must never block the app.
    return null;
  }
}

async function installUpdate(
  update: Update,
  onProgress: (progress: UpdateProgress) => void,
): Promise<void> {
  let downloaded = 0;
  let total: number | null = null;
  await update.downloadAndInstall((event: DownloadEvent) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        downloaded = 0;
        onProgress({ downloaded, total });
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress({ downloaded, total });
        break;
      case "Finished":
        onProgress({ downloaded, total });
        break;
    }
  });
  await relaunch();
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Offers to download and install an update the user already knows about. */
export async function promptAppUpdate(update: Update): Promise<void> {
  const notes = (update.body ?? "").trim();
  const status = h("p", { class: "muted", text: "The vault stays on this computer. Only the app installer is downloaded from GitHub." });
  const progress = h("p", { class: "muted", text: "" });
  progress.hidden = true;

  await showDialog({
    title: `Update to ${update.version}`,
    size: "medium",
    body: h(
      "div",
      { class: "stack" },
      h("p", { text: `A newer version of Lados is available (${update.version}).` }),
      notes ? h("pre", { class: "update-notes", text: notes }) : null,
      status,
      progress,
    ),
    cancelLabel: "Later",
    submitLabel: "Download and install",
    busyLabel: "Updating…",
    onSubmit: async () => {
      status.textContent = "Downloading the update…";
      progress.hidden = false;
      await installUpdate(update, ({ downloaded, total }) => {
        progress.textContent =
          total && total > 0
            ? `${formatBytes(downloaded)} of ${formatBytes(total)}`
            : `${formatBytes(downloaded)} downloaded`;
      });
      return true;
    },
  });
}

/** Silent startup check: only opens a dialog when an update exists. */
export async function maybeOfferAppUpdate(): Promise<void> {
  const update = await checkForAppUpdate();
  if (!update) return;
  try {
    await promptAppUpdate(update);
  } catch {
    // Dialog cancelled or install failed after toast — ignore.
  }
}

/** Manual check from Settings. */
export async function checkForAppUpdateInteractive(): Promise<void> {
  const update = await checkForAppUpdate();
  if (!update) {
    toast("Lados is up to date");
    return;
  }
  await promptAppUpdate(update);
}
