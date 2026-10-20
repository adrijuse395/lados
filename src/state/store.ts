// What the interface is showing. The vault itself lives in Rust; this is only its summary.

import type { AppStatus, EntrySummary, FolderView, Preferences, Uuid, VaultView } from "../types";
import { expiryState } from "../utils/format";

export type SidebarView = { kind: "all" } | { kind: "expiring" } | { kind: "folder"; id: Uuid };
export type SortOrder = "title" | "modified";

interface Store {
  status: AppStatus | null;
  vault: VaultView | null;
  view: SidebarView;
  selectedEntryId: Uuid | null;
  query: string;
  /** Entries matching `query`, or `null` when not searching. */
  searchResults: Set<Uuid> | null;
  expandedFolders: Set<Uuid>;
  sort: SortOrder;
}

export const store: Store = {
  status: null,
  vault: null,
  view: { kind: "all" },
  selectedEntryId: null,
  query: "",
  searchResults: null,
  expandedFolders: new Set(),
  sort: "title",
};

interface FolderIndex {
  byId: Map<Uuid, FolderView>;
  children: Map<Uuid | null, FolderView[]>;
}

let folders: FolderIndex = { byId: new Map(), children: new Map() };

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

export function compareText(a: string, b: string): number {
  return collator.compare(a, b);
}

export function preferences(): Preferences {
  if (!store.status) throw new Error("The application status is not loaded");
  return store.status.preferences;
}

/** Replaces the vault summary and drops any selection that no longer exists. */
export function setVault(vault: VaultView): void {
  store.vault = vault;
  const byId = new Map(vault.folders.map((folder) => [folder.id, folder]));
  const children = new Map<Uuid | null, FolderView[]>();
  for (const folder of vault.folders) {
    const parent = folder.parentId && byId.has(folder.parentId) ? folder.parentId : null;
    const siblings = children.get(parent) ?? [];
    siblings.push(folder);
    children.set(parent, siblings);
  }
  for (const siblings of children.values()) {
    siblings.sort((a, b) => compareText(a.name, b.name));
  }
  folders = { byId, children };

  if (store.selectedEntryId && !vault.entries.some((entry) => entry.id === store.selectedEntryId)) {
    store.selectedEntryId = null;
  }
  if (store.view.kind === "folder" && !byId.has(store.view.id)) {
    store.view = { kind: "all" };
  }
}

export function resetVaultState(): void {
  store.vault = null;
  store.view = { kind: "all" };
  store.selectedEntryId = null;
  store.query = "";
  store.searchResults = null;
  folders = { byId: new Map(), children: new Map() };
}

export function folderById(id: Uuid | null): FolderView | undefined {
  return id ? folders.byId.get(id) : undefined;
}

export function childFolders(parentId: Uuid | null): FolderView[] {
  return folders.children.get(parentId) ?? [];
}

/** The folder and everything below it. */
export function folderSubtree(id: Uuid): Set<Uuid> {
  const result = new Set<Uuid>([id]);
  const pending = [id];
  while (pending.length > 0) {
    for (const child of childFolders(pending.pop() ?? null)) {
      if (!result.has(child.id)) {
        result.add(child.id);
        pending.push(child.id);
      }
    }
  }
  return result;
}

export function folderPath(id: Uuid | null): string[] {
  const names: string[] = [];
  const seen = new Set<Uuid>();
  let folder = folderById(id);
  while (folder && !seen.has(folder.id)) {
    seen.add(folder.id);
    names.unshift(folder.name);
    folder = folderById(folder.parentId);
  }
  return names;
}

/** Folders in tree order, with their depth, for drop-down lists. */
export function flattenedFolders(): { folder: FolderView; depth: number }[] {
  const result: { folder: FolderView; depth: number }[] = [];
  const visit = (parentId: Uuid | null, depth: number) => {
    for (const folder of childFolders(parentId)) {
      result.push({ folder, depth });
      visit(folder.id, depth + 1);
    }
  };
  visit(null, 0);
  return result;
}

/** Number of entries in each folder, subfolders included. */
export function entryCounts(): Map<Uuid, number> {
  const counts = new Map<Uuid, number>();
  for (const entry of store.vault?.entries ?? []) {
    const seen = new Set<Uuid>();
    let folder = folderById(entry.folderId);
    while (folder && !seen.has(folder.id)) {
      seen.add(folder.id);
      counts.set(folder.id, (counts.get(folder.id) ?? 0) + 1);
      folder = folderById(folder.parentId);
    }
  }
  return counts;
}

export function isExpiring(entry: EntrySummary): boolean {
  const state = expiryState(entry.expirationDate);
  return state === "expired" || state === "soon";
}

export function selectedEntry(): EntrySummary | undefined {
  return store.vault?.entries.find((entry) => entry.id === store.selectedEntryId);
}

/** The entries listed in the middle pane, in display order. */
export function visibleEntries(): EntrySummary[] {
  const vault = store.vault;
  if (!vault) return [];

  if (store.searchResults) {
    const results = store.searchResults;
    return sortEntries(vault.entries.filter((entry) => results.has(entry.id)));
  }
  switch (store.view.kind) {
    case "all":
      return sortEntries(vault.entries.slice());
    case "expiring":
      return vault.entries
        .filter(isExpiring)
        .sort((a, b) => (a.expirationDate ?? "").localeCompare(b.expirationDate ?? ""));
    case "folder": {
      const inside = folderSubtree(store.view.id);
      return sortEntries(vault.entries.filter((entry) => entry.folderId !== null && inside.has(entry.folderId)));
    }
  }
}

function sortEntries(entries: EntrySummary[]): EntrySummary[] {
  if (store.sort === "modified") {
    return entries.sort((a, b) => Date.parse(b.modifiedAt) - Date.parse(a.modifiedAt));
  }
  return entries.sort((a, b) => compareText(a.title, b.title) || compareText(a.username, b.username));
}

export function viewTitle(): string {
  if (store.searchResults) return "Search results";
  switch (store.view.kind) {
    case "all":
      return "All entries";
    case "expiring":
      return "Expiring soon";
    case "folder":
      return folderById(store.view.id)?.name ?? "Folder";
  }
}

/** The folder that receives new entries: the one being viewed, if any. */
export function currentFolderId(): Uuid | null {
  return store.view.kind === "folder" && !store.searchResults ? store.view.id : null;
}
