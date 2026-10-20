import * as actions from "../actions";
import { store } from "../state/store";
import type { Uuid } from "../types";
import { openMenu } from "../ui/menu";

/** The actions available on one entry, shared by the list's context menu and the details pane. */
export function openEntryMenu(anchor: HTMLElement | MouseEvent, id: Uuid): void {
  const entry = store.vault?.entries.find((candidate) => candidate.id === id);
  if (!entry) return;
  openMenu(anchor, [
    { label: "Copy username", icon: "user", shortcut: "Ctrl+B", disabled: !entry.username, onSelect: () => actions.copyUsername(id) },
    { label: "Copy password", icon: "key", shortcut: "Ctrl+C", disabled: !entry.hasPassword, onSelect: () => actions.copyPassword(id) },
    { label: "Open web address", icon: "external", shortcut: "Ctrl+U", disabled: !entry.url, onSelect: () => void actions.openUrl(id) },
    "separator",
    { label: "Edit…", icon: "edit", shortcut: "Ctrl+E", onSelect: () => void actions.editEntry(id) },
    { label: "Duplicate", icon: "duplicate", onSelect: () => void actions.duplicateEntry(id) },
    { label: "Move to folder…", icon: "move", onSelect: () => void actions.chooseFolderForEntry(id) },
    { label: "Previous versions…", icon: "history", onSelect: () => void actions.showHistory(id) },
    "separator",
    { label: "Delete", icon: "trash", shortcut: "Del", danger: true, onSelect: () => void actions.deleteEntry(id) },
  ]);
}
