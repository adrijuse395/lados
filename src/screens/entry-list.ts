// Middle pane: the entries of the current view or search.

import * as actions from "../actions";
import { folderPath, store, viewTitle, visibleEntries, type SortOrder } from "../state/store";
import type { EntrySummary } from "../types";
import { itemIcon, selectInput } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { openMenu } from "../ui/menu";
import { EXPIRY_WARNING_DAYS, expiryState } from "../utils/format";
import { openEntryMenu } from "./entry-menu";
import { ENTRY_DRAG_TYPE } from "./sidebar";

let container: HTMLElement | null = null;

function hostOf(url: string): string {
  return url.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").split(/[/?#]/)[0].replace(/^www\./i, "");
}

/** Where the entry lives, relative to the folder being viewed. */
function locationOf(entry: EntrySummary): string {
  if (!entry.folderId) return "";
  const path = folderPath(entry.folderId);
  if (!store.searchResults && store.view.kind === "folder") {
    return path.slice(folderPath(store.view.id).length).join(" › ");
  }
  return path.join(" › ");
}

function entryRow(entry: EntrySummary): HTMLElement {
  const selected = entry.id === store.selectedEntryId;
  const expiry = expiryState(entry.expirationDate);
  const location = locationOf(entry);
  const subtitle = entry.username || (entry.url ? hostOf(entry.url) : "");
  return h(
    "li",
    {
      id: `entry-${entry.id}`,
      class: ["entry-row", selected && "selected"],
      role: "option",
      "aria-selected": String(selected),
      draggable: true,
      dataset: { id: entry.id },
      onClick: () => actions.selectEntry(entry.id),
      onDblClick: () => void actions.editEntry(entry.id),
      onContextMenu: (event: MouseEvent) => {
        event.preventDefault();
        actions.selectEntry(entry.id);
        openEntryMenu(event, entry.id);
      },
      onDragStart: (event: DragEvent) => {
        event.dataTransfer?.setData(ENTRY_DRAG_TYPE, entry.id);
        if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
      },
    },
    itemIcon(entry.icon, "key"),
    h(
      "div",
      { class: "entry-text" },
      h(
        "div",
        { class: "entry-title" },
        h("span", { class: "truncate", text: entry.title }),
        expiry === "expired" ? h("span", { class: "badge danger", text: "Expired" }) : null,
        expiry === "soon" ? h("span", { class: "badge warning", text: "Expires soon" }) : null,
      ),
      h(
        "div",
        { class: "entry-subtitle" },
        h("span", { class: "truncate", text: subtitle }),
        location ? h("span", { class: "entry-location truncate", text: location }) : null,
      ),
    ),
  );
}

function emptyState(): HTMLElement {
  const vault = store.vault;
  if (store.searchResults) {
    return h(
      "div",
      { class: "empty-state" },
      svgIcon("search", "icon empty-icon"),
      h("p", { class: "empty-title", text: "Nothing found" }),
      h("p", { text: `No entry matches “${store.query.trim()}”. Protected fields are never searched.` }),
    );
  }
  if (!vault || vault.entries.length === 0) {
    return h(
      "div",
      { class: "empty-state" },
      svgIcon("key", "icon empty-icon"),
      h("p", { class: "empty-title", text: "Your vault is empty" }),
      h("p", { text: "Create your first entry, or bring your passwords from another program." }),
      h(
        "div",
        { class: "empty-actions" },
        h("button", { type: "button", class: "button primary", onClick: () => void actions.newEntry() }, svgIcon("plus"), "New entry"),
        h(
          "button",
          {
            type: "button",
            class: "button",
            onClick: (event: MouseEvent) =>
              openMenu(event.currentTarget as HTMLElement, [
                { label: "From a CSV file…", icon: "import", onSelect: () => void actions.importCsv() },
                { label: "From KeePass (.kdbx)…", icon: "import", onSelect: () => void actions.importKdbx() },
              ]),
          },
          svgIcon("import"),
          "Import…",
        ),
      ),
    );
  }
  if (store.view.kind === "expiring") {
    return h(
      "div",
      { class: "empty-state" },
      svgIcon("clock", "icon empty-icon"),
      h("p", { class: "empty-title", text: "Nothing expires soon" }),
      h("p", { text: `No entry expires in the next ${EXPIRY_WARNING_DAYS} days.` }),
    );
  }
  return h(
    "div",
    { class: "empty-state" },
    svgIcon("folder", "icon empty-icon"),
    h("p", { class: "empty-title", text: "This folder is empty" }),
    h("button", { type: "button", class: "button", onClick: () => void actions.newEntry() }, svgIcon("plus"), "New entry here"),
  );
}

export function renderEntryList(target: HTMLElement): void {
  container = target;
  const entries = visibleEntries();
  const title = viewTitle();

  const sort = selectInput<SortOrder>(
    [
      { value: "title", label: "By title" },
      { value: "modified", label: "Recently modified" },
    ],
    store.sort,
  );
  sort.classList.add("compact");
  sort.setAttribute("aria-label", "Sort the entries");
  sort.addEventListener("change", () => {
    store.sort = sort.value as SortOrder;
    renderEntryList(target);
  });
  const sortable = store.searchResults !== null || store.view.kind !== "expiring";

  mount(
    target,
    h(
      "div",
      { class: "pane-header" },
      h("div", { class: "pane-title" }, h("h2", { class: "truncate", text: title }), h("span", { class: "count", text: String(entries.length) })),
      sortable && entries.length > 1 ? sort : null,
    ),
    entries.length > 0
      ? h(
          "ul",
          {
            class: "entries",
            role: "listbox",
            tabIndex: 0,
            "aria-label": title,
            "aria-activedescendant": store.selectedEntryId ? `entry-${store.selectedEntryId}` : undefined,
          },
          entries.map(entryRow),
        )
      : emptyState(),
  );
  scrollToSelection();
}

/** Moves the highlight to the selected entry without rebuilding the list. */
export function highlightSelection(): void {
  if (!container) return;
  for (const row of container.querySelectorAll<HTMLElement>(".entry-row")) {
    const selected = row.dataset.id === store.selectedEntryId;
    row.classList.toggle("selected", selected);
    row.setAttribute("aria-selected", String(selected));
  }
  const list = container.querySelector(".entries");
  if (list && store.selectedEntryId) list.setAttribute("aria-activedescendant", `entry-${store.selectedEntryId}`);
  scrollToSelection();
}

function scrollToSelection(): void {
  container?.querySelector(".entry-row.selected")?.scrollIntoView({ block: "nearest" });
}

export function focusEntryList(): void {
  container?.querySelector<HTMLElement>(".entries")?.focus();
}
