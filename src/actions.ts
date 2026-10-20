// Everything the user can do with an unlocked vault. Buttons, menus and keyboard shortcuts
// all call these functions.

import { redraw, refreshSelection, reloadVault, showStatus } from "./app";
import { credentialsDialog } from "./dialogs/credentials";
import { safeFileName } from "./dialogs/create-vault";
import { folderSelect, openEntryEditor } from "./dialogs/entry-editor";
import { openFolderEditor } from "./dialogs/folder-editor";
import { openGenerator } from "./dialogs/generator";
import { openHistory } from "./dialogs/history";
import { showImportSummary, showMergeReport } from "./dialogs/reports";
import { openSettings } from "./dialogs/settings";
import { backend, isCredentialError } from "./services/backend";
import {
  currentFolderId,
  entryCounts,
  folderById,
  folderPath,
  folderSubtree,
  preferences,
  store,
  visibleEntries,
} from "./state/store";
import type { EntryField, EntrySummary, MergeReport, Uuid } from "./types";
import { field } from "./ui/controls";
import { h } from "./ui/dom";
import { svgIcon } from "./ui/icons";
import { confirmDialog, promptDialog, showDialog } from "./ui/modal";
import { toast, toastError } from "./ui/toast";
import { plural } from "./utils/format";

/** Runs an action and reports any failure as a notification. */
export async function attempt(action: () => Promise<unknown>): Promise<void> {
  try {
    await action();
  } catch (error) {
    toastError(error);
  }
}

function entryById(id: Uuid): EntrySummary | undefined {
  return store.vault?.entries.find((entry) => entry.id === id);
}

function folderLabel(folderId: Uuid | null): string {
  return folderId ? folderPath(folderId).join(" › ") : "the top level";
}

export function selectEntry(id: Uuid | null): void {
  if (store.selectedEntryId === id) return;
  store.selectedEntryId = id;
  refreshSelection();
}

// ----- Session -----

export const lockVault = () => attempt(async () => showStatus(await backend.lockVault()));
export const closeVault = () => attempt(async () => showStatus(await backend.closeVault()));
export const quit = () => attempt(() => backend.quit());

export const renameVault = () =>
  attempt(async () => {
    const name = await promptDialog({ title: "Rename the vault", label: "Vault name", value: store.vault?.name, submitLabel: "Rename" });
    if (name === null) return;
    await backend.renameVault(name);
    await reloadVault();
  });

export const changeMasterKey = () =>
  attempt(async () => {
    const status = await credentialsDialog({
      title: "Change the master key",
      message: h(
        "div",
        { class: "notice warning" },
        svgIcon("warning"),
        h("span", {
          text: "The vault will be encrypted again with the new password and key file. Other copies of this vault keep the old key.",
        }),
      ),
      submitLabel: "Change master key",
      busyLabel: "Encrypting…",
      newSecret: true,
      allowCreateKeyFile: true,
      initialKeyFile: store.status?.keyFile ?? null,
      action: ({ password, keyFile }) => backend.changeMasterKey(password, keyFile),
    });
    if (!status) return;
    await showStatus(status);
    toast("The master key was changed", "success");
  });

// ----- Entries -----

export const newEntry = () =>
  attempt(async () => {
    const id = await openEntryEditor({ folderId: currentFolderId() });
    if (!id) return;
    store.selectedEntryId = id;
    await reloadVault();
  });

export const editEntry = (id = store.selectedEntryId) =>
  attempt(async () => {
    if (!id) return;
    const saved = await openEntryEditor({ entryId: id });
    if (saved) await reloadVault();
  });

export const duplicateEntry = (id = store.selectedEntryId) =>
  attempt(async () => {
    if (!id) return;
    store.selectedEntryId = await backend.duplicateEntry(id);
    await reloadVault();
    toast("Entry duplicated", "success");
  });

export const deleteEntry = (id = store.selectedEntryId) =>
  attempt(async () => {
    const entry = id ? entryById(id) : undefined;
    if (!entry) return;
    const confirmed = await confirmDialog({
      title: "Delete this entry?",
      message: h("p", {}, "“", h("strong", { text: entry.title }), "” will be deleted, together with its previous versions."),
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return;
    const list = visibleEntries();
    const index = list.findIndex((candidate) => candidate.id === entry.id);
    await backend.deleteEntry(entry.id);
    if (store.selectedEntryId === entry.id) {
      store.selectedEntryId = (list[index + 1] ?? list[index - 1])?.id ?? null;
    }
    await reloadVault();
    toast("Entry deleted");
  });

export const moveEntry = (id: Uuid, folderId: Uuid | null) =>
  attempt(async () => {
    const entry = entryById(id);
    if (!entry || entry.folderId === folderId) return;
    await backend.moveEntry(id, folderId);
    await reloadVault();
    toast(`Moved to ${folderLabel(folderId)}`);
  });

export const chooseFolderForEntry = (id = store.selectedEntryId) =>
  attempt(async () => {
    const entry = id ? entryById(id) : undefined;
    if (!entry) return;
    const select = folderSelect(entry.folderId);
    select.setAttribute("data-autofocus", "");
    const choice = await showDialog<{ folderId: Uuid | null }>({
      title: "Move entry",
      size: "small",
      body: field(`Move “${entry.title}” to`, select),
      submitLabel: "Move",
      onSubmit: () => ({ folderId: select.value || null }),
    });
    if (choice) await moveEntry(entry.id, choice.folderId);
  });

const FIELD_LABELS: Record<EntryField["kind"], string> = {
  username: "Username",
  password: "Password",
  url: "Web address",
  notes: "Notes",
  custom: "Value",
};

export const copyField = (id: Uuid, field: EntryField, secret = field.kind === "password", label = FIELD_LABELS[field.kind]) =>
  attempt(async () => {
    await backend.copyField(id, field);
    const seconds = preferences().clipboardClearSeconds;
    toast(secret && seconds > 0 ? `${label} copied. The clipboard will be cleared in ${seconds} s` : `${label} copied`, "success");
  });

export const copyPassword = (id = store.selectedEntryId) => {
  const entry = id ? entryById(id) : undefined;
  if (!entry) return;
  if (!entry.hasPassword) {
    toast("This entry has no password");
    return;
  }
  void copyField(entry.id, { kind: "password" });
};

export const copyUsername = (id = store.selectedEntryId) => {
  const entry = id ? entryById(id) : undefined;
  if (!entry) return;
  if (!entry.username) {
    toast("This entry has no username");
    return;
  }
  void copyField(entry.id, { kind: "username" });
};

export const openUrl = (id = store.selectedEntryId) =>
  attempt(async () => {
    const entry = id ? entryById(id) : undefined;
    if (!entry) return;
    if (!entry.url) {
      toast("This entry has no web address");
      return;
    }
    await backend.openEntryUrl(entry.id);
  });

export const showHistory = (id = store.selectedEntryId) =>
  attempt(async () => {
    if (!id) return;
    if (await openHistory(id)) {
      await reloadVault();
      toast("Version restored", "success");
    }
  });

// ----- Folders -----

export const newFolder = (parentId: Uuid | null = null) =>
  attempt(async () => {
    const id = await openFolderEditor({ parentId });
    if (!id) return;
    if (parentId) store.expandedFolders.add(parentId);
    store.view = { kind: "folder", id };
    store.query = "";
    store.searchResults = null;
    await reloadVault();
  });

export const editFolder = (id: Uuid) =>
  attempt(async () => {
    if (await openFolderEditor({ folderId: id })) await reloadVault();
  });

export const moveFolder = (id: Uuid, parentId: Uuid | null) =>
  attempt(async () => {
    const folder = folderById(id);
    if (!folder || folder.parentId === parentId) return;
    if (parentId && folderSubtree(id).has(parentId)) {
      toast("A folder cannot be moved inside itself", "error");
      return;
    }
    await backend.moveFolder(id, parentId);
    if (parentId) store.expandedFolders.add(parentId);
    await reloadVault();
    toast(`Moved to ${folderLabel(parentId)}`);
  });

export const deleteFolder = (id: Uuid) =>
  attempt(async () => {
    const folder = folderById(id);
    if (!folder) return;
    const entries = entryCounts().get(id) ?? 0;
    const subfolders = folderSubtree(id).size - 1;
    const contents = [
      subfolders > 0 && plural(subfolders, "subfolder"),
      entries > 0 && plural(entries, "entry", "entries"),
    ].filter(Boolean);
    const confirmed = await confirmDialog({
      title: "Delete this folder?",
      message: h(
        "p",
        {},
        "“",
        h("strong", { text: folder.name }),
        "” will be deleted",
        contents.length > 0 ? `, together with its ${contents.join(" and ")}.` : ".",
      ),
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return;
    await backend.deleteFolder(id);
    await reloadVault();
    toast("Folder deleted");
  });

export function selectView(view: typeof store.view): void {
  store.view = view;
  store.query = "";
  store.searchResults = null;
  const entries = visibleEntries();
  if (!entries.some((entry) => entry.id === store.selectedEntryId)) {
    store.selectedEntryId = entries[0]?.id ?? null;
  }
  redraw();
}

// ----- Tools -----

export const showGenerator = () => attempt(() => openGenerator());
export const showSettings = () => attempt(() => openSettings());

// ----- Import, export and merge -----

export const importCsv = () =>
  attempt(async () => {
    const path = await backend.pickOpenPath("csv", "Import a CSV file");
    if (!path) return;
    const summary = await backend.importCsv(path);
    await reloadVault();
    showImportSummary(summary);
  });

export const importKdbx = () =>
  attempt(async () => {
    const path = await backend.pickOpenPath("kdbx", "Import a KeePass database");
    if (!path) return;
    const summary = await credentialsDialog({
      title: "Open the KeePass database",
      message: "Enter the master key of the KeePass database. Its entries will be copied into this vault.",
      submitLabel: "Import",
      busyLabel: "Importing…",
      passwordOptional: true,
      action: ({ password, keyFile }) => backend.importKdbx(path, password, keyFile),
    });
    if (!summary) return;
    await reloadVault();
    showImportSummary(summary);
  });

export const exportCsv = () =>
  attempt(async () => {
    const confirmed = await confirmDialog({
      title: "Export without encryption?",
      message: h(
        "div",
        { class: "stack" },
        h("p", { text: "A CSV file stores every password as plain text. Anyone who can open the file can read them." }),
        h("p", { text: "Use it only to move your passwords to another program, and delete the file afterwards." }),
      ),
      confirmLabel: "Export anyway",
      danger: true,
    });
    if (!confirmed) return;
    const path = await backend.pickSavePath("csv", "Export to a CSV file", `${safeFileName(store.vault?.name ?? "", "Vault")}.csv`);
    if (!path) return;
    await backend.exportCsv(path);
    toast("Vault exported to CSV", "success");
  });

export const exportKdbx = () =>
  attempt(async () => {
    const path = await backend.pickSavePath("kdbx", "Export to a KeePass database", `${safeFileName(store.vault?.name ?? "", "Vault")}.kdbx`);
    if (!path) return;
    const done = await credentialsDialog({
      title: "Protect the KeePass database",
      message: "Choose the master key of the new KeePass file. It can be the same as this vault's or a different one.",
      submitLabel: "Export",
      busyLabel: "Exporting…",
      newSecret: true,
      allowCreateKeyFile: true,
      action: async ({ password, keyFile }) => {
        await backend.exportKdbx(path, password, keyFile);
        return true;
      },
    });
    if (done) toast("Vault exported to KeePass format", "success");
  });

/** Merges another Lados vault into this one. Its own key is tried first, as copies usually share it. */
export const mergeVault = () =>
  attempt(async () => {
    const path = await backend.pickOpenPath("vault", "Choose the vault to merge into this one");
    if (!path) return;
    let report: MergeReport | null;
    try {
      report = await backend.mergeVaultFile(path, null, null);
    } catch (error) {
      if (!isCredentialError(error)) throw error;
      report = await credentialsDialog({
        title: "Unlock the other vault",
        message: "That vault uses a different master key. Enter it to merge both vaults.",
        submitLabel: "Merge",
        busyLabel: "Merging…",
        action: ({ password, keyFile }) => backend.mergeVaultFile(path, password, keyFile),
      });
    }
    if (!report) return;
    await reloadVault();
    showMergeReport(report);
  });

export const showShortcuts = () =>
  attempt(() => {
    const shortcuts: [string, string][] = [
      ["Ctrl+N", "New entry"],
      ["Ctrl+E / Enter", "Edit the selected entry"],
      ["Ctrl+C", "Copy the password"],
      ["Ctrl+B", "Copy the username"],
      ["Ctrl+U", "Open the web address"],
      ["Delete", "Delete the selected entry"],
      ["Ctrl+F", "Search"],
      ["Ctrl+G", "Password generator"],
      ["Ctrl+L", "Lock the vault"],
      ["↑ ↓", "Move through the entries"],
      ["Esc", "Clear the search or close a dialog"],
    ];
    return showDialog<true>({
      title: "Keyboard shortcuts",
      size: "small",
      body: h("dl", { class: "shortcuts" }, shortcuts.map(([keys, label]) => [h("dt", {}, h("kbd", { text: keys })), h("dd", { text: label })])),
      hideCancel: true,
      submitLabel: "Close",
      onSubmit: () => true,
    });
  });
