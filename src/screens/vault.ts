// The main window of an unlocked vault: top bar, folders, entry list and details.

import * as actions from "../actions";
import { backend } from "../services/backend";
import { store, visibleEntries } from "../state/store";
import type { Uuid } from "../types";
import { iconButton } from "../ui/controls";
import { h, isTyping, mount } from "../ui/dom";
import { dragonMark, svgIcon } from "../ui/icons";
import { openMenu } from "../ui/menu";
import { isModalOpen } from "../ui/modal";
import { toastError } from "../ui/toast";
import { renderEntryDetails } from "./entry-details";
import { focusEntryList, highlightSelection, renderEntryList } from "./entry-list";
import { renderSidebar } from "./sidebar";

interface VaultElements {
  screen: HTMLElement;
  name: HTMLElement;
  search: HTMLInputElement;
  sidebar: HTMLElement;
  list: HTMLElement;
  details: HTMLElement;
}

let elements: VaultElements | null = null;
let searchTimer = 0;
let searchGeneration = 0;

export function isVaultMounted(): boolean {
  return elements?.screen.isConnected ?? false;
}

function openMoreMenu(anchor: HTMLElement): void {
  openMenu(anchor, [
    { label: "Import from CSV…", icon: "import", onSelect: () => void actions.importCsv() },
    { label: "Import from KeePass…", icon: "import", onSelect: () => void actions.importKdbx() },
    { label: "Export to CSV…", icon: "export", onSelect: () => void actions.exportCsv() },
    { label: "Export to KeePass…", icon: "export", onSelect: () => void actions.exportKdbx() },
    "separator",
    { label: "Merge another vault…", icon: "merge", onSelect: () => void actions.mergeVault() },
    "separator",
    { label: "Rename vault…", icon: "edit", onSelect: () => void actions.renameVault() },
    { label: "Change master key…", icon: "key", onSelect: () => void actions.changeMasterKey() },
    { label: "Settings…", icon: "settings", onSelect: () => void actions.showSettings() },
    { label: "Keyboard shortcuts", icon: "info", onSelect: () => void actions.showShortcuts() },
    "separator",
    { label: "Close vault", icon: "signOut", onSelect: () => void actions.closeVault() },
    { label: "Quit Lados", icon: "power", onSelect: () => void actions.quit() },
  ]);
}

async function runSearch(): Promise<void> {
  const current = ++searchGeneration;
  const query = store.query.trim();
  let results: Set<Uuid> | null = null;
  if (query) {
    try {
      results = new Set(await backend.searchEntries(query));
    } catch (error) {
      toastError(error);
      return;
    }
  }
  if (current !== searchGeneration) return;
  store.searchResults = results;
  const entries = visibleEntries();
  if (!entries.some((entry) => entry.id === store.selectedEntryId)) {
    store.selectedEntryId = entries[0]?.id ?? null;
  }
  drawVault();
}

function setQuery(query: string, immediately = false): void {
  store.query = query;
  window.clearTimeout(searchTimer);
  if (immediately) {
    void runSearch();
  } else {
    searchTimer = window.setTimeout(() => void runSearch(), 150);
  }
}

export function renderVault(container: HTMLElement): void {
  const search = h("input", {
    class: "search-input",
    type: "search",
    placeholder: "Search",
    spellcheck: false,
    autocomplete: "off",
    "aria-label": "Search entries",
    value: store.query,
    onInput: () => setQuery(search.value),
    onKeyDown: (event: KeyboardEvent) => {
      if (event.key === "Escape" && search.value) {
        event.preventDefault();
        event.stopPropagation();
        search.value = "";
        setQuery("", true);
      } else if (event.key === "ArrowDown" || event.key === "Enter") {
        event.preventDefault();
        focusEntryList();
      }
    },
  });
  const name = h("span", { class: "vault-name truncate" });
  const sidebar = h("aside", { class: "sidebar", "aria-label": "Folders" });
  const list = h("section", { class: "entry-pane", "aria-label": "Entries" });
  const details = h("section", { class: "details-pane", "aria-label": "Entry details" });

  const screen = h(
    "div",
    { class: "vault-screen" },
    h(
      "header",
      { class: "topbar" },
      h("div", { class: "topbar-brand" }, dragonMark("logo small"), name),
      h("label", { class: "search-box" }, svgIcon("search"), search, h("kbd", { text: "Ctrl+F" })),
      h(
        "div",
        { class: "topbar-actions" },
        h("button", { type: "button", class: "button primary", title: "New entry (Ctrl+N)", onClick: () => void actions.newEntry() }, svgIcon("plus"), "New entry"),
        iconButton("dice", "Password generator (Ctrl+G)", () => void actions.showGenerator()),
        iconButton("lock", "Lock (Ctrl+L)", () => void actions.lockVault()),
        iconButton("more", "More actions", (event) => openMoreMenu(event.currentTarget as HTMLElement)),
      ),
    ),
    h("div", { class: "panes" }, sidebar, list, details),
  );

  mount(container, screen);
  elements = { screen, name, search, sidebar, list, details };
  drawVault();
}

/** Redraws every pane from the store. */
export function drawVault(): void {
  if (!elements || !isVaultMounted() || !store.vault) return;
  elements.name.textContent = store.vault.name;
  elements.name.title = store.vault.path;
  if (document.activeElement !== elements.search) elements.search.value = store.query;
  renderSidebar(elements.sidebar);
  renderEntryList(elements.list);
  void renderEntryDetails(elements.details);
}

export function updateSelection(): void {
  if (!elements || !isVaultMounted()) return;
  highlightSelection();
  void renderEntryDetails(elements.details);
}

function moveSelection(step: number | "first" | "last"): void {
  const entries = visibleEntries();
  if (entries.length === 0) return;
  const index = entries.findIndex((entry) => entry.id === store.selectedEntryId);
  let next: number;
  if (step === "first") next = 0;
  else if (step === "last") next = entries.length - 1;
  else next = index < 0 ? 0 : Math.min(entries.length - 1, Math.max(0, index + step));
  actions.selectEntry(entries[next].id);
}

function hasTextSelection(target: EventTarget | null): boolean {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    return target.selectionStart !== target.selectionEnd;
  }
  return (window.getSelection()?.toString() ?? "") !== "";
}

function onKeyDown(event: KeyboardEvent): void {
  if (!isVaultMounted() || isModalOpen() || event.defaultPrevented) return;
  const key = event.key.toLowerCase();
  const typing = isTyping(event.target);

  if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey) {
    const commands: Record<string, () => void> = {
      f: () => {
        elements?.search.focus();
        elements?.search.select();
      },
      n: () => void actions.newEntry(),
      l: () => void actions.lockVault(),
      g: () => void actions.showGenerator(),
      e: () => void actions.editEntry(),
      u: () => void actions.openUrl(),
      b: () => actions.copyUsername(),
      c: () => actions.copyPassword(),
    };
    const command = commands[key];
    if (!command || (key === "c" && hasTextSelection(event.target))) return;
    event.preventDefault();
    command();
    return;
  }
  if (typing || event.altKey || event.ctrlKey || event.metaKey || event.target instanceof HTMLButtonElement) return;

  const inList = event.target instanceof Element && event.target.closest(".entry-pane") !== null;
  const onPage = event.target === document.body || inList;
  switch (event.key) {
    case "ArrowDown":
      if (!onPage) return;
      moveSelection(1);
      break;
    case "ArrowUp":
      if (!onPage) return;
      moveSelection(-1);
      break;
    case "Home":
      if (!inList) return;
      moveSelection("first");
      break;
    case "End":
      if (!inList) return;
      moveSelection("last");
      break;
    case "Enter":
      if (!inList) return;
      void actions.editEntry();
      break;
    case "Delete":
      if (!onPage) return;
      void actions.deleteEntry();
      break;
    case "Escape":
      if (!store.query) return;
      if (elements) elements.search.value = "";
      setQuery("", true);
      break;
    default:
      return;
  }
  event.preventDefault();
}

/** Keyboard shortcuts of the vault screen. Installed once at start-up. */
export function installShortcuts(): void {
  document.addEventListener("keydown", onKeyDown);
}
