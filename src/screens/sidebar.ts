// Left pane: the virtual views and the folder tree, with drag and drop.

import * as actions from "../actions";
import { childFolders, entryCounts, folderSubtree, isExpiring, store, type SidebarView } from "../state/store";
import type { FolderView, Uuid } from "../types";
import { iconButton, itemIcon } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { svgIcon, type IconName } from "../ui/icons";
import { openMenu, type MenuEntry } from "../ui/menu";

export const ENTRY_DRAG_TYPE = "application/x-lados-entry";
export const FOLDER_DRAG_TYPE = "application/x-lados-folder";

let container: HTMLElement | null = null;
// The drag data cannot be read until the drop, so the folder being dragged is remembered here.
let draggedFolderId: Uuid | null = null;

function isActive(view: SidebarView): boolean {
  if (store.searchResults) return false;
  const current = store.view;
  return current.kind === view.kind && (current.kind !== "folder" || (view.kind === "folder" && current.id === view.id));
}

/** Entries and folders dropped here move into `folderId` (`null` means the top level). */
function acceptDrops(element: HTMLElement, folderId: Uuid | null): void {
  element.addEventListener("dragover", (event) => {
    const types = event.dataTransfer?.types ?? [];
    if (!types.includes(ENTRY_DRAG_TYPE) && !types.includes(FOLDER_DRAG_TYPE)) return;
    if (types.includes(FOLDER_DRAG_TYPE) && draggedFolderId && folderId && folderSubtree(draggedFolderId).has(folderId)) {
      return;
    }
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    element.classList.add("drop-target");
  });
  element.addEventListener("dragleave", (event) => {
    if (!element.contains(event.relatedTarget as Node | null)) element.classList.remove("drop-target");
  });
  element.addEventListener("drop", (event) => {
    event.preventDefault();
    element.classList.remove("drop-target");
    const entryId = event.dataTransfer?.getData(ENTRY_DRAG_TYPE);
    const draggedFolder = event.dataTransfer?.getData(FOLDER_DRAG_TYPE);
    if (entryId) {
      void actions.moveEntry(entryId, folderId);
    } else if (draggedFolder && draggedFolder !== folderId) {
      void actions.moveFolder(draggedFolder, folderId);
    }
  });
}

function navItem(label: string, icon: IconName, count: number, view: SidebarView, dropTitle?: string): HTMLElement {
  const active = isActive(view);
  const item = h(
    "button",
    {
      type: "button",
      class: ["nav-item", active && "active"],
      "aria-current": active ? "true" : undefined,
      title: dropTitle,
      onClick: () => actions.selectView(view),
    },
    svgIcon(icon),
    h("span", { class: "nav-label", text: label }),
    h("span", { class: "count", text: String(count) }),
  );
  if (dropTitle) acceptDrops(item, null);
  return item;
}

function toggleFolder(id: Uuid): void {
  if (store.expandedFolders.has(id)) {
    store.expandedFolders.delete(id);
  } else {
    store.expandedFolders.add(id);
  }
  if (container) renderSidebar(container);
}

function openFolderMenu(anchor: HTMLElement | MouseEvent, folder: FolderView): void {
  const entries: MenuEntry[] = [
    {
      label: "New entry here",
      icon: "plus",
      onSelect: () => {
        actions.selectView({ kind: "folder", id: folder.id });
        void actions.newEntry();
      },
    },
    { label: "New subfolder…", icon: "folderPlus", onSelect: () => void actions.newFolder(folder.id) },
    { label: "Edit folder…", icon: "edit", shortcut: "F2", onSelect: () => void actions.editFolder(folder.id) },
  ];
  if (folder.parentId) {
    entries.push({ label: "Move to the top level", icon: "move", onSelect: () => void actions.moveFolder(folder.id, null) });
  }
  entries.push("separator", {
    label: "Delete folder",
    icon: "trash",
    danger: true,
    onSelect: () => void actions.deleteFolder(folder.id),
  });
  openMenu(anchor, entries);
}

function folderNode(folder: FolderView, depth: number, counts: Map<Uuid, number>): HTMLElement {
  const children = childFolders(folder.id);
  const expanded = store.expandedFolders.has(folder.id);
  const active = isActive({ kind: "folder", id: folder.id });

  const toggle =
    children.length > 0
      ? h(
          "button",
          {
            type: "button",
            class: "tree-toggle",
            tabIndex: -1,
            "aria-label": expanded ? "Collapse" : "Expand",
            onClick: (event: MouseEvent) => {
              event.stopPropagation();
              toggleFolder(folder.id);
            },
          },
          svgIcon(expanded ? "chevronDown" : "chevronRight"),
        )
      : h("span", { class: "tree-toggle" });

  const row = h(
    "div",
    {
      class: ["tree-row", active && "active"],
      style: `--depth: ${depth}`,
      role: "treeitem",
      tabIndex: 0,
      draggable: true,
      dataset: { id: folder.id },
      "aria-selected": String(active),
      "aria-expanded": children.length > 0 ? String(expanded) : undefined,
      onClick: () => actions.selectView({ kind: "folder", id: folder.id }),
      onDblClick: () => {
        if (children.length > 0) toggleFolder(folder.id);
      },
      onKeyDown: (event: KeyboardEvent) => {
        let handled = true;
        if (event.key === "Enter" || event.key === " ") {
          actions.selectView({ kind: "folder", id: folder.id });
        } else if (event.key === "ArrowRight" && children.length > 0 && !expanded) {
          toggleFolder(folder.id);
        } else if (event.key === "ArrowLeft" && expanded) {
          toggleFolder(folder.id);
        } else if (event.key === "F2") {
          void actions.editFolder(folder.id);
        } else if (event.key === "Delete") {
          void actions.deleteFolder(folder.id);
        } else {
          handled = false;
        }
        if (handled) {
          event.preventDefault();
          event.stopPropagation();
        }
      },
      onContextMenu: (event: MouseEvent) => {
        event.preventDefault();
        openFolderMenu(event, folder);
      },
      onDragStart: (event: DragEvent) => {
        draggedFolderId = folder.id;
        event.dataTransfer?.setData(FOLDER_DRAG_TYPE, folder.id);
        if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
      },
      onDragEnd: () => {
        draggedFolderId = null;
      },
    },
    toggle,
    itemIcon(folder.icon, "folder"),
    h("span", { class: "tree-label", text: folder.name, title: folder.name }),
    h("span", { class: "count", text: String(counts.get(folder.id) ?? 0) }),
    iconButton(
      "more",
      "Folder actions",
      (event) => {
        event.stopPropagation();
        openFolderMenu(event.currentTarget as HTMLElement, folder);
      },
      "row-action",
    ),
  );
  acceptDrops(row, folder.id);

  return h(
    "li",
    { role: "none" },
    row,
    expanded && children.length > 0
      ? h("ul", { role: "group" }, children.map((child) => folderNode(child, depth + 1, counts)))
      : null,
  );
}

export function renderSidebar(target: HTMLElement): void {
  container = target;
  const vault = store.vault;
  if (!vault) return;
  const focused = document.activeElement;
  const focusedFolder = focused instanceof HTMLElement ? focused.closest<HTMLElement>(".tree-row")?.dataset.id : undefined;

  const counts = entryCounts();
  const expiring = vault.entries.filter(isExpiring).length;
  const topFolders = childFolders(null);

  const foldersHeader = h(
    "div",
    { class: "sidebar-heading", title: "Drop a folder here to move it to the top level" },
    h("span", { text: "Folders" }),
    iconButton("folderPlus", "New folder", () => void actions.newFolder(null), "small"),
  );
  acceptDrops(foldersHeader, null);

  mount(
    target,
    h(
      "nav",
      { class: "sidebar-nav", "aria-label": "Views" },
      navItem("All entries", "list", vault.entries.length, { kind: "all" }, "Drop an entry here to take it out of its folder"),
      expiring > 0 || store.view.kind === "expiring" ? navItem("Expiring soon", "clock", expiring, { kind: "expiring" }) : null,
    ),
    foldersHeader,
    topFolders.length > 0
      ? h("ul", { class: "tree", role: "tree", "aria-label": "Folders" }, topFolders.map((folder) => folderNode(folder, 0, counts)))
      : h("p", { class: "sidebar-empty", text: "Organize your entries in folders. Create one with the + button." }),
  );

  if (focusedFolder) target.querySelector<HTMLElement>(`.tree-row[data-id="${CSS.escape(focusedFolder)}"]`)?.focus();
}
