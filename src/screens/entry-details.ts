// Right pane: the selected entry. Secrets stay masked until the user asks for them,
// and are fetched from the core one at a time.

import * as actions from "../actions";
import { BackendError, backend } from "../services/backend";
import { store } from "../state/store";
import type { EntryDetails, EntryField, Uuid } from "../types";
import { iconButton, itemIcon, passwordGlyphs } from "../ui/controls";
import { h, mount, type Child } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { toastError } from "../ui/toast";
import { describeExpiry, expiryState, formatDateTime, plural } from "../utils/format";
import { openEntryMenu } from "./entry-menu";

const MASK = "••••••••••••";

let generation = 0;

function fieldRow(label: string, value: Child, buttons: Child[] = []): HTMLElement {
  return h(
    "div",
    { class: "field-row" },
    h("dt", { text: label }),
    h("dd", {}, h("div", { class: "field-value" }, value), h("div", { class: "field-actions" }, buttons)),
  );
}

function plainRow(label: string, value: string, entryId: Uuid, field: EntryField): HTMLElement {
  return fieldRow(label, h("span", { class: "selectable", text: value }), [
    iconButton("copy", `Copy ${label.toLowerCase()}`, () => void actions.copyField(entryId, field, false, label)),
  ]);
}

function secretRow(label: string, entryId: Uuid, field: EntryField): HTMLElement {
  let revealed = false;
  const value = h("span", { class: "secret-value mono masked", text: MASK });
  const toggle = iconButton("eye", "Show", async () => {
    if (revealed) {
      revealed = false;
      value.replaceChildren(MASK);
      value.classList.add("masked");
    } else {
      try {
        const secret = await backend.revealField(entryId, field);
        revealed = true;
        value.replaceChildren(...passwordGlyphs(secret));
        value.classList.remove("masked");
      } catch (error) {
        toastError(error);
        return;
      }
    }
    toggle.replaceChildren(svgIcon(revealed ? "eyeOff" : "eye"));
    toggle.title = revealed ? "Hide" : "Show";
    toggle.setAttribute("aria-label", toggle.title);
  });
  return fieldRow(label, value, [
    toggle,
    iconButton("copy", `Copy ${label.toLowerCase()}`, () => void actions.copyField(entryId, field, true, label)),
  ]);
}

function detailsView(details: EntryDetails): HTMLElement {
  const rows: HTMLElement[] = [];
  if (details.username) rows.push(plainRow("Username", details.username, details.id, { kind: "username" }));
  if (details.hasPassword) rows.push(secretRow("Password", details.id, { kind: "password" }));
  if (details.url) {
    rows.push(
      fieldRow(
        "Website",
        h("button", { type: "button", class: "link-button url", title: "Open in the browser", onClick: () => void actions.openUrl(details.id) }, details.url),
        [
          iconButton("external", "Open in the browser", () => void actions.openUrl(details.id)),
          iconButton("copy", "Copy web address", () => void actions.copyField(details.id, { kind: "url" }, false, "Web address")),
        ],
      ),
    );
  }
  if (details.expirationDate) {
    const state = expiryState(details.expirationDate);
    rows.push(
      fieldRow(
        "Expires",
        h("span", { class: ["expiry", state && `expiry-${state}`] }, svgIcon("calendar"), describeExpiry(details.expirationDate)),
      ),
    );
  }
  details.customFields.forEach((custom, index) => {
    const field: EntryField = { kind: "custom", index };
    rows.push(custom.protected ? secretRow(custom.name, details.id, field) : plainRow(custom.name, custom.value ?? "", details.id, field));
  });

  return h(
    "article",
    { class: "details" },
    h(
      "header",
      { class: "details-header" },
      itemIcon(details.icon, "key", "details-icon"),
      h(
        "div",
        { class: "details-heading" },
        h("h2", { class: "selectable", text: details.title }),
        h("div", { class: "breadcrumb" }, svgIcon("folder"), details.folderPath.length > 0 ? details.folderPath.join(" › ") : "No folder"),
      ),
      h(
        "div",
        { class: "details-actions" },
        h("button", { type: "button", class: "button", title: "Edit (Ctrl+E)", onClick: () => void actions.editEntry(details.id) }, svgIcon("edit"), "Edit"),
        iconButton("more", "More actions", (event) => openEntryMenu(event.currentTarget as HTMLElement, details.id)),
      ),
    ),
    rows.length > 0 ? h("dl", { class: "field-list" }, rows) : h("p", { class: "muted", text: "This entry only has a title." }),
    details.notes
      ? h(
          "section",
          { class: "notes" },
          h(
            "div",
            { class: "section-header" },
            h("h3", { text: "Notes" }),
            iconButton("copy", "Copy notes", () => void actions.copyField(details.id, { kind: "notes" }, false, "Notes")),
          ),
          h("div", { class: "notes-text selectable", text: details.notes }),
        )
      : null,
    h(
      "footer",
      { class: "details-meta" },
      h("span", { text: `Created ${formatDateTime(details.createdAt)}` }),
      h("span", { text: `Modified ${formatDateTime(details.modifiedAt)}` }),
      details.historyCount > 0
        ? h(
            "button",
            { type: "button", class: "link-button", onClick: () => void actions.showHistory(details.id) },
            svgIcon("history"),
            plural(details.historyCount, "previous version"),
          )
        : null,
    ),
  );
}

function placeholder(): HTMLElement {
  const empty = (store.vault?.entries.length ?? 0) === 0;
  return h(
    "div",
    { class: "details-placeholder" },
    svgIcon("shield", "icon placeholder-icon"),
    h("p", { text: empty ? "The details of your entries will appear here." : "Select an entry to see its details." }),
  );
}

export async function renderEntryDetails(container: HTMLElement): Promise<void> {
  const current = ++generation;
  const id = store.selectedEntryId;
  if (!id) {
    mount(container, placeholder());
    return;
  }
  try {
    const details = await backend.getEntry(id);
    if (current === generation) mount(container, detailsView(details));
  } catch (error) {
    if (current !== generation) return;
    mount(container, placeholder());
    if (!(error instanceof BackendError && (error.kind === "notFound" || error.kind === "locked"))) toastError(error);
  }
}
